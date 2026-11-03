//! Images a dDoc embeds: AES-256-GCM as Penumbra writes it (12-byte IV, tag in the
//! node, ciphertext alone on IPFS). `fixtures/image.json` was made with WebCrypto.

use fileverse_core::doc::ImageRef;
use fileverse_core::ipfs::{Cid, Gateways};
use fileverse_core::{b64, viewer};
use serde_json::Value;

fn setup() -> (Value, Gateways, std::path::PathBuf) {
    let fx: Value = serde_json::from_str(include_str!("fixtures/image.json")).unwrap();
    let dir = std::env::temp_dir().join(format!("fv-img-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cid = fx["ref"]["cid"].as_str().unwrap();
    std::fs::write(dir.join(Cid::parse(cid).unwrap().to_v1()), b64::decode(fx["block"].as_str().unwrap()).unwrap()).unwrap();
    // no gateway answers: the block comes from the hash-checked cache
    (fx, Gateways::new(vec!["http://127.0.0.1:1".into()], Some(dir.clone())), dir)
}

#[test]
fn an_embedded_image_decrypts() {
    let (fx, gw, dir) = setup();
    let r: ImageRef = serde_json::from_value(fx["ref"].clone()).unwrap();
    let (mime, bytes) = viewer::open_image(&gw, &r).unwrap();
    assert_eq!(mime, "image/png");
    assert_eq!(bytes, b64::decode(fx["png"].as_str().unwrap()).unwrap());
    // without a MIME type, the bytes say what they are
    let (mime, _) = viewer::open_image(&gw, &ImageRef { mime: String::new(), ..r.clone() }).unwrap();
    assert_eq!(mime, "image/png");
    // a wrong key or tag fails
    let bad = ImageRef { tag: b64::encode(&[0u8; 16]), ..r };
    assert!(viewer::open_image(&gw, &bad).unwrap_err().contains("does not open"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A draft's own image: published encrypted (as dDocs stores images) and read back
/// through the same path a reader uses.
#[test]
fn a_drafts_image_is_published_encrypted() {
    use fileverse_core::publish;
    let picture: Vec<u8> = std::iter::once(0x89u8).chain(b"PNG\r\n\x1a\n".iter().copied()).chain((0..600_000).map(|i| (i % 251) as u8)).collect();
    let local = ImageRef { local: "d1/pic.png".into(), mime: "image/png".into(), width: "50%".into(), ..Default::default() };
    let md = format!("# Deck\n\n![a picture]({})\n", local.to_url());
    let (out, blocks) = publish::upload_local_images(&md, |l| {
        assert_eq!(l, "d1/pic.png");
        Ok(picture.clone())
    })
    .unwrap();
    assert!(!out.contains(&local.to_url()));
    let url = out.split("](").nth(1).unwrap().split(')').next().unwrap();
    let r = ImageRef::from_url(url).unwrap();
    assert!(r.local.is_empty() && !r.cid.is_empty() && r.width == "50%");
    assert!(blocks.len() > 1, "a 600 KB image takes several blocks");
    // a reader with only the blocks (no gateway) opens it
    let dir = std::env::temp_dir().join(format!("fv-pub-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gw = Gateways::new(vec!["http://127.0.0.1:1".into()], Some(dir.clone()));
    for (c, b) in &blocks {
        gw.cache_block(&Cid::parse(c).unwrap(), b);
    }
    let (mime, bytes) = viewer::open_image(&gw, &r).unwrap();
    assert_eq!((mime.as_str(), bytes.len()), ("image/png", picture.len()));
    assert_eq!(bytes, picture);
    let _ = std::fs::remove_dir_all(dir);
}
