//! The state regions of gap and deed UTXOs, byte for byte as the contracts lay them out.

pub const ZERO32: [u8; 32] = [0u8; 32];
pub const KEY_MIN: [u8; 32] = [0u8; 32];
pub const KEY_MAX: [u8; 32] = [0xffu8; 32];

pub const GAP_STATE_LEN: usize = 66;
pub const DEED_STATE_LEN: usize = 103;

pub const STATUS_PENDING: u8 = 0x01;
pub const STATUS_ACTIVE: u8 = 0x02;

const OWNER_PUBKEY: u8 = 0x00;

fn push_byte(out: &mut Vec<u8>, b: u8) {
    out.extend_from_slice(&[0x01, b]);
}

fn push32(out: &mut Vec<u8>, v: &[u8; 32]) {
    out.push(0x20);
    out.extend_from_slice(v);
}

pub fn gap_state(lo: &[u8; 32], hi: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(GAP_STATE_LEN);
    push32(&mut out, lo);
    push32(&mut out, hi);
    out
}

pub fn deed_state(status: u8, key: &[u8; 32], owner_type: u8, owner: &[u8; 32], name: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DEED_STATE_LEN);
    push_byte(&mut out, status);
    push32(&mut out, key);
    push_byte(&mut out, owner_type);
    push32(&mut out, owner);
    push32(&mut out, name);
    out
}

pub fn pending_deed_state(key: &[u8; 32], claim: &[u8; 32]) -> Vec<u8> {
    deed_state(STATUS_PENDING, key, OWNER_PUBKEY, claim, &ZERO32)
}

pub fn padded_name(name: &str) -> Option<[u8; 32]> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out[..bytes.len()].copy_from_slice(bytes);
    Some(out)
}

pub fn key_of(name: &str) -> [u8; 32] {
    *blake3::hash(name.as_bytes()).as_bytes()
}
