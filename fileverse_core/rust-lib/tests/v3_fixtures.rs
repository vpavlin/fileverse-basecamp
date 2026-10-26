//! Offline end-to-end checks of the v3 (ddocs.new / dsheets.new) formats.
//!
//! `fixtures/v3.json` is produced by `tools/make-v3-fixtures.mjs` with the JS
//! primitives Fileverse itself uses. Each case is opened through the real
//! `viewer::open_link`: a mock RPC answers `files(id)` with the ABI-encoded
//! record, and the blobs are served from a block cache that the IPFS layer
//! re-hashes against their CIDs, exactly as it does for gateway answers.

use std::cell::RefCell;

use fileverse_core::chain::Rpc;
use fileverse_core::ipfs::{Cid, Gateways};
use fileverse_core::{b64, crypto, viewer};
use serde_json::Value;

const PORTAL: &str = "0x1111111111111111111111111111111111111111";

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

/// (string appFileId, uint8 fileType, string meta, string content, string gate, uint256 version, address owner)
fn encode_record(app: &str, ft: u64, meta: &str, content: &str, gate: &str) -> String {
    let strings = [app, meta, content, gate];
    let tails: Vec<Vec<u8>> = strings.iter().map(|s| enc_string(s)).collect();
    let mut off = 7 * 32;
    let mut offs = vec![];
    for t in &tails {
        offs.push(off as u64);
        off += t.len();
    }
    let mut head = vec![];
    head.extend(word_u(offs[0]));
    head.extend(word_u(ft));
    head.extend(word_u(offs[1]));
    head.extend(word_u(offs[2]));
    head.extend(word_u(offs[3]));
    head.extend(word_u(0));
    head.extend(word_u(0xabcdef));
    for t in tails {
        head.extend(t);
    }
    format!("0x{}", hex::encode(head))
}

struct MockRpc {
    answer: String,
    calls: RefCell<u32>,
}

impl Rpc for MockRpc {
    fn eth_call(&self, to: &str, _data: &str) -> Result<String, String> {
        assert_eq!(to, PORTAL);
        *self.calls.borrow_mut() += 1;
        Ok(self.answer.clone())
    }
    fn describe(&self) -> String {
        "mock".into()
    }
}

fn fixtures() -> Value {
    serde_json::from_str(include_str!("fixtures/v3.json")).unwrap()
}

fn gateways(fx: &Value) -> (Gateways, tempdir::Dir) {
    let dir = tempdir::Dir::new();
    for (cid, data) in fx["blocks"].as_object().unwrap() {
        let c = Cid::parse(cid).unwrap();
        std::fs::write(dir.path.join(c.to_v1()), b64::decode(data.as_str().unwrap()).unwrap()).unwrap();
    }
    // No real gateway: anything not in the cache must fail, not be fetched.
    (Gateways::new(vec!["http://127.0.0.1:1".into()], Some(dir.path.clone())), dir)
}

mod tempdir {
    pub struct Dir {
        pub path: std::path::PathBuf,
    }
    impl Dir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!("fv-test-{}-{}", std::process::id(), rand_suffix()));
            std::fs::create_dir_all(&path).unwrap();
            Dir { path }
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }
}

