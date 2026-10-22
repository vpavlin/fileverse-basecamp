//! The few Solidity ABI shapes the viewer reads: `getFileCount()` and the two
//! `files(uint256)` layouts (legacy Portal and v3 App). Hand-rolled so the module
//! does not pull a full EVM stack in for three calls.

use sha3::{Digest, Keccak256};

pub fn selector(signature: &str) -> [u8; 4] {
    let h = Keccak256::digest(signature.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

/// Calldata for a function taking zero or one uint256.
pub fn calldata(signature: &str, arg: Option<u64>) -> String {
    let mut out = selector(signature).to_vec();
    if let Some(v) = arg {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&v.to_be_bytes());
        out.extend_from_slice(&word);
    }
    format!("0x{}", hex::encode(out))
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Uint,
    Address,
    Str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Uint(u128),
    Address(String),
    Str(String),
}

impl Val {
    pub fn as_str(&self) -> &str {
        match self {
            Val::Str(s) | Val::Address(s) => s,
            Val::Uint(_) => "",
        }
    }
    pub fn as_u128(&self) -> u128 {
        match self {
            Val::Uint(v) => *v,
            _ => 0,
        }
    }
}

pub fn hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s.trim().trim_start_matches("0x")).map_err(|e| format!("bad hex: {e}"))
}

fn word(data: &[u8], at: usize) -> Result<&[u8], String> {
    data.get(at..at + 32).ok_or_else(|| format!("ABI data too short at {at}"))
}

fn word_uint(w: &[u8]) -> Result<u128, String> {
    if w[..16].iter().any(|b| *b != 0) {
        return Err("uint does not fit 128 bits".into());
    }
    Ok(u128::from_be_bytes(w[16..32].try_into().unwrap()))
}

/// Decode a tuple of static uints/addresses and dynamic strings.
pub fn decode(data: &[u8], kinds: &[Kind]) -> Result<Vec<Val>, String> {
    let mut out = Vec::with_capacity(kinds.len());
    for (i, k) in kinds.iter().enumerate() {
        let w = word(data, i * 32)?;
        out.push(match k {
            Kind::Uint => Val::Uint(word_uint(w)?),
            Kind::Address => Val::Address(format!("0x{}", hex::encode(&w[12..32]))),
            Kind::Str => {
                let off = word_uint(w)? as usize;
                let len = word_uint(word(data, off)?)? as usize;
                let bytes = data
                    .get(off + 32..off + 32 + len)
                    .ok_or("ABI string out of range")?;
                Val::Str(String::from_utf8_lossy(bytes).into_owned())
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors() {
        assert_eq!(hex::encode(selector("getFileCount()")), "bab50cc9");
    }

    #[test]
    fn string_and_uint() {
        // (string "ab", uint 7)
        let mut d = vec![0u8; 32 * 4];
        d[31] = 0x40;
        d[63] = 7;
        d[95] = 2;
        d[96] = b'a';
        d[97] = b'b';
        let v = decode(&d, &[Kind::Str, Kind::Uint]).unwrap();
        assert_eq!(v, vec![Val::Str("ab".into()), Val::Uint(7)]);
    }
}
