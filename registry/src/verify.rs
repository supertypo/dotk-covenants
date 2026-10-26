use std::collections::HashSet;

use anyhow::{Result, anyhow, bail, ensure};
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::hashing::covenant_id::covenant_id;
use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint, TransactionOutput};
use kaspa_rpc_core::RpcHash;

use crate::api::{KIND_ACTIVE, KIND_OWNER_UNKNOWN, KIND_PENDING, Manifest, Snapshot};
use crate::compile::{self, Template, Templates};
use crate::hex;
use crate::state::{self, KEY_MAX, KEY_MIN};

/// Pinned, so an API cannot point the proof at another covenant.
pub const REGISTRY_COVENANT_ID: &str = "ee2128c03dfac7f6d74734bb3c879bd999434c47a55945b8a6daae2a1e4a21de";

fn published(abi: &silverscript_abi::SilAbiArtifact, what: &str) -> Result<Template> {
    let mut contracts = abi.contracts.values();
    let (Some(contract), None) = (contracts.next(), contracts.next()) else {
        bail!("the {what} artifact must hold exactly one contract");
    };
    let c = &contract.compiled;
    ensure!(
        c.state_span.offset.checked_add(c.state_span.len).is_some_and(|end| end <= c.bytecode.len()),
        "the {what} artifact's state region lies outside its bytecode"
    );
    Ok(Template { bytecode: c.bytecode.clone(), state_start: c.state_span.offset, state_len: c.state_span.len })
}

/// Step 1: the contracts compile to the published bytecode.
pub fn check_compile(manifest: &Manifest) -> Result<(Templates, String)> {
    ensure!(manifest.network == "mainnet", "the API serves a {} deployment, not mainnet", manifest.network);
    let t = compile::compile(&manifest.params)?;
    for (what, ours, abi, pinned) in [
        ("DotkGap", &t.gap, &manifest.gap_abi, &manifest.gap_template_hash),
        ("DotkDeed", &t.deed, &manifest.deed_abi, &manifest.deed_template_hash),
    ] {
        let theirs = published(abi, what)?;
        ensure!(
            *ours == theirs,
            "{what} compiles to {} bytes with template hash {}, but the deployment publishes {} bytes with template hash {}",
            ours.bytecode.len(),
            hex::encode(&ours.template_hash()),
            theirs.bytecode.len(),
            hex::encode(&theirs.template_hash())
        );
        let hash = hex::encode(&ours.template_hash());
        ensure!(hash == *pinned, "{what} template hash is {hash}, but the deployment pins {pinned}");
    }
    let detail = format!(
        "both contracts compile to the deployed bytecode\n  DotkGap  template hash {} ({} bytes)\n  DotkDeed template hash {} ({} bytes)",
        hex::encode(&t.gap.template_hash()),
        t.gap.bytecode.len(),
        hex::encode(&t.deed.template_hash()),
        t.deed.bytecode.len()
    );
    Ok((t, detail))
}

/// Step 2: the genesis gap built from the compiled bytecode gives the pinned covenant id.
pub fn check_genesis(manifest: &Manifest, t: &Templates) -> Result<(RpcHash, String)> {
    let binding = &manifest.genesis_binding;
    let spk = t.gap.p2sh_spk(&state::gap_state(&KEY_MIN, &KEY_MAX))?;
    let outpoint = TransactionOutpoint::new(
        RpcHash::from_bytes(hex::decode32(&binding.authorizing_outpoint.transaction_id)?),
        binding.authorizing_outpoint.index,
    );
    let output = TransactionOutput::new(manifest.params.gap_value, ScriptPublicKey::from_vec(0, spk));
    let id = covenant_id(outpoint, [(binding.output_index, &output)].into_iter());
    ensure!(
        id.to_string() == REGISTRY_COVENANT_ID,
        "the genesis gap gives covenant id {id}, but the mainnet registry's id is {REGISTRY_COVENANT_ID}"
    );
    ensure!(
        manifest.registry_covenant_id == REGISTRY_COVENANT_ID,
        "the API names covenant id {}, but the mainnet registry's id is {REGISTRY_COVENANT_ID}",
        manifest.registry_covenant_id
    );
    let detail = format!(
        "the genesis gap gives the registry's covenant id\n  covenant id {id}\n  genesis outpoint {}:{}",
        binding.authorizing_outpoint.transaction_id, binding.authorizing_outpoint.index
    );
    Ok((id, detail))
}

fn p2sh_address(template: &Template, state: &[u8]) -> Result<Address> {
    let spk = template.p2sh_spk(state)?;
    Ok(Address::new(Prefix::Mainnet, Version::ScriptHash, &spk[2..34]))
}

/// The addresses that a snapshot implies: every gap between its keys, and every deed it claims.
#[derive(Debug, Clone)]
pub struct Probe {
    pub keys: usize,
    pub gaps: Vec<Address>,
    pub deeds: Vec<(String, Address)>,
    pub pending: usize,
    pub owner_unknown: usize,
}