fn open_case(fx: &Value, name: &str) -> Value {
    let c = fx["cases"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap();
    let r = &c["record"];
    let rpc = MockRpc {
        answer: encode_record(
            r["appFileId"].as_str().unwrap(),
            r["fileType"].as_u64().unwrap(),
            r["metadata"].as_str().unwrap(),
            r["content"].as_str().unwrap(),
            r["gate"].as_str().unwrap(),
        ),
        calls: RefCell::new(0),
    };
    let (gw, _dir) = gateways(fx);
    let url = format!("https://ddocs.new/{PORTAL}/7#key={}", c["linkKey"].as_str().unwrap());
    let v = viewer::open_link(&rpc, &gw, &url).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(*rpc.calls.borrow(), 1);
    assert_eq!(v["title"], c["title"], "{name}");
    assert_eq!(v["kind"], c["kind"], "{name}");
    assert_eq!(v["layout"], "v3");
    for b in v["provenance"]["blobs"].as_array().unwrap() {
        assert_eq!(b["verified"], true);
    }
    v
}

fn check_doc(v: &Value) {
    let tabs = v["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0]["name"], "Intro");
    assert_eq!(tabs[1]["name"], "Second");
    let b = tabs[0]["blocks"].as_array().unwrap();
    assert_eq!(b[0]["t"], "h2");
    // Document text never becomes markup.
    assert_eq!(b[0]["html"], "Hello &lt;script&gt;");
    assert_eq!(b[1]["html"], "plain <b>bold</b><a href=\"https://logos.co\"> link</a>");
    assert_eq!(b[2]["t"], "li");
    assert_eq!(b[2]["text"], "item one");
    assert_eq!(tabs[1]["blocks"][0]["text"], "on tab two");
    let md = v["markdown"].as_str().unwrap();
    assert!(md.contains("## Hello <script>"), "{md}");
    assert!(md.contains("plain **bold**[ link](https://logos.co)"), "{md}");
    assert!(md.contains("- item one"), "{md}");
}

#[test]
fn ddoc_raw_link_secretbox_gate() {
    check_doc(&open_case(&fixtures(), "ddoc-raw-secretbox"));
}

#[test]
fn ddoc_api_link_argon_and_cbc_gate() {
    check_doc(&open_case(&fixtures(), "ddoc-api-argon-cbc"));
}

#[test]
fn dsheet_gz1() {
    let v = open_case(&fixtures(), "dsheet-gz1");
    let s = &v["sheets"][0];
    assert_eq!(s["name"], "Budget");
    assert_eq!(s["rows"], serde_json::json!([["Item", "Cost"], ["Coffee", "$4.50"]]));
}

#[test]
fn ecies_matches_fileverse_crypto() {
    let fx = fixtures();
    let e = &fx["ecies"];
    let pt = crypto::ecies_open(&b64::decode(e["privateKey"].as_str().unwrap()).unwrap(), e["ciphertext"].as_str().unwrap()).unwrap();
    assert_eq!(pt, b64::decode(e["plaintext"].as_str().unwrap()).unwrap());
}

#[test]
fn wrong_key_is_a_clear_error() {
    let fx = fixtures();
    let c = &fx["cases"][0];
    let r = &c["record"];
    let rpc = MockRpc {
        answer: encode_record("x", 2, r["metadata"].as_str().unwrap(), r["content"].as_str().unwrap(), r["gate"].as_str().unwrap()),
        calls: RefCell::new(0),
    };
    let (gw, _dir) = gateways(&fx);
    let bad = b64::encode(&[7u8; 32]);
    let e = viewer::open_link(&rpc, &gw, &format!("https://ddocs.new/{PORTAL}/7#key={bad}")).unwrap_err();
    assert!(e.contains("does not open"), "{e}");
}

/// The public Fileverse Agents SDK doc (a legacy portal), over the real network:
/// `cargo test --no-default-features -- --ignored live_legacy`
#[test]
#[ignore]
fn live_legacy_share_link() {
    let gw = Gateways::new(fileverse_core::ipfs::DEFAULT_GATEWAYS.iter().map(|s| s.to_string()).collect(), None);
    let rpc = fileverse_core::chain::HttpRpc::new("https://rpc.gnosischain.com");
    let v = viewer::open_link(
        &rpc,
        &gw,
        "https://docs.fileverse.io/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/57#key=VWweDIp0IV7cWWPpYflsPkgEcekIkYXkdPkxfO02R2JbjXq-u1tf6Axsp7824S_7",
    )
    .unwrap();
    assert_eq!(v["title"], "Fileverse Agents SDK - Docs 💛");
    assert_eq!(v["layout"], "legacy");
    assert!(v["markdown"].as_str().unwrap().contains("install @fileverse/agents"));
}

/// A ddocs.new short link's portal, resolved on Gnosis from the registry's Mint log:
/// `cargo test --no-default-features -- --ignored live_short`
#[test]
#[ignore]
fn live_short_link_token() {
    let rpc = fileverse_core::chain::HttpRpc::new("https://rpc.gnosischain.com");
    let p = fileverse_core::chain::resolve_token(&rpc, 2, 1).unwrap();
    assert_eq!(p, "0x6a453c9f14651c05e04ff9ee77ed1ac6f36674e5");
}
