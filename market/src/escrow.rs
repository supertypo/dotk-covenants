//! The listing escrow: its template, its state, and the addresses that a listing's terms give.

use anyhow::{Result, anyhow, bail, ensure};
use dotk_registry::compile::Template;
use dotk_registry::state::{self, STATUS_ACTIVE};
use dotk_registry::verify::p2sh_address_on;
use kaspa_addresses::{Address, Prefix, Version};
use silverscript_lang::ast::Expr;
use silverscript_lang::compiler::{CompileOptions, compile_contract};

pub const SOURCE: &str = include_str!("../sil/dotk_listing.v2.sil");
pub const ARTIFACT: &str = include_str!("../sil/dotk_listing.v2.json");

/// Pinned, so the proof compares the compiler with a hash that no API serves.
pub const TEMPLATE_HASH: &str = "cfc5bc3f94cb3c8d80b30091adaa126ce159e076e72aa13952ac3bde7de179e4";

/// The listing version of this escrow, as the payload and the market state it.
pub const VERSION: u8 = 2;
pub const STATE_LEN: usize = 251;

pub const KAS: u64 = 100_000_000;
pub const MIN_FEE: u64 = 20_000_000;
pub const FEE_CAP_DIVISOR: u64 = 10;
pub const MIN_PRICE: u64 = MIN_FEE * FEE_CAP_DIVISOR;
pub const MAX_PRICE: u64 = 80_000_000 * KAS;
/// The payload states the window in four bytes.
pub const MAX_WINDOW: u64 = u32::MAX as u64;
pub const RESERVE: u64 = 50_000_000;
/// The listing transaction's output that holds the reserve.
pub const RESERVE_INDEX: u32 = 1;

/// The deed's owner type of a P2SH owner.
pub const OWNER_SCRIPT_HASH: u8 = 0x03;
pub const SCHNORR: u8 = 0x00;
pub const ECDSA_ODD: u8 = 0x85;
pub const ECDSA_EVEN: u8 = 0x86;

/// What a listing's state takes from the registry: the covenant id, and the deed template.
pub struct Registry {
    pub covenant_id: [u8; 32],
    pub deed: Template,
}

/// A listing's terms, everything that its escrow state holds besides the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub seller_type: u8,
    pub seller: [u8; 32],
    pub price: u64,
    pub fee: u64,
    pub window: u64,
    pub venue_type: u8,
    pub venue: [u8; 32],
    pub salt_txid: [u8; 32],
    pub salt_index: u32,
}

fn key_scheme(what: &str, scheme: u8) -> Result<()> {
    ensure!(matches!(scheme, SCHNORR | ECDSA_ODD | ECDSA_EVEN), "the {what} scheme is {scheme:#04x}, not a key scheme");
    Ok(())
}

fn push_byte(out: &mut Vec<u8>, b: u8) {
    out.extend_from_slice(&[0x01, b]);
}

fn push32(out: &mut Vec<u8>, v: &[u8; 32]) {
    out.push(0x20);
    out.extend_from_slice(v);
}

fn push_int(out: &mut Vec<u8>, v: u64) {
    out.push(0x08);
    out.extend_from_slice(&v.to_le_bytes());
}

fn int(v: u64) -> Expr<'static> {
    Expr::int(i64::try_from(v).expect("every term and length fits a script integer"))
}

impl Listing {
    /// The terms under which a buy can pass and the market serves the listing.
    pub fn check_terms(&self) -> Result<()> {
        ensure!(state::padded_name(&self.name).is_some(), "the name {:?} is not 1 to 32 bytes", self.name);
        key_scheme("seller", self.seller_type)?;
        key_scheme("venue", self.venue_type)?;
        ensure!((MIN_PRICE..=MAX_PRICE).contains(&self.price), "the price {} is outside {MIN_PRICE} to {MAX_PRICE}", self.price);
        ensure!(self.fee >= MIN_FEE, "the fee {} is below {MIN_FEE}", self.fee);
        ensure!(self.fee <= self.price / FEE_CAP_DIVISOR, "the fee {} is above a tenth of the price {}", self.fee, self.price);
        ensure!(self.window <= MAX_WINDOW, "the window {} is above {MAX_WINDOW}", self.window);
        Ok(())
    }

    pub fn key(&self) -> [u8; 32] {
        state::key_of(&self.name)
    }

    pub fn salt(&self) -> [u8; 36] {
        let mut salt = [0u8; 36];
        salt[..32].copy_from_slice(&self.salt_txid);
        salt[32..].copy_from_slice(&self.salt_index.to_le_bytes());
        salt
    }

    /// The escrow's state span, field by field as the source declares it.
    pub fn state(&self, r: &Registry) -> Vec<u8> {
        let mut out = Vec::with_capacity(STATE_LEN);
        push_byte(&mut out, self.seller_type);
        push32(&mut out, &self.seller);
        push32(&mut out, &self.key());
        push_int(&mut out, self.price);
        push_int(&mut out, self.window);
        push_byte(&mut out, self.venue_type);
        push32(&mut out, &self.venue);
        push_int(&mut out, self.fee);
        push32(&mut out, &r.covenant_id);
        push32(&mut out, &r.deed.template_hash());
        push_int(&mut out, r.deed.prefix().len() as u64);
        push_int(&mut out, r.deed.suffix().len() as u64);
        out.push(0x24);
        out.extend_from_slice(&self.salt());
        out
    }

