//! "Your own files": a backup key opens a portal's files with no share link.
//! `fixtures/own.json` comes from `tools/make-own-fixtures.mjs` (WebCrypto RSA-OAEP
//! and @noble ECIES, as Fileverse uses them). The mock RPC answers getFileCount()
//! and files(i) for two portals; blobs come from a hash-checked block cache.

use std::collections::HashMap;

use fileverse_core::abi;
use fileverse_core::chain::Rpc;
use fileverse_core::ipfs::{Cid, Gateways};
use fileverse_core::{b64, backup};
use serde_json::Value;

fn word_u(v: u64) -> Vec<u8> {
    let mut w = vec![0u8; 32];
    w[24..].copy_from_slice(&v.to_be_bytes());
    w
}

fn enc_string(s: &str) -> Vec<u8> {
    let mut out = word_u(s.len() as u64);
    let mut b = s.as_bytes().to_vec();
    b.resize(s.len().div_ceil(32) * 32, 0);
    out.extend(b);
    out
}

/// ABI-encode a tuple of (is_string, value) words.
fn encode(items: &[(bool, String)]) -> String {
    let mut head = vec![];
    let mut tail = vec![];
    let base = items.len() * 32;
    for (is_str, v) in items {
        if *is_str {
            head.extend(word_u((base + tail.len()) as u64));
            tail.extend(enc_string(v));
        } else {
            head.extend(word_u(v.parse().unwrap()));
        }
    }
    head.extend(tail);
    format!("0x{}", hex::encode(head))
}

struct Portals(HashMap<String, Vec<Value>>);

impl Rpc for Portals {
    fn eth_call(&self, to: &str, data: &str) -> Result<String, String> {
        let files = self.0.get(to).ok_or("unknown contract")?;
        let d = abi::hex_bytes(data)?;
        if d[..4] == abi::selector("getFileCount()") {
            return Ok(encode(&[(false, files.len().to_string())]));
        }
        let id = u64::from_be_bytes(d[28..36].try_into().unwrap()) as usize;
        let f = files.get(id).ok_or("no such file")?;
        let s = |k: &str| f[k].as_str().unwrap_or("").to_string();
        if f.get("legacy").is_some() {
            // (metadata, content, gate, fileType, version)
            return Ok(encode(&[(true, s("metadata")), (true, s("content")), (true, s("gate")), (false, "1".into()), (false, "0".into())]));
        }
        Ok(encode(&[
            (true, s("appFileId")),
            (false, f["fileType"].to_string()),
            (true, s("metadata")),
            (true, s("content")),
            (true, s("gate")),
            (false, "0".into()),
            (false, "0".into()),
        ]))
    }
    fn describe(&self) -> String {
        "mock".into()
    }
}

fn setup() -> (Value, Portals, Gateways, std::path::PathBuf) {
    let fx: Value = serde_json::from_str(include_str!("fixtures/own.json")).unwrap();
    let dir = std::env::temp_dir().join(format!("fv-own-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (cid, data) in fx["blocks"].as_object().unwrap() {
        std::fs::write(dir.join(Cid::parse(cid).unwrap().to_v1()), b64::decode(data.as_str().unwrap()).unwrap()).unwrap();
    }
    let portals = fx["portals"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_array().unwrap().clone())).collect();
    let gw = Gateways::new(vec!["http://127.0.0.1:1".into()], Some(dir.clone()));
    (fx, Portals(portals), gw, dir)
}

#[test]
fn parses_a_mixed_backup() {
    let (fx, ..) = setup();
    let keys = backup::parse(&fx["backup"].to_string()).unwrap();
    let mut kinds: Vec<_> = keys.iter().map(|k| (k.portal.clone(), k.kind.clone())).collect();
    kinds.sort();
    assert_eq!(kinds, vec![
        ("0x2222222222222222222222222222222222222222".to_string(), "ec".to_string()),
        ("0x3333333333333333333333333333333333333333".to_string(), "legacy".to_string()),
    ]);
    assert!(backup::parse("{\"x\":1}").is_err());
}

