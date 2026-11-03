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
