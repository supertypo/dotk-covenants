use anyhow::{Result, anyhow};

pub fn decode(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(anyhow!("{text:?} is not hex: odd length"));
    }
    let mut out = vec![0u8; text.len() / 2];
    faster_hex::hex_decode(text.as_bytes(), &mut out).map_err(|e| anyhow!("{text:?} is not hex: {e}"))?;
    Ok(out)
}

pub fn decode32(text: &str) -> Result<[u8; 32]> {
    decode(text)?.try_into().map_err(|_| anyhow!("{text:?} is not 32 bytes"))
}

pub fn encode(bytes: &[u8]) -> String {
    faster_hex::hex_string(bytes)
}