pub fn probe(snapshot: &Snapshot, t: &Templates, id: RpcHash) -> Result<Probe> {
    ensure!(
        snapshot.registry_covenant_id == id.to_string(),
        "the snapshot is of covenant id {}, not {id}",
        snapshot.registry_covenant_id
    );
    let mut keys = Vec::with_capacity(snapshot.deeds.len());
    let mut deeds = Vec::with_capacity(snapshot.deeds.len());
    let mut pending = 0;
    let mut owner_unknown = 0;
    for deed in &snapshot.deeds {
        let key = hex::decode32(&deed.key)?;
        keys.push(key);
        let (label, state) = match deed.kind {
            KIND_ACTIVE => {
                let name = deed.name.as_deref().ok_or_else(|| anyhow!("active deed {} has no name", deed.key))?;
                ensure!(
                    state::key_of(name) == key,
                    "the name {name:?} hashes to {}, not to {}",
                    hex::encode(&state::key_of(name)),
                    deed.key
                );
                let padded = state::padded_name(name).ok_or_else(|| anyhow!("{name:?} is not 1 to 32 bytes"))?;
                let owner_type = deed.owner_type.ok_or_else(|| anyhow!("deed {name} has no owner type"))?;
                let owner = hex::decode32(deed.owner.as_deref().ok_or_else(|| anyhow!("deed {name} has no owner"))?)?;
                (name.to_string(), state::deed_state(state::STATUS_ACTIVE, &key, owner_type, &owner, &padded))
            }
            KIND_PENDING => {
                pending += 1;
                let claim = hex::decode32(deed.claim.as_deref().ok_or_else(|| anyhow!("pending deed {} has no claim", deed.key))?)?;
                (format!("pending key {}", deed.key), state::pending_deed_state(&key, &claim))
            }
            KIND_OWNER_UNKNOWN => {
                owner_unknown += 1;
                continue;
            }
            other => bail!("deed {} has unknown kind {other}", deed.key),
        };
        deeds.push((label, p2sh_address(&t.deed, &state)?));
    }
    // A repeated key or a keyspace bound would derive an empty gap.
    keys.sort_unstable();
    ensure!(keys.windows(2).all(|w| w[0] < w[1]), "the snapshot lists a key twice");
    ensure!(
        keys.first().is_none_or(|k| *k > KEY_MIN) && keys.last().is_none_or(|k| *k < KEY_MAX),
        "the snapshot lists a keyspace bound as a key"
    );
    let mut gaps = Vec::with_capacity(keys.len() + 1);
    let mut lo = KEY_MIN;
    for key in &keys {
        gaps.push(p2sh_address(&t.gap, &state::gap_state(&lo, key))?);
        lo = *key;
    }
    gaps.push(p2sh_address(&t.gap, &state::gap_state(&lo, &KEY_MAX))?);
    Ok(Probe { keys: keys.len(), gaps, deeds, pending, owner_unknown })
}

/// The addresses among a node's UTXO entries that hold a UTXO carrying `id`. An entry without its
/// address is a malformed answer.
pub fn holding(entries: impl IntoIterator<Item = (Option<Address>, Option<RpcHash>)>, id: RpcHash) -> Result<HashSet<Address>> {
    let mut found = HashSet::new();
    for (address, covenant) in entries {
        let address = address.ok_or_else(|| anyhow!("the node answered a UTXO entry without its address"))?;
        if covenant == Some(id) {
            found.insert(address);
        }
    }
    Ok(found)
}

impl Probe {
    pub fn missing_gaps(&self, found: &HashSet<Address>) -> usize {
        self.gaps.iter().filter(|a| !found.contains(a)).count()
    }

    pub fn missing_deeds(&self, found: &HashSet<Address>) -> Vec<&str> {
        self.deeds.iter().filter(|(_, a)| !found.contains(a)).map(|(n, _)| n.as_str()).collect()
    }

    /// Step 3: every derived gap exists, so the snapshot's keys are exactly the registered keys.
    pub fn check_gaps(&self, found: &HashSet<Address>) -> Result<String> {
        let missing = self.missing_gaps(found);
        ensure!(
            missing == 0,
            "{missing} of {} gaps that the snapshot implies hold no registry UTXO, so the snapshot's keys are not the registered keys",
            self.gaps.len()
        );
        Ok(format!("all {} gaps exist on chain, so the keyspace holds exactly {} registered keys", self.gaps.len(), self.keys))
    }

    /// Step 4: every deed exists with its claimed state. An owner-unknown deed proves no owner,
    /// so it fails the step rather than pass unchecked.
    pub fn check_deeds(&self, found: &HashSet<Address>) -> Result<String> {
        let missing = self.missing_deeds(found);
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(10).copied().collect();
            bail!("{} of {} deeds hold no registry UTXO, for example: {}", missing.len(), self.deeds.len(), shown.join(", "));
        }
        ensure!(self.owner_unknown == 0, "{} deeds are listed as owner-unknown, so their owners cannot be proven", self.owner_unknown);
        let active = self.deeds.len() - self.pending;
        let mut detail =
            format!("all {} deeds exist on chain, {active} active with the snapshot's names and owners", self.deeds.len());
        if self.pending > 0 {
            detail.push_str(&format!(", and {} pending with the snapshot's keys and claims", self.pending));
        }
        Ok(detail)
    }
}
