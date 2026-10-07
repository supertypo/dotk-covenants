//! The real mainnet answer proves, and every tampered or wrong answer fails.

use std::collections::HashSet;
use std::sync::LazyLock;

use dotk_covenants::api::{Deed, GenesisBinding, KIND_ACTIVE, KIND_OWNER_UNKNOWN, KIND_PENDING, Manifest, Outpoint, Params, Snapshot};
use dotk_covenants::compile::{self, Templates};
use dotk_covenants::verify::{self, REGISTRY_COVENANT_ID};
use dotk_covenants::{hex, state};
use kaspa_addresses::Address;
use kaspa_rpc_core::RpcHash;

/// A one-line change to an answer, and its name for the failure message.
type Tamper<T> = (&'static str, fn(&mut T));

fn params() -> Params {
    Params {
        devfund_spk: "207ee85afca8273d94739037e7b4c736fcfc9e12d5c468eea5ce7b89181b49386bac".to_string(),
        fee_1ch: 399_800_000_000,
        fee_2ch: 199_800_000_000,
        fee_3ch: 99_800_000_000,
        fee_4ch: 24_800_000_000,
        fee_5plus: 3_800_000_000,
        bond: 100_000_000,
        deposit: 3_600_000_000,
        gap_value: 100_000_000,
        t_evict: 3000,
    }
}

static TEMPLATES: LazyLock<Templates> = LazyLock::new(|| compile::compile(&params()).unwrap());

fn id() -> RpcHash {
    RpcHash::from_bytes(hex::decode32(REGISTRY_COVENANT_ID).unwrap())
}

/// The mainnet manifest, with the deployed template hashes and genesis outpoint.
fn manifest() -> Manifest {
    Manifest {
        network: "mainnet".to_string(),
        registry_covenant_id: REGISTRY_COVENANT_ID.to_string(),
        gap_abi: TEMPLATES.gap_abi.clone(),
        deed_abi: TEMPLATES.deed_abi.clone(),
        gap_template_hash: "ba1e7b8e2dfda606a902c89437625b1c31f82668cd8cfdc7cf365a8ad77c5a3d".to_string(),
        deed_template_hash: "238a8d0a5af1ea136d50dd49973487929b8e713a84f1bd1650e97776b9a3f5a0".to_string(),
        params: params(),
        genesis_binding: GenesisBinding {
            authorizing_outpoint: Outpoint {
                transaction_id: "e077087a4761098dcb55a953057b4a92aadf0f834c7df85f53c8bae8634d5d10".to_string(),
                index: 0,
            },
            output_index: 0,
        },
    }
}

fn active(name: &str, owner: u8) -> Deed {
    Deed {
        key: hex::encode(&state::key_of(name)),
        kind: KIND_ACTIVE,
        name: Some(name.to_string()),
        owner_type: Some(0),
        owner: Some(hex::encode(&[owner; 32])),
        claim: None,
    }
}

fn pending(name: &str, claim: u8) -> Deed {
    Deed { kind: KIND_PENDING, name: None, owner_type: None, owner: None, claim: Some(hex::encode(&[claim; 32])), ..active(name, 0) }
}

fn snapshot() -> Snapshot {
    Snapshot {
        registry_covenant_id: REGISTRY_COVENANT_ID.to_string(),
        deeds: vec![active("alice", 1), active("bob", 2), pending("eve", 3)],
    }
}

/// What the node holds for the true registry: every gap and every deed.
fn chain() -> HashSet<Address> {
    let p = verify::probe(&snapshot(), &TEMPLATES, id()).unwrap();
    p.gaps.into_iter().chain(p.deeds.into_iter().map(|(_, a)| a)).collect()
}

/// The positions of the deeds with the lowest and the highest key.
fn by_key(s: &Snapshot) -> (usize, usize) {
    let order = |i: &usize| s.deeds[*i].key.clone();
    let all = 0..s.deeds.len();
    (all.clone().min_by_key(order).unwrap(), all.max_by_key(order).unwrap())
}

/// Steps 1 and 2.
fn proves_deployment(m: &Manifest) -> bool {
    verify::check_compile(m).and_then(|(t, _)| verify::check_genesis(m, &t)).is_ok()
}

/// Steps 3 and 4.
fn proves_registry(s: &Snapshot, node: &HashSet<Address>) -> bool {
    let Ok(p) = verify::probe(s, &TEMPLATES, id()) else { return false };
    p.check_gaps(node).is_ok() && p.check_deeds(node).is_ok()
}

#[test]
fn the_true_answer_proves() {
    assert!(proves_deployment(&manifest()));
    assert!(proves_registry(&snapshot(), &chain()));
}

#[test]
fn a_tampered_manifest_fails() {
    let tampers: [Tamper<Manifest>; 6] = [
        ("a fee", |m| m.params.fee_5plus -= 1),
        ("the devfund", |m| m.params.devfund_spk = m.params.devfund_spk.replacen("7e", "7f", 1)),
        ("the bytecode", |m| m.gap_abi.contracts.values_mut().next().unwrap().compiled.bytecode[200] ^= 1),
        ("the network", |m| m.network = "testnet-10".to_string()),
        ("the covenant id", |m| m.registry_covenant_id = "11".repeat(32)),
        ("the genesis outpoint", |m| m.genesis_binding.authorizing_outpoint.index = 1),
    ];
    for (what, tamper) in tampers {
        let mut m = manifest();
        tamper(&mut m);
        assert!(!proves_deployment(&m), "a changed {what} proved");
    }
}

#[test]
fn a_tampered_snapshot_fails() {
    let tampers: [Tamper<Snapshot>; 11] = [
        ("dropped name", |s| drop(s.deeds.remove(0))),
        ("dropped lowest name", |s| drop(s.deeds.remove(by_key(s).0))),
        ("dropped highest name", |s| drop(s.deeds.remove(by_key(s).1))),
        ("added name", |s| s.deeds.push(active("mallory", 4))),
        ("repeated name", |s| s.deeds.push(active("alice", 1))),
        ("wrong name", |s| s.deeds[0] = active("alicf", 1)),
        ("wrong owner", |s| s.deeds[1].owner = Some(hex::encode(&[9; 32]))),
        ("wrong owner type", |s| s.deeds[1].owner_type = Some(0x86)),
        ("wrong claim", |s| s.deeds[2].claim = Some(hex::encode(&[9; 32]))),
        ("hidden owner", |s| s.deeds[1].kind = KIND_OWNER_UNKNOWN),
        ("other covenant", |s| s.registry_covenant_id = "11".repeat(32)),
    ];
    for (what, tamper) in tampers {
        let mut s = snapshot();
        tamper(&mut s);
        assert!(!proves_registry(&s, &chain()), "a {what} proved");
    }
}

#[test]
fn a_wrong_node_answer_fails() {
    for missing in chain() {
        let mut node = chain();
        node.remove(&missing);
        assert!(!proves_registry(&snapshot(), &node), "a node without {missing} proved");
    }
    for wrong in [None, Some(RpcHash::from_bytes([0x11; 32]))] {
        let entries = chain().into_iter().enumerate().map(|(i, a)| (Some(a), if i == 0 { wrong } else { Some(id()) }));
        assert!(!proves_registry(&snapshot(), &verify::holding(entries, id()).unwrap()), "a UTXO with covenant id {wrong:?} counted");
    }
    assert!(verify::holding([(None, Some(id()))], id()).is_err(), "an entry without its address counted");
}
