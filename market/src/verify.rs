use std::collections::HashMap;

use anyhow::{Result, anyhow, ensure};
use dotk_registry::api::Manifest;
use dotk_registry::compile::Template;
use dotk_registry::hex;
use dotk_registry::verify::{self as registry, Network};
use kaspa_addresses::Address;

use crate::api;
use crate::escrow::{self, Listing, RESERVE, RESERVE_INDEX, Registry, VERSION};

/// Step 1: the registry runs on its contracts, which gives the deed template.
pub fn check_registry(manifest: &Manifest, network: &Network) -> Result<(Registry, String)> {
    let (t, _) = registry::check_compile_on(manifest, network)?;
    let (id, _) = registry::check_genesis_on(manifest, &t, network)?;
    let deed_hash = hex::encode(&t.deed.template_hash());
    let detail =
        format!("the {} registry runs on its contracts\n  covenant id {id}\n  DotkDeed template hash {deed_hash}", network.name);
    Ok((Registry { covenant_id: id.as_bytes(), deed: t.deed }, detail))
}

/// Step 2: the escrow source compiles to the published template.
pub fn check_escrow(r: &Registry) -> Result<(Template, String)> {
    let ours = escrow::template(r)?;
    let theirs = escrow::published()?;
    ensure!(
        ours.prefix() == theirs.prefix() && ours.suffix() == theirs.suffix() && ours.state_len == theirs.state_len,
        "the escrow source compiles to template hash {}, but the artifact holds template hash {}",
        hex::encode(&ours.template_hash()),
        hex::encode(&theirs.template_hash())
    );
    let hash = hex::encode(&ours.template_hash());
    ensure!(hash == escrow::TEMPLATE_HASH, "the escrow's template hash is {hash}, but this repository pins {}", escrow::TEMPLATE_HASH);
    let detail = format!(
        "the escrow source compiles to the published template\n  DotkListing template hash {hash} ({} bytes)",
        ours.bytecode.len()
    );
    Ok((ours, detail))
}

fn equal(what: &str, served: &str, ours: &str) -> Result<()> {
    ensure!(served == ours, "the API serves {what} {served}, but the terms give {ours}");
    Ok(())
}

/// A served listing's terms, read from the hex the API serves.
pub fn terms(l: &api::Listing) -> Result<Listing> {
    Ok(Listing {
        name: l.name.clone(),
        seller_type: l.seller_type,
        seller: hex::decode32(&l.seller)?,
        price: l.price,
        fee: l.fee.ok_or_else(|| anyhow!("the API serves no fee"))?,
        window: l.window,
        venue_type: l.venue_type,
        venue: hex::decode32(&l.venue)?,
        salt_txid: hex::decode32(&l.salt_txid)?,
        salt_index: l.salt_index,
    })
}

/// What the node must hold for one listing.
#[derive(Debug, Clone)]
pub struct Expected {
    pub name: String,
    pub deed: Address,
    pub escrow: Address,
    /// The listing transaction, whose output 0 is the deed and output 1 the reserve.
    pub listed_txid: [u8; 32],
    /// The reserve's DAA score, when the API serves a reserve.
    pub reserve_daa: Option<u64>,
}

/// Every claim of one listing that its terms settle, without the node.
pub fn rebuild(l: &api::Listing, t: &Template, r: &Registry, network: &Network) -> Result<Expected> {
    let terms = terms(l)?;
    terms.check_terms()?;
    equal("the key", &l.key, &hex::encode(&terms.key()))?;
    equal("the registry covenant id", &l.registry_covenant_id, network.registry_covenant_id)?;
    let e = escrow::escrow(t, &terms, r)?;
    equal("the owner hash", &l.owner_hash, &hex::encode(&e.owner_hash))?;
    let escrow_address = escrow::escrow_address(&e, network.prefix);
    equal("the escrow address", &l.escrow_address, &escrow_address.to_string())?;
    let deed = escrow::deed_address(r, &terms, &e, network.prefix)?;
    equal("the deed address", &l.deed_address, &deed.to_string())?;
    let seller = escrow::key_address(terms.seller_type, &terms.seller, network.prefix)?;
    equal("the seller address", &l.seller_address, &seller.to_string())?;
    let venue = escrow::key_address(terms.venue_type, &terms.venue, network.prefix)?;
    equal("the venue address", &l.venue_address, &venue.to_string())?;
    equal("the reserve transaction", &l.reserve.txid, &l.listed_txid)?;
    ensure!(l.reserve.index == RESERVE_INDEX, "the API serves the reserve at output {}, not {RESERVE_INDEX}", l.reserve.index);
    let expires = l.reserve.daa_score.filter(|_| terms.window > 0).and_then(|daa| daa.checked_add(terms.window));
    ensure!(
        l.expires_daa_score == expires,
        "the API serves expiry {:?}, but the reserve and the window give {expires:?}",
        l.expires_daa_score
    );
    let listed_txid = hex::decode32(&l.listed_txid)?;
    Ok(Expected { name: l.name.clone(), deed, escrow: escrow_address, listed_txid, reserve_daa: l.reserve.daa_score })
}

