//! The mainnet registry, compiled from its parameters, and a listing on it that the market serves.

#![allow(dead_code)]

use std::sync::LazyLock;

use dotk_listing::api;
use dotk_listing::escrow::{self, ECDSA_EVEN, Listing, RESERVE, RESERVE_INDEX, Registry, SCHNORR};
use dotk_listing::verify::{self, Found, Utxo};
use dotk_registry::api::Params;
use dotk_registry::compile::{self, Template};
use dotk_registry::hex;
use dotk_registry::verify::{MAINNET, REGISTRY_COVENANT_ID};

pub fn params() -> Params {
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

pub static REGISTRY: LazyLock<Registry> = LazyLock::new(|| Registry {
    covenant_id: hex::decode32(REGISTRY_COVENANT_ID).unwrap(),
    deed: compile::compile(&params()).unwrap().deed,
});

pub static TEMPLATE: LazyLock<Template> = LazyLock::new(|| escrow::template(&REGISTRY).unwrap());

pub const LISTED_TXID: [u8; 32] = [0xa1; 32];
pub const RESERVE_DAA: u64 = 400_000_000;

/// A listing with a window, by a schnorr seller at an even ECDSA venue. Both keys are points on
/// the curve.
pub fn listing() -> Listing {
    Listing {
        name: "alice".to_string(),
        seller_type: SCHNORR,
        seller: hex::decode32("62c0a046dacce86ddd0343c6d3c7c79c2208ba0d9c9cf24a6d046d21d21f90f7").unwrap(),
        price: 25 * escrow::KAS,
        fee: escrow::KAS / 2,
        window: 864_000,
        venue_type: ECDSA_EVEN,
        venue: hex::decode32("989c0b76cb563971fdc9bef31ec06c3560f3249d6ee9e5d83c57625596e05f6f").unwrap(),
        salt_txid: [0x77; 32],
        salt_index: 0,
    }
}

/// What a true market serves for `l`.
pub fn serve(l: &Listing) -> api::Listing {
    let p = MAINNET.prefix;
    let e = escrow::escrow(&TEMPLATE, l, &REGISTRY).unwrap();
    api::Listing {
        name: l.name.clone(),
        key: hex::encode(&l.key()),
        owner_hash: hex::encode(&e.owner_hash),
        version: escrow::VERSION,
        price: l.price,
        fee: Some(l.fee),
        window: l.window,
        seller_type: l.seller_type,
        seller: hex::encode(&l.seller),
        seller_address: escrow::key_address(l.seller_type, &l.seller, p).unwrap().to_string(),
        venue_type: l.venue_type,
        venue: hex::encode(&l.venue),
        venue_address: escrow::key_address(l.venue_type, &l.venue, p).unwrap().to_string(),
        salt_txid: hex::encode(&l.salt_txid),
        salt_index: l.salt_index,
        registry_covenant_id: REGISTRY_COVENANT_ID.to_string(),
        deed_address: escrow::deed_address(&REGISTRY, l, &e, p).unwrap().to_string(),
        escrow_address: escrow::escrow_address(&e, p).to_string(),
        listed_txid: hex::encode(&LISTED_TXID),
        reserve: api::Reserve { txid: hex::encode(&LISTED_TXID), index: RESERVE_INDEX, daa_score: Some(RESERVE_DAA) },
        expires_daa_score: (l.window > 0).then_some(RESERVE_DAA + l.window),
    }
}

/// What the node holds for a true listing: its deed, and its reserve at the escrow.
pub fn chain(l: &Listing) -> Vec<Utxo> {
    let p = MAINNET.prefix;
    let e = escrow::escrow(&TEMPLATE, l, &REGISTRY).unwrap();
    vec![
        Utxo {
            address: escrow::deed_address(&REGISTRY, l, &e, p).unwrap(),
            txid: LISTED_TXID,
            index: 0,
            amount: 100_000_000,
            daa_score: RESERVE_DAA,
            covenant_id: Some(REGISTRY.covenant_id),
        },
        Utxo {
            address: escrow::escrow_address(&e, p),
            txid: LISTED_TXID,
            index: RESERVE_INDEX,
            amount: RESERVE,
            daa_score: RESERVE_DAA,
            covenant_id: None,
        },
    ]
}

/// Whether the served listings and the node's UTXOs prove.
pub fn proves(served: &[api::Listing], utxos: Vec<Utxo>) -> bool {
    let found: Found = verify::by_address(utxos);
    verify::probe(served, &TEMPLATE, &REGISTRY, &MAINNET).is_ok_and(|p| p.check_chain(&found, &REGISTRY).is_ok())
}
