//! Edit -> publish -> open: a Markdown draft becomes Fileverse-format files and a
//! manifest, and the published link opens with no chain and no gateway, from
//! the hash-checked block cache alone.

use fileverse_core::chain::Rpc;
use fileverse_core::ipfs::{BlockSource, Cid, Gateways};
use fileverse_core::{publish, viewer};

struct NoChain;
impl Rpc for NoChain {
    fn eth_call(&self, _: &str, _: &str) -> Result<String, String> {
        panic!("a published draft must not touch the chain")
    }
    fn describe(&self) -> String {
        "none".into()
    }
}

/// A source that holds the published blocks, like a Logos Storage mirror.
struct Mirror(Vec<(String, Vec<u8>)>, bool);
impl BlockSource for Mirror {
    fn name(&self) -> String {
        "mirror".into()
    }
    fn block(&self, cid: &Cid) -> Option<Vec<u8>> {
        let want = cid.to_v1();
        self.0.iter().find(|(c, _)| *c == want).map(|(_, b)| if self.1 { b"tampered".to_vec() } else { b.clone() })
    }
}

const MD: &str = "# Plan\n\nWe go **self-sovereign**.\n\n## Steps\n\n- IPFS\n- Logos Storage\n";

#[test]
fn publish_then_open_from_a_mirror() {
    let p = publish::build("draft1", "Our plan", MD, &[5u8; 32], 1792700000000).unwrap();
    assert!(p.link.starts_with("fvbc://ipfs/bafkrei"));
    assert_eq!(p.blocks.len(), 4);
    assert_eq!(p.blocks[0].0, p.manifest);
    let mut gw = Gateways::new(vec!["http://127.0.0.1:1".into()], None);
    gw.extra.push(Box::new(Mirror(p.blocks.clone(), false)));
    let v = viewer::open_link(&NoChain, &gw, &p.link).unwrap();
    assert_eq!(v["title"], "Our plan");
    assert_eq!(v["layout"], "logos");
    let blocks = v["tabs"][0]["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["t"], "h1");
    assert_eq!(blocks[1]["html"], "We go <b>self-sovereign</b>.");
    // H1 title slide, its paragraph, the H2 section
    assert_eq!(v["tabs"][0]["slides"].as_array().unwrap().len(), 3);
    for b in v["provenance"]["blobs"].as_array().unwrap() {
        assert_eq!(b["gateway"], "mirror");
        assert_eq!(b["verified"], true);
    }
}

#[test]
fn a_tampering_mirror_is_refused() {
    let p = publish::build("d", "t", MD, &[5u8; 32], 1).unwrap();
    let mut gw = Gateways::new(vec!["http://127.0.0.1:1".into()], None);
    gw.extra.push(Box::new(Mirror(p.blocks.clone(), true)));
    let e = viewer::open_link(&NoChain, &gw, &p.link).unwrap_err();
    assert!(e.contains("does not match its CID"), "{e}");
}

#[test]
fn the_wrong_key_does_not_open_it() {
    let p = publish::build("d", "t", MD, &[5u8; 32], 1).unwrap();
    let mut gw = Gateways::new(vec!["http://127.0.0.1:1".into()], None);
    gw.extra.push(Box::new(Mirror(p.blocks.clone(), false)));
    let bad = format!("{}#key={}", p.link.split('#').next().unwrap(), fileverse_core::b64::encode_url(&[6u8; 32]));
    assert!(viewer::open_link(&NoChain, &gw, &bad).unwrap_err().contains("does not open"));
}

#[test]
fn too_large_is_a_clear_error() {
    let big = "word ".repeat(300_000);
    let e = publish::build("d", "t", &big, &[5u8; 32], 1).err().unwrap();
    assert!(e.contains("limited to"), "{e}");
}