    /// The same fields as constructor arguments, which the compiler writes into the state span.
    pub fn ctor(&self, r: &Registry) -> Vec<Expr<'static>> {
        vec![
            Expr::byte(self.seller_type),
            Expr::bytes(self.seller.to_vec()),
            Expr::bytes(self.key().to_vec()),
            int(self.price),
            int(self.window),
            Expr::byte(self.venue_type),
            Expr::bytes(self.venue.to_vec()),
            int(self.fee),
            Expr::bytes(r.covenant_id.to_vec()),
            Expr::bytes(r.deed.template_hash().to_vec()),
            int(r.deed.prefix().len() as u64),
            int(r.deed.suffix().len() as u64),
            Expr::bytes(self.salt().to_vec()),
        ]
    }
}

/// The source compiled over `ctor`.
pub fn compile(ctor: &[Expr<'static>]) -> Result<Template> {
    let c = compile_contract(SOURCE, ctor, CompileOptions::default()).map_err(|e| anyhow!("compiling DotkListing: {e}"))?;
    let t = Template { bytecode: c.bytecode.clone(), state_start: c.state_layout.start, state_len: c.state_layout.len };
    ensure!(t.state_len == STATE_LEN, "DotkListing state is {} bytes, expected {STATE_LEN}", t.state_len);
    Ok(t)
}

/// The template of the source. Every state field has a fixed size, so the bytes around the state
/// do not depend on the listing.
pub fn template(r: &Registry) -> Result<Template> {
    let placeholder = Listing { name: "a".to_string(), seller: [0; 32], venue: [0; 32], salt_txid: [0; 32], ..sample_terms() };
    compile(&placeholder.ctor(r))
}

/// Terms that every rule accepts, for a placeholder.
fn sample_terms() -> Listing {
    Listing {
        name: String::new(),
        seller_type: SCHNORR,
        seller: [0; 32],
        price: MIN_PRICE,
        fee: MIN_FEE,
        window: 0,
        venue_type: SCHNORR,
        venue: [0; 32],
        salt_txid: [0; 32],
        salt_index: 0,
    }
}

/// The template of the published artifact.
pub fn published() -> Result<Template> {
    let abi: silverscript_abi::SilAbiArtifact = serde_json::from_str(ARTIFACT)?;
    let mut contracts = abi.contracts.values();
    let (Some(contract), None) = (contracts.next(), contracts.next()) else {
        bail!("the escrow artifact must hold exactly one contract");
    };
    let c = &contract.compiled;
    ensure!(
        c.state_span.offset.checked_add(c.state_span.len).is_some_and(|end| end <= c.bytecode.len()),
        "the escrow artifact's state region lies outside its bytecode"
    );
    Ok(Template { bytecode: c.bytecode.clone(), state_start: c.state_span.offset, state_len: c.state_span.len })
}

/// One listing's escrow script and its hash, the deed's owner.
pub struct Escrow {
    pub script: Vec<u8>,
    pub owner_hash: [u8; 32],
}

pub fn escrow(t: &Template, l: &Listing, r: &Registry) -> Result<Escrow> {
    let state = l.state(r);
    let script = t.materialize(&state)?;
    let spk = t.p2sh_spk(&state)?;
    let owner_hash = spk[2..34].try_into().expect("a P2SH script holds a 32-byte hash");
    Ok(Escrow { script, owner_hash })
}

pub fn escrow_address(e: &Escrow, prefix: Prefix) -> Address {
    Address::new(prefix, Version::ScriptHash, &e.owner_hash)
}

/// The address of the name's deed while the escrow owns it.
pub fn deed_address(r: &Registry, l: &Listing, e: &Escrow, prefix: Prefix) -> Result<Address> {
    let name = state::padded_name(&l.name).ok_or_else(|| anyhow!("the name {:?} is not 1 to 32 bytes", l.name))?;
    p2sh_address_on(&r.deed, &state::deed_state(STATUS_ACTIVE, &l.key(), OWNER_SCRIPT_HASH, &e.owner_hash, &name), prefix)
}

/// The address that pays a key owner: the seller's payout, or the venue's fee.
pub fn key_address(scheme: u8, key: &[u8; 32], prefix: Prefix) -> Result<Address> {
    let parity = match scheme {
        SCHNORR => return Ok(Address::new(prefix, Version::PubKey, key)),
        ECDSA_EVEN => 0x02,
        ECDSA_ODD => 0x03,
        other => bail!("scheme {other:#04x} is not a key scheme"),
    };
    let mut point = vec![parity];
    point.extend_from_slice(key);
    Ok(Address::new(prefix, Version::PubKeyECDSA, &point))
}
