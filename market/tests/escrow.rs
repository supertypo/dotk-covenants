//! The published artifact is the source, compiled, and the state encoder writes what the
//! compiler writes.

use dotk_listing::escrow::{
    self, ECDSA_EVEN, ECDSA_ODD, FEE_CAP_DIVISOR, KAS, Listing, MAX_PRICE, MAX_WINDOW, MIN_FEE, MIN_PRICE, SCHNORR,
};
use dotk_registry::hex;
use dotk_registry::verify::MAINNET;
use silverscript_lang::ast::Expr;
use silverscript_lang::compiler::{CompileOptions, compile_contract, sil_abi_artifact_from_compiled};

mod common;
use common::*;

/// The sample state that the artifact carries. Every value is distinct, so a field in the wrong
/// slot shows.
fn sample_ctor() -> Vec<Expr<'static>> {
    vec![
        Expr::byte(SCHNORR),
        Expr::bytes(vec![0x11; 32]),
        Expr::bytes(vec![0x22; 32]),
        Expr::int(1_000_000_000),
        Expr::int(7_776_000),
        Expr::byte(SCHNORR),
        Expr::bytes(vec![0x44; 32]),
        Expr::int(99_999_999),
        Expr::bytes(vec![0x55; 32]),
        Expr::bytes(vec![0x66; 32]),
        Expr::int(1000),
        Expr::int(2000),
        Expr::bytes(vec![0x77; 36]),
    ]
}

#[test]
fn the_artifact_is_the_source_compiled() {
    let ctor = sample_ctor();
    let contract = compile_contract(escrow::SOURCE, &ctor, CompileOptions::default()).unwrap();
    let ours = serde_json::to_value(sil_abi_artifact_from_compiled(&contract, &ctor).unwrap()).unwrap();
    let published: serde_json::Value = serde_json::from_str(escrow::ARTIFACT).unwrap();
    assert_eq!(ours, published);
}

/// The guide states the mainnet deed template's lengths.
#[test]
fn the_mainnet_deed_template_has_the_lengths_that_the_guide_states() {
    assert_eq!((REGISTRY.deed.prefix().len(), REGISTRY.deed.suffix().len()), (1, 3116));
    assert_eq!(hex::encode(&REGISTRY.deed.template_hash()), "238a8d0a5af1ea136d50dd49973487929b8e713a84f1bd1650e97776b9a3f5a0");
}

#[test]
fn the_published_template_has_the_pinned_hash() {
    let t = escrow::published().unwrap();
    assert_eq!(hex::encode(&t.template_hash()), escrow::TEMPLATE_HASH);
    assert_eq!((t.state_start, t.state_len), (1, escrow::STATE_LEN));
    assert_eq!(TEMPLATE.prefix(), t.prefix());
    assert_eq!(TEMPLATE.suffix(), t.suffix());
}

/// Listings at every bound of the terms and under every key scheme.
fn edges() -> Vec<Listing> {
    let base = listing();
    vec![
        base.clone(),
        Listing { price: MIN_PRICE, fee: MIN_FEE, window: 0, ..base.clone() },
        Listing { price: MAX_PRICE, fee: MAX_PRICE / FEE_CAP_DIVISOR, window: MAX_WINDOW, ..base.clone() },
        Listing { seller_type: ECDSA_ODD, venue_type: SCHNORR, ..base.clone() },
        Listing { seller_type: ECDSA_EVEN, venue_type: ECDSA_ODD, ..base.clone() },
        Listing { name: "a".repeat(32), salt_index: 0xdead_beef, seller: [0xff; 32], venue: [0; 32], ..base },
    ]
}

#[test]
fn the_state_encoder_writes_the_compilers_state() {
    for l in edges() {
        let compiled = escrow::compile(&l.ctor(&REGISTRY)).unwrap();
        assert_eq!(TEMPLATE.materialize(&l.state(&REGISTRY)).unwrap(), compiled.bytecode, "the state of {l:?}");
    }
}

#[test]
fn the_terms_keep_the_bounds_that_a_buy_checks() {
    for l in edges() {
        l.check_terms().unwrap();
    }
    let base = listing();
    let refused = [
        ("a fee below the floor", Listing { fee: MIN_FEE - 1, ..base.clone() }),
        ("a fee above a tenth", Listing { fee: base.price / FEE_CAP_DIVISOR + 1, ..base.clone() }),
        ("a price below the floor", Listing { price: MIN_PRICE - 1, fee: MIN_FEE, ..base.clone() }),
        ("a price above the ceiling", Listing { price: MAX_PRICE + 1, ..base.clone() }),
        ("a window above four bytes", Listing { window: MAX_WINDOW + 1, ..base.clone() }),
        ("a P2SH seller", Listing { seller_type: 0x03, ..base.clone() }),
        ("a P2SH venue", Listing { venue_type: 0x03, ..base.clone() }),
        ("an empty name", Listing { name: String::new(), ..base.clone() }),
        ("a name above 32 bytes", Listing { name: "a".repeat(33), ..base }),
    ];
    for (what, l) in refused {
        assert!(l.check_terms().is_err(), "{what} passed");
    }
    assert_eq!(MIN_PRICE, 2 * KAS);
}

/// A fixed listing on the mainnet registry, for a builder to test its encoder against.
#[test]
fn a_fixed_listing_keeps_its_escrow() {
    let l = listing();
    let e = escrow::escrow(&TEMPLATE, &l, &REGISTRY).unwrap();
    assert_eq!(hex::encode(&e.owner_hash), "c19d1d3a207f1844c29549b91044dd2fea83752187f9c939b43e1c349911a0f2");
    assert_eq!(
        escrow::escrow_address(&e, MAINNET.prefix).to_string(),
        "kaspa:prqe68f6ypl3s3xzj4ymjyzym5h74qm4yxrlnjfekslpcdyezxs0y6hne75lv"
    );
    assert_eq!(
        escrow::deed_address(&REGISTRY, &l, &e, MAINNET.prefix).unwrap().to_string(),
        "kaspa:pr0rt8znmdk2valc0ka0fuz2hp64kl5ej64memne8t5c5erw67qhzm737j5qc"
    );
}
