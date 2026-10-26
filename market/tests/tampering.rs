//! A true market answer proves, and every tampered or wrong answer fails.

use dotk_listing::api;
use dotk_listing::escrow::{FEE_CAP_DIVISOR, Listing, MIN_FEE, RESERVE};
use dotk_listing::verify;
use dotk_registry::hex;
use dotk_registry::verify::MAINNET;

mod common;
use common::*;

/// A one-line change to an answer, and its name for the failure message.
type Tamper<T> = (&'static str, fn(&mut T));

/// A second listing, of a name that never expires.
fn other() -> Listing {
    Listing { name: "bob".to_string(), window: 0, salt_txid: [0x78; 32], ..listing() }
}

fn served() -> Vec<api::Listing> {
    vec![serve(&listing()), serve(&other())]
}

fn utxos() -> Vec<verify::Utxo> {
    let mut all = chain(&listing());
    all.extend(chain(&other()));
    all
}

#[test]
fn the_true_answer_proves() {
    assert!(proves(&served(), utxos()));
    let probe = verify::probe(&served(), &TEMPLATE, &REGISTRY, &MAINNET).unwrap();
    assert_eq!((probe.listings.len(), probe.other_versions), (2, 0));
}

#[test]
fn a_tampered_listing_fails() {
    let tampers: [Tamper<api::Listing>; 23] = [
        ("price", |l| l.price += 1),
        ("fee", |l| l.fee = l.fee.map(|f| f + 1)),
        ("missing fee", |l| l.fee = None),
        ("window", |l| l.window += 1),
        ("seller", |l| l.seller = hex::encode(&[0x12; 32])),
        ("seller type", |l| l.seller_type = 0x85),
        ("venue", |l| l.venue = hex::encode(&[0x45; 32])),
        ("venue type", |l| l.venue_type = 0x85),
        ("salt", |l| l.salt_txid = hex::encode(&[0x79; 32])),
        ("salt index", |l| l.salt_index = 1),
        ("name", |l| l.name = "alicf".to_string()),
        ("key", |l| l.key = hex::encode(&[0x13; 32])),
        ("owner hash", |l| l.owner_hash = hex::encode(&[0x14; 32])),
        ("escrow address", |l| l.escrow_address = l.seller_address.clone()),
        ("deed address", |l| l.deed_address = l.escrow_address.clone()),
        ("seller address", |l| l.seller_address = l.venue_address.clone()),
        ("venue address", |l| l.venue_address = l.seller_address.clone()),
        ("registry", |l| l.registry_covenant_id = "11".repeat(32)),
        ("reserve transaction", |l| l.reserve.txid = hex::encode(&[0x15; 32])),
        ("reserve output", |l| l.reserve.index = 0),
        ("expiry", |l| l.expires_daa_score = l.expires_daa_score.map(|e| e + 1)),
        ("missing expiry", |l| l.expires_daa_score = None),
        ("reserve age", |l| l.reserve.daa_score = l.reserve.daa_score.map(|d| d - 1)),
    ];
    for (what, tamper) in tampers {
        let mut s = served();
        tamper(&mut s[0]);
        assert!(!proves(&s, utxos()), "a changed {what} proved");
    }
}

#[test]
fn an_expiry_on_a_listing_that_never_expires_fails() {
    let mut s = served();
    s[1].expires_daa_score = Some(RESERVE_DAA);
    assert!(!proves(&s, utxos()));
}

/// Terms that a buy refuses fail, even when every served address and hash is the one they give.
#[test]
fn a_listing_that_nobody_can_buy_fails() {
    let low = Listing { fee: MIN_FEE - 1, ..listing() };
    let high = Listing { fee: listing().price / FEE_CAP_DIVISOR + 1, ..listing() };
    for l in [low, high] {
        assert!(!proves(&[serve(&l)], chain(&l)), "a fee of {} proved", l.fee);
    }
}

#[test]
fn a_repeated_listing_fails() {
    let mut s = served();
    s.push(serve(&listing()));
    assert!(!proves(&s, utxos()));
}

#[test]
fn another_escrow_version_is_counted_and_not_proven() {
    let mut s = served();
    s[1].version = 1;
    s[1].owner_hash = hex::encode(&[0x16; 32]);
    let probe = verify::probe(&s, &TEMPLATE, &REGISTRY, &MAINNET).unwrap();
    assert_eq!((probe.listings.len(), probe.other_versions), (1, 1));
    assert!(proves(&s, chain(&listing())));
}

#[test]
fn a_wrong_node_answer_fails() {
    let tampers: [Tamper<Vec<verify::Utxo>>; 7] = [
        ("no deed", |u| drop(u.remove(0))),
        ("a deed of another covenant", |u| u[0].covenant_id = Some([0x11; 32])),
        ("a deed without a covenant", |u| u[0].covenant_id = None),
        ("no reserve", |u| drop(u.remove(1))),
        ("a reserve of another value", |u| u[1].amount = RESERVE + 1),
        ("a reserve of another age", |u| u[1].daa_score += 1),
        ("a reserve at another output", |u| u[1].index = 2),
    ];
    for (what, tamper) in tampers {
        let mut u = utxos();
        tamper(&mut u);
        assert!(!proves(&served(), u), "a node with {what} proved");
    }
}

/// A listing whose reserve the market does not serve still proves its deed, and it claims no
/// expiry.
#[test]
fn a_listing_without_a_served_reserve_proves_its_deed() {
    let mut s = served();
    s[0].reserve.daa_score = None;
    s[0].expires_daa_score = None;
    let mut u = utxos();
    u.remove(1);
    assert!(proves(&s, u.clone()));
    u.remove(0);
    assert!(!proves(&s, u));
}