/// The answer to a lookup of `name`: the listing of that name, under this escrow.
pub fn check_named(l: &api::Listing, name: &str) -> Result<()> {
    ensure!(l.name == name, "asked for {name}, the market serves the listing of {}", l.name);
    ensure!(l.version == VERSION, "the market serves a version {} listing of {name}, which this repository does not cover", l.version);
    Ok(())
}

/// The listings that this escrow covers, rebuilt, and the count of the others.
#[derive(Debug, Clone)]
pub struct Probe {
    pub listings: Vec<Expected>,
    pub other_versions: usize,
}

pub fn probe(served: &[api::Listing], t: &Template, r: &Registry, network: &Network) -> Result<Probe> {
    let mut listings = Vec::new();
    let mut other_versions = 0;
    let mut names = std::collections::HashSet::new();
    for l in served {
        ensure!(names.insert(l.name.as_str()), "the API serves two listings of {}", l.name);
        if l.version != VERSION {
            other_versions += 1;
            continue;
        }
        listings.push(rebuild(l, t, r, network).map_err(|e| e.context(format!("the listing of {}", l.name)))?);
    }
    Ok(Probe { listings, other_versions })
}

/// One UTXO in a node's answer.
#[derive(Debug, Clone)]
pub struct Utxo {
    pub address: Address,
    pub txid: [u8; 32],
    pub index: u32,
    pub amount: u64,
    pub daa_score: u64,
    pub covenant_id: Option<[u8; 32]>,
}

/// The node's UTXOs, by address.
pub type Found = HashMap<Address, Vec<Utxo>>;

pub fn by_address(utxos: impl IntoIterator<Item = Utxo>) -> Found {
    let mut found = Found::new();
    for u in utxos {
        found.entry(u.address.clone()).or_default().push(u);
    }
    found
}

impl Probe {
    pub fn addresses(&self) -> Vec<Address> {
        self.listings.iter().flat_map(|l| [l.deed.clone(), l.escrow.clone()]).collect()
    }

    /// The listings whose deed or reserve the node does not show as served.
    ///
    /// The deed must sit at output 0 of the served listing transaction, because `expire` takes
    /// only the reserve beside it. While the escrow holds the deed, no entrypoint has spent that
    /// reserve, so a reserve that the API leaves out must be absent on chain too.
    pub fn missing(&self, found: &Found, r: &Registry) -> Vec<String> {
        let mut missing = Vec::new();
        for l in &self.listings {
            let deed = found
                .get(&l.deed)
                .and_then(|us| us.iter().find(|u| u.covenant_id == Some(r.covenant_id) && u.txid == l.listed_txid && u.index == 0));
            let reserve = found
                .get(&l.escrow)
                .and_then(|us| us.iter().find(|u| u.txid == l.listed_txid && u.index == RESERVE_INDEX && u.amount == RESERVE));
            let problem = match (deed, reserve, l.reserve_daa) {
                (None, ..) => "no deed in its escrow at the listing transaction",
                (Some(_), Some(_), None) => "a reserve that the market does not serve",
                (Some(_), None, Some(_)) => "no reserve as served",
                (Some(d), Some(u), Some(daa)) if u.daa_score != daa || d.daa_score != daa => "a reserve of another age",
                _ => continue,
            };
            missing.push(format!("{} ({problem})", l.name));
        }
        missing
    }

    /// Step 4: every listing's deed sits in its escrow, and every served reserve exists.
    pub fn check_chain(&self, found: &Found, r: &Registry) -> Result<String> {
        let missing = self.missing(found, r);
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(10).map(String::as_str).collect();
            anyhow::bail!("{} of {} listings fail on chain, for example: {}", missing.len(), self.listings.len(), shown.join(", "));
        }
        Ok(match self.listings.len() {
            0 => "no listing of this escrow to look up".to_string(),
            n => format!("all {n} listings hold their deed in their escrow on chain"),
        })
    }
}
