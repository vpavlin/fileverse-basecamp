//! logos-pin v1 (docs/PIN-HUB.md): pin requests to the public cache on the VPS, and the
//! hub's signed status. Plain Rust; the Delivery and Storage calls are in the glue.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey, Signature};
use serde_json::{json, Value};
use std::path::Path;

pub const REQUEST_TOPIC: &str = "/logos-pin/1/requests/json";
pub const STATUS_TOPIC: &str = "/logos-pin/1/status/json";
const REQUEST_TAG: &[u8] = b"logos-pin/v1\n";
const STATUS_TAG: &[u8] = b"logos-pin/v1/status\n";
/// At most this many CIDs (storage + ipfs) in one request.
pub const MAX_CIDS: usize = 64;

/// The hub on the VPS (its README): status key, Storage and IPFS peers.
pub const HUB_KEY: &str = "cabe0c212fee03b007dc0057a7e98c45340dfddbf37d4abd22b93d1125c8ba2d";
pub const HUB_STORAGE_PEER: &str = "16Uiu2HAmKRRjSxvkNWM7iHbXuSANCrrLmopXWtQshSDuFQ4YqHuG";
pub const HUB_STORAGE_ADDR: &str = "/ip4/128.140.55.128/tcp/8499";
pub const HUB_IPFS_PEER: &str = "12D3KooWC6fTPiToL8mf6k5z8R1i6Rm1U9aGRvCfLuy9mdNiXfn4";
pub const HUB_IPFS_ADDRS: &[&str] = &["/ip4/128.140.55.128/tcp/4001", "/ip4/128.140.55.128/udp/4001/quic-v1"];

/// JCS (RFC 8785) for what v1 messages hold (strings, integers, arrays, objects): keys sorted,
/// no whitespace. serde_json's map is ordered by key and its escaping is the minimal one JCS
/// wants, for the ASCII keys v1 uses.
pub fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let parts: Vec<String> = keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&m[*k]))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// The bytes a v1 message is signed over: the tag, then the message without `sig`.
pub fn signing_bytes(tag: &[u8], msg: &Value) -> Vec<u8> {
    let mut m = msg.clone();
    if let Some(o) = m.as_object_mut() {
        o.remove("sig");
    }
    let mut out = tag.to_vec();
    out.extend_from_slice(canonical(&m).as_bytes());
    out
}

/// This installation's key: 32 random bytes in `path` (mode 600), made on first use.
pub fn load_or_create_key(path: &Path) -> Result<SigningKey, String> {
    if let Ok(b) = std::fs::read(path) {
        let seed: [u8; 32] = b.as_slice().try_into().map_err(|_| "the pin key file is damaged".to_string())?;
        return Ok(SigningKey::from_bytes(&seed));
    }
    let seed: [u8; 32] = rand::random();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    write_secret(path, &seed)?;
    Ok(SigningKey::from_bytes(&seed))
}

#[cfg(unix)]
fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())
}
#[cfg(not(unix))]
fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

pub fn public_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// A signed v1 request. `kind` "pin" or "unpin".
#[allow(clippy::too_many_arguments)]
pub fn request(key: &SigningKey, kind: &str, id: &str, app: &str, storage: &[String], ipfs: &[String], peer: &str, addrs: &[String], at: u64) -> Value {
    let mut m = json!({
        "v": 1, "type": kind, "id": id, "app": app,
        "storage": storage, "ipfs": ipfs,
        "from": { "storagePeer": peer, "addrs": addrs },
        "at": at, "key": public_hex(key),
    });
    let sig = key.sign(&signing_bytes(REQUEST_TAG, &m));
    m["sig"] = json!(hex::encode(sig.to_bytes()));
    m
}

/// A request id: 16 random bytes, hex.
pub fn new_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// Splits the CIDs into requests of at most MAX_CIDS (storage first).
pub fn chunks(storage: &[String], ipfs: &[String]) -> Vec<(Vec<String>, Vec<String>)> {
    let all: Vec<(bool, &String)> = storage.iter().map(|c| (true, c)).chain(ipfs.iter().map(|c| (false, c))).collect();
    all.chunks(MAX_CIDS)
        .map(|c| (
            c.iter().filter(|(s, _)| *s).map(|(_, x)| (*x).clone()).collect(),
            c.iter().filter(|(s, _)| !*s).map(|(_, x)| (*x).clone()).collect(),
        ))
        .collect()
}

fn verify(tag: &[u8], msg: &Value, key_hex: &str) -> bool {
    let Ok(kb) = hex::decode(key_hex) else { return false };
    let Ok(kb): Result<[u8; 32], _> = kb.try_into() else { return false };
    let Ok(vk) = VerifyingKey::from_bytes(&kb) else { return false };
    let Some(sig) = msg["sig"].as_str().and_then(|s| hex::decode(s).ok()) else { return false };
    let Ok(sig): Result<[u8; 64], _> = sig.try_into() else { return false };
    vk.verify(&signing_bytes(tag, msg), &Signature::from_bytes(&sig)).is_ok()
}

/// A status message from the hub, checked: its key is the hub's and its signature holds.
pub fn status_from_hub(msg: &Value) -> bool {
    msg["v"] == 1 && msg["key"].as_str() == Some(HUB_KEY) && verify(STATUS_TAG, msg, HUB_KEY)
}

/// Checks a request's own signature (for tests and the hub).
pub fn request_ok(msg: &Value) -> bool {
    msg["key"].as_str().is_some_and(|k| verify(REQUEST_TAG, msg, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_sorts_and_compacts() {
        let v = json!({"b": 1, "a": {"z": "é\n\"", "c": [1, "x"]}, "A": true});
        assert_eq!(canonical(&v), r#"{"A":true,"a":{"c":[1,"x"],"z":"é\n\""},"b":1}"#);
    }

    #[test]
    fn request_signs_and_verifies() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let r = request(&key, "pin", "00ff", "fileverse", &["zDvZ".into()], &["bafk".into()], "16Uiu", &["/ip4/1.2.3.4/tcp/1".into()], 1796000000000);
        assert!(request_ok(&r));
        // the parsed text checks too (the verifier recomputes the bytes)
        let back: Value = serde_json::from_str(&serde_json::to_string_pretty(&r).unwrap()).unwrap();
        assert!(request_ok(&back));
        let mut bad = r.clone();
        bad["at"] = json!(1796000000001u64);
        assert!(!request_ok(&bad));
    }

    #[test]
    fn chunks_cap_at_64() {
        let s: Vec<String> = (0..50).map(|i| format!("s{i}")).collect();
        let p: Vec<String> = (0..30).map(|i| format!("p{i}")).collect();
        let c = chunks(&s, &p);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0.len() + c[0].1.len(), 64);
        assert_eq!(c[1].0.len(), 0);
        assert_eq!(c[1].1.len(), 16);
    }

    #[test]
    fn hub_status_needs_hub_key() {
        let fake = SigningKey::from_bytes(&[9u8; 32]);
        let mut m = json!({"v": 1, "id": "x", "state": "kept", "key": public_hex(&fake)});
        let sig = fake.sign(&signing_bytes(STATUS_TAG, &m));
        m["sig"] = json!(hex::encode(sig.to_bytes()));
        assert!(!status_from_hub(&m));
    }
}
