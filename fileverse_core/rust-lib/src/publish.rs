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

/// The images a draft holds on this device (`fvimg:` references with `local`), as a
/// reader will see them: each encrypted with a fresh key (AES-256-GCM, 12-byte IV, the
/// tag kept apart as dDocs does) and stored as UnixFS blocks. Returns the Markdown with
/// those references replaced, and the blocks to publish.
pub fn upload_local_images(
    markdown: &str,
    read_local: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(String, Vec<(String, Vec<u8>)>), String> {
    let mut out = markdown.to_string();
    let mut blocks = vec![];
    let mut done: Vec<String> = vec![];
    for part in markdown.split("](").skip(1) {
        let url = part.split(')').next().unwrap_or("");
        let Some(r) = crate::doc::ImageRef::from_url(url) else { continue };
        if r.local.is_empty() || done.iter().any(|d| d == url) {
            continue;
        }
        let bytes = read_local(&r.local)?;
        let key: [u8; 32] = crypto::random();
        let iv: [u8; 12] = crypto::random();
        let mut ct = crypto::gcm_seal(&key, &iv, &bytes)?;
        let tag = ct.split_off(ct.len() - 16);
        let (cid, b) = crate::ipfs::unixfs_file(&ct);
        blocks.extend(b);
        let up = crate::doc::ImageRef {
            cid,
            key: b64::encode(&key),
            nonce: b64::encode(&iv),
            tag: b64::encode(&tag),
            mime: r.mime.clone(),
            width: r.width.clone(),
            align: r.align.clone(),
            local: String::new(),
        };
        out = out.replace(&format!("]({url})"), &format!("]({})", up.to_url()));
        done.push(url.to_string());
    }
    Ok((out, blocks))
}

pub struct Published {
    pub link: String,
    pub manifest: String,
    /// Every block (CIDv1, bytes): manifest first.
    pub blocks: Vec<(String, Vec<u8>)>,
    /// The manifest as JSON, to add Storage CIDs to once the files are in Logos Storage.
    pub manifest_json: Value,
    pub secret: [u8; 32],
}

/// The manifest again with where each file is in Logos Storage (`"storage": { ipfs CID:
/// [storage CID, size] }`), so a reader anywhere can fetch them from Storage: its block
/// and CID.
pub fn with_storage(manifest: &Value, storage: &[(String, String, u64)]) -> Result<(String, Vec<u8>), String> {
    let mut m = manifest.clone();
    let map: serde_json::Map<String, Value> = storage.iter().map(|(ipfs, ls, n)| (ipfs.clone(), json!([ls, n]))).collect();
    m["storage"] = Value::Object(map);
    let bytes = m.to_string().into_bytes();
    let cid = raw_cid(&bytes).to_v1();
    Ok((cid, bytes))
}

/// The share link: the manifest's IPFS CID, and its Logos Storage CID when it has one.
pub fn link_for(manifest_cid: &str, storage_cid: Option<&str>, secret: &[u8; 32]) -> String {
    match storage_cid {
        Some(ls) if !ls.is_empty() => format!("{LINK_PREFIX}{manifest_cid}?ls={ls}#key={}", b64::encode_url(secret)),
        _ => format!("{LINK_PREFIX}{manifest_cid}#key={}", b64::encode_url(secret)),
    }
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
        link: link_for(&manifest_cid, None, secret),
        manifest: manifest_cid,
        blocks,
        manifest_json: manifest,
        secret: *secret,
    })
}

/// `fvbc://ipfs/<cid>[?ls=<storage cid>]#key=<secret>` -> (cid, secret).
pub fn parse_link(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix(LINK_PREFIX)?;
    let (path, frag) = rest.split_once('#')?;
    let cid = path.split('?').next().unwrap_or("");
    let key = frag.split('&').find_map(|p| p.strip_prefix("key="))?;
    Some((cid.trim_end_matches('/').to_string(), key.to_string()))
}

/// The manifest's Logos Storage CID from a link (`?ls=`), if it has one.
pub fn link_storage(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix(LINK_PREFIX)?;
    let path = rest.split('#').next().unwrap_or("");
    let q = path.split_once('?')?.1;
    q.split('&').find_map(|p| p.strip_prefix("ls=")).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn manifest_ok(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some(MANIFEST_TYPE)
}

#[cfg(test)]
mod storage_link_tests {
    use super::*;

    #[test]
    fn a_link_carries_the_manifests_storage_cid() {
        let secret = [7u8; 32];
        let l = link_for("bafkreimanifest", Some("zDvStorageCid"), &secret);
        assert!(l.starts_with("fvbc://ipfs/bafkreimanifest?ls=zDvStorageCid#key="), "{l}");
        assert_eq!(parse_link(&l).unwrap().0, "bafkreimanifest");
        assert_eq!(link_storage(&l).as_deref(), Some("zDvStorageCid"));
        // old links: no ls, still parsed
        let old = link_for("bafkreimanifest", None, &secret);
        assert_eq!(parse_link(&old).unwrap().0, "bafkreimanifest");
        assert_eq!(link_storage(&old), None);
    }

    #[test]
    fn the_manifest_names_where_each_file_is_in_storage() {
        let p = build("d1", "T", "# T\n\nhello\n", &[1u8; 32], 1).unwrap();
        let list: Vec<(String, String, u64)> = p.blocks.iter().filter(|(c, _)| *c != p.manifest).map(|(c, b)| (c.clone(), format!("ls-{c}"), b.len() as u64)).collect();
        let (cid, bytes) = with_storage(&p.manifest_json, &list).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(manifest_ok(&v));
        assert_eq!(v["storage"].as_object().unwrap().len(), list.len());
        assert_eq!(v["content"], p.manifest_json["content"]);
        assert_eq!(cid, raw_cid(&bytes).to_v1());
        assert_ne!(cid, p.manifest);
    }
}