#[test]
fn lists_and_opens_a_current_portal() {
    let (fx, rpc, gw, dir) = setup();
    let keys = backup::parse(&fx["backup"]["current"].to_string()).unwrap().remove(0);
    let list = backup::list(&rpc, &gw, &keys, 50).unwrap();
    // newest first; the deleted file is not listed; the PQ file is listed with a reason
    let ids: Vec<u64> = list.iter().map(|e| e["fileId"].as_u64().unwrap()).collect();
    assert_eq!(ids, vec![2, 1, 0]);
    assert_eq!(list[1]["title"], "My sheet");
    assert_eq!(list[1]["kind"], "dsheet");
    assert_eq!(list[2]["title"], "My notes");
    assert!(list[0]["error"].as_str().unwrap().contains("post-quantum"), "{}", list[0]);

    let doc = backup::open(&rpc, &gw, &keys, 0).unwrap();
    assert_eq!(doc["title"], "My notes");
    assert_eq!(doc["tabs"][0]["blocks"][0]["text"], "My notes body");
    let sheet = backup::open(&rpc, &gw, &keys, 1).unwrap();
    assert_eq!(sheet["sheets"][0]["rows"][0][0], "own sheet");
    assert!(backup::open(&rpc, &gw, &keys, 2).unwrap_err().contains("post-quantum"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opens_a_legacy_rsa_portal() {
    let (fx, rpc, gw, dir) = setup();
    let keys = backup::parse(&fx["backup"]["legacy"].to_string()).unwrap().remove(0);
    let list = backup::list(&rpc, &gw, &keys, 50).unwrap();
    assert_eq!(list[2]["title"], "Old doc");
    let doc = backup::open(&rpc, &gw, &keys, 0).unwrap();
    assert_eq!(doc["layout"], "legacy");
    assert_eq!(doc["tabs"][0]["blocks"][0]["text"], "Legacy body");
    let _ = std::fs::remove_dir_all(dir);
}

/// Older docs seal the title with the share-link secret: it is opened from
/// `ownerLock.lockedLinkKey` when present, else taken from the document's text.
#[test]
fn titles_sealed_with_the_link_secret() {
    let (fx, rpc, gw, dir) = setup();
    let keys = backup::parse(&fx["backup"]["legacy"].to_string()).unwrap().remove(0);
    let list = backup::list(&rpc, &gw, &keys, 50).unwrap();
    assert_eq!(list[0]["title"], "Sealed title 2", "{}", list[0]);
    assert!(list[0].get("titleFromText").is_none());
    assert_eq!(list[1]["title"], "Heading of doc 1", "{}", list[1]);
    assert_eq!(list[1]["titleFromText"], true);
    assert!(list[2].get("titleFromText").is_none());

    let doc = backup::open(&rpc, &gw, &keys, 2).unwrap();
    assert_eq!(doc["title"], "Sealed title 2");
    let doc = backup::open(&rpc, &gw, &keys, 1).unwrap();
    assert_eq!(doc["title"], "Heading of doc 1");
    assert_eq!(doc["titleFromText"], true);
    assert_eq!(doc["tabs"][0]["blocks"][1]["text"], "Body 1");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_wrong_key_says_so() {
    let (fx, rpc, gw, dir) = setup();
    let mut b = fx["backup"]["current"].clone();
    b["appDecryptionKey"] = Value::String(b64::encode(&[9u8; 32]));
    let keys = backup::parse(&b.to_string()).unwrap().remove(0);
    let e = backup::open(&rpc, &gw, &keys, 0).unwrap_err();
    assert!(e.contains("authentication failed"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_listed_file_can_be_fetched_for_backup() {
    let (fx, rpc, gw, dir) = setup();
    let keys = backup::parse(&fx["backup"]["current"].to_string()).unwrap().remove(0);
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let list = backup::list_with_progress(&rpc, &gw, &keys, 50, &|_, _| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })
    .unwrap();
    // progress was reported for every slot read (4 on chain, the deleted one included)
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(list[2]["cids"].as_array().unwrap().len(), 3);
    let got = backup::fetch_files(&gw, &list, &|_, _| {});
    assert_eq!(got.len(), 3);
    for g in &got {
        assert!(g.get("error").is_none(), "{g}");
        // metadata + content (no gate on these fixtures): two raw blocks
        assert_eq!(g["blocks"].as_array().unwrap().len(), 2, "{g}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
