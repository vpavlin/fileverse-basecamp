//! Publishing a Markdown draft as a Fileverse-format document, with no chain and
//! no Fileverse service.
//!
//! The files are what ddocs.new writes for a v3 document (AES-256-GCM content
//! `ct‖tag‖iv`, a nonce-prefixed GCM title, a secretbox link lock), plus a small
//! manifest naming them, because there is no `files(id)` record to do that. Each
//! file is one raw IPFS block, so it can be pinned on any node, copied into Logos
//! Storage and checked against its CID by any reader. The share link is
//! `fvbc://ipfs/<manifest CID>#key=<link secret>`.

use serde_json::{json, Value};

use crate::ipfs::raw_cid;
use crate::{b64, crypto, md};

pub const MANIFEST_TYPE: &str = "fileverse-basecamp/1";
pub const LINK_PREFIX: &str = "fvbc://ipfs/";
/// One block per file: what Kubo's block/put and bitswap accept.
pub const MAX_FILE: usize = 1024 * 1024;

pub struct Published {
    pub link: String,
    pub manifest: String,
    /// Every block (CIDv1, bytes): manifest first.
    pub blocks: Vec<(String, Vec<u8>)>,
}

fn add(blocks: &mut Vec<(String, Vec<u8>)>, what: &str, bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_FILE {
        return Err(format!("the {what} is {} KiB; publishing is limited to {} KiB per file for now", bytes.len() / 1024, MAX_FILE / 1024));
    }
    let cid = raw_cid(&bytes).to_v1();
    blocks.push((cid.clone(), bytes));
    Ok(cid)
}

/// Encrypt and package a draft. `secret` is the draft's link secret (kept per
/// draft so its links share a key); every publish gets a fresh file key.
pub fn build(id: &str, title: &str, markdown: &str, secret: &[u8; 32], now_ms: u64) -> Result<Published, String> {
    let update = md::to_yjs(markdown);
    let file_key: [u8; 32] = crypto::random();
    let iv: [u8; 12] = crypto::random();
    let content = json!({ "file": b64::encode(&update), "source": "ddoc" }).to_string();
    let mut content_blob = crypto::gcm_seal(&file_key, &iv, content.as_bytes())?;
    content_blob.extend_from_slice(&iv);

    let tn: [u8; 12] = crypto::random();
    let mut title_ct = tn.to_vec();
    title_ct.extend(crypto::gcm_seal(&file_key, &tn, title.as_bytes())?);

    let gn: [u8; 24] = crypto::random();
    let gate = json!({
        "lockedFileKey": format!("{}{}{}", b64::encode_url(&gn), crypto::SEP, b64::encode_url(&crypto::secretbox_seal(secret, &gn, &file_key)?)),
    });
    let meta = json!({
        "title": b64::encode(&title_ct),
        "size": content_blob.len(),
        "mimeType": "application/json",
        "kind": "ddoc",
        "version": "4",
        "ddocId": id,
        "nonce": b64::encode(&crypto::random::<24>()),
        "createdAt": now_ms,
        "contentUpdatedAt": now_ms,
        "linkKeyType": "raw",
        "sourceApp": "logos-basecamp",
    });

    let mut blocks = vec![];
    let content_cid = add(&mut blocks, "document", content_blob)?;
    let meta_cid = add(&mut blocks, "metadata", meta.to_string().into_bytes())?;
    let gate_cid = add(&mut blocks, "link lock", gate.to_string().into_bytes())?;
    let manifest = json!({ "type": MANIFEST_TYPE, "metadata": meta_cid, "content": content_cid, "gate": gate_cid });
    let manifest_cid = add(&mut blocks, "manifest", manifest.to_string().into_bytes())?;
    blocks.rotate_right(1);
    Ok(Published {
        link: format!("{LINK_PREFIX}{manifest_cid}#key={}", b64::encode_url(secret)),
        manifest: manifest_cid,
        blocks,
    })
}

/// `fvbc://ipfs/<cid>#key=<secret>` -> (cid, secret).
pub fn parse_link(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix(LINK_PREFIX)?;
    let (cid, frag) = rest.split_once('#')?;
    let key = frag.split('&').find_map(|p| p.strip_prefix("key="))?;
    Some((cid.trim_end_matches('/').to_string(), key.to_string()))
}

pub fn manifest_ok(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some(MANIFEST_TYPE)
}
