//! Share link -> rendered document, using only a Gnosis RPC and IPFS.
//!
//! The steps mirror what ddocs.new does when a link is opened, with every
//! Fileverse service taken out of the path:
//!   1. read `files(id)` on the portal contract           (chain::read_file)
//!   2. fetch the metadata, gate and content CIDs         (ipfs::Gateways, hash-checked)
//!   3. recover the link secret from `#key=`              (raw, or Argon2id-wrapped)
//!   4. open the link lock to get the file key            (secretbox, or PBKDF2 + AES-CBC)
//!   5. decrypt content and title                         (AES-256-GCM)
//!   6. decode the Yjs state and render it                (doc / sheet)

use serde::Serialize;
use serde_json::{json, Value};

use crate::chain::{self, FileRecord, Rpc};
use crate::ipfs::{Gateways, Provenance};
use crate::{b64, crypto, doc, sheet};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Link {
    pub portal: String,
    pub file_id: u64,
    #[serde(skip)]
    pub key: String,
    pub host: String,
}

/// `https://<host>/<portal>/<fileId>#key=<linkKey>` in any of its spellings
/// (docs.fileverse.io, sheets.fileverse.io, ddocs.new, dsheets.new, ...).
pub fn parse_link(url: &str) -> Result<Link, String> {
    let url = url.trim();
    let (path, frag) = url.split_once('#').ok_or("the link has no #key= part; it is needed to decrypt")?;
    let key = frag
        .split(['&', '#'])
        // `#key=` (older links) or `#k=` (current ddocs.new links)
        .find_map(|p| p.strip_prefix("key=").or_else(|| p.strip_prefix("k=")))
        .ok_or("the link has no key= in its fragment")?
        .to_string();
    let host = path
        .split("//")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or("")
        .to_string();
    let segs: Vec<&str> = path.split(['/', '?']).collect();
    for w in segs.windows(2) {
        if chain::valid_address(w[0]) {
            if let Ok(id) = w[1].parse::<u64>() {
                return Ok(Link { portal: w[0].to_string(), file_id: id, key, host });
            }
        }
    }
    Err("could not find <portal address>/<file number> in the link".into())
}

pub(crate) enum FileKey {
    /// Legacy portals: AES key + IV + tag, the tag kept apart from the content.
    Legacy { key: Vec<u8>, iv: Vec<u8>, tag: Vec<u8> },
    /// v3: a bare AES-256 key; IV and tag ride at the end of the content.
    Raw(Vec<u8>),
}

pub(crate) fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn link_secret(link_key: &[u8], meta: &Value, steps: &mut Vec<String>) -> Result<Vec<u8>, String> {
    if link_key.len() == 32 {
        steps.push("link key is the raw link secret".into());
        return Ok(link_key.to_vec());
    }
    let ddoc_id = str_field(meta, "ddocId");
    let nonce = b64::decode(str_field(meta, "nonce"))?;
    if ddoc_id.is_empty() || nonce.len() != 24 {
        return Err("the file's metadata has no ddocId/nonce to unwrap the link key with".into());
    }
    for m in crypto::LINK_ARGON_MEMORY_KIB {
        let wrap = crypto::link_wrapping_key(ddoc_id, &nonce, *m)?;
        if let Some(s) = crypto::secretbox_open(&wrap, &nonce, link_key) {
            steps.push(format!("link key unwrapped with Argon2id (m={} MiB)", m / 1024));
            return Ok(s);
        }
    }
    Err("the link key does not open this file (wrong or truncated link?)".into())
}

pub(crate) fn file_key_of(plain: &[u8]) -> Result<FileKey, String> {
    if plain.first() == Some(&b'{') {
        let v: Value = serde_json::from_slice(plain).map_err(|e| format!("file key: {e}"))?;
        return Ok(FileKey::Legacy {
            key: b64::decode(str_field(&v, "key"))?,
            iv: b64::decode(str_field(&v, "iv"))?,
            tag: b64::decode(str_field(&v, "authTag"))?,
        });
    }
    if plain.len() == 32 {
        return Ok(FileKey::Raw(plain.to_vec()));
    }
    // Some writers lock the base64 text of the key rather than its bytes.
    let k = b64::decode(std::str::from_utf8(plain).map_err(|_| "file key has an unknown format")?)?;
    if k.len() == 32 {
        return Ok(FileKey::Raw(k));
    }
    Err(format!("file key has an unknown format ({} bytes)", plain.len()))
}

fn open_link_lock(lock: &Value, secret: &[u8], steps: &mut Vec<String>) -> Result<FileKey, String> {
    let locked = str_field(lock, "lockedFileKey");
    if locked.is_empty() {
        return Err("this file has no link lock: it is not shared by link".into());
    }
    let material = str_field(lock, "keyMaterial");
    if !material.is_empty() {
        // Fileverse API layout: salt__n__nonce__n__secretbox(ikm); lockedFileKey = iv__n__AES-CBC.
        let parts: Vec<&str> = material.split(crypto::SEP).collect();
        if parts.len() != 3 {
            return Err("link lock key material has an unknown format".into());
        }
        let salt = b64::decode(parts[0])?;
        let ikm = crypto::secretbox_open(secret, &b64::decode(parts[1])?, &b64::decode(parts[2])?)
            .ok_or("the link key does not open this file's link lock")?;
        let dk = crypto::pbkdf2_32(&ikm, &salt);
        let (iv, ct) = locked.split_once(crypto::SEP).ok_or("bad lockedFileKey")?;
        let fk = crypto::cbc_open(&dk, &b64::decode(iv)?, &b64::decode(ct)?)?;
        steps.push("link lock opened (PBKDF2 + AES-CBC)".into());
        return file_key_of(&fk);
    }
    let fk = crypto::open_nonce_appended(locked, secret).ok_or("the link key does not open this file's link lock")?;
    steps.push("link lock opened (secretbox)".into());
    file_key_of(&fk)
}

/// Legacy portals seal the title with the link secret and the metadata nonce;
/// v3 files with the file key (nonce-prefixed GCM). Unreadable titles stay empty.
/// The title with the file key, and the link secret when the caller has one
/// (backup-key opens find it in `ownerLock.lockedLinkKey`, when that exists).
pub(crate) fn title_with(meta: &Value, fk: &FileKey, secret: &[u8]) -> String {
    decrypt_title(meta, fk, secret)
}

/// Unwrap a share-link key (as found in a URL fragment) to the link secret.
pub(crate) fn link_secret_of(link_key: &[u8], meta: &Value) -> Result<Vec<u8>, String> {
    link_secret(link_key, meta, &mut vec![])
}

/// A stand-in title from the document itself: its first heading, else its first
/// line of text. Older dDocs seal the title with the share-link secret only, which
/// a backup key cannot open; Fileverse's own app then shows "Untitled" unless the
/// browser remembers the title.
pub(crate) fn title_from_tabs(tabs: &[doc::Tab]) -> String {
    let blocks = || tabs.iter().flat_map(|t| t.blocks.iter()).filter(|b| !b.text.trim().is_empty());
    let first = blocks().find(|b| b.t.starts_with('h')).or_else(|| blocks().find(|b| b.t != "code"));
    let line = first.map(|b| b.text.lines().next().unwrap_or("").trim().to_string()).unwrap_or_default();
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>().trim_end())
    } else {
        line
    }
}

/// Fetch and decrypt a dDoc's content just to name it (see `title_from_tabs`).
pub(crate) fn title_from_content(gw: &Gateways, content_cid: &str, fk: &FileKey) -> Result<String, String> {
    let (blob, _) = gw.get(content_cid)?;
    let content: Value = serde_json::from_slice(&decrypt_content(&blob, fk)?).map_err(|e| format!("decrypted content is not JSON: {e}"))?;
    let ydoc = doc::load(&yjs_bytes(content.get("file").unwrap_or(&Value::Null))?)?;
    Ok(title_from_tabs(&doc::tabs(&ydoc)))
}

fn decrypt_title(meta: &Value, fk: &FileKey, secret: &[u8]) -> String {
    let t = str_field(meta, "title");
    let Ok(ct) = b64::decode(t) else { return String::new() };
    if let Ok(nonce) = b64::decode(str_field(meta, "nonce")) {
        if let Some(b) = crypto::secretbox_open(secret, &nonce, &ct) {
            return String::from_utf8_lossy(&b).into_owned();
        }
    }
    let r = match fk {
        FileKey::Legacy { key, iv, .. } => crypto::gcm_open(key, iv, &ct),
        FileKey::Raw(key) => crypto::gcm_open_nonce_prefixed(key, &ct),
    };
    r.ok().and_then(|b| String::from_utf8(b).ok()).unwrap_or_default()
}

fn decrypt_content(blob: &[u8], fk: &FileKey) -> Result<Vec<u8>, String> {
    match fk {
        FileKey::Legacy { key, iv, tag } => {
            let mut ct = blob.to_vec();
            ct.extend(tag);
            crypto::gcm_open(key, iv, &ct)
        }
        FileKey::Raw(key) => crypto::gcm_open_tag_iv_suffixed(key, blob),
    }
}

fn yjs_bytes(file: &Value) -> Result<Vec<u8>, String> {
    let s = file.as_str().ok_or("this file's content is not a Yjs document (older format)")?;
    if let Some(z) = s.strip_prefix("gz1:") {
        let mut out = vec![];
        use std::io::Read;
        flate2::read::GzDecoder::new(&b64::decode(z)?[..])
            .read_to_end(&mut out)
            .map_err(|e| format!("gz1 content: {e}"))?;
        return Ok(out);
    }
    b64::decode(s)
}

pub(crate) fn fetch_json(gw: &Gateways, cid: &str, prov: &mut Vec<Provenance>) -> Result<Value, String> {
    let (b, p) = gw.get(cid)?;
    prov.push(p);
    serde_json::from_slice(&b).map_err(|e| format!("{cid} is not JSON: {e}"))
}

/// A document published from Basecamp: manifest -> metadata, gate, content.
pub fn open_published(gw: &Gateways, url: &str) -> Result<Value, String> {
    let (cid, key) = crate::publish::parse_link(url).ok_or("not a fvbc:// link")?;
    let mut prov = vec![];
    let manifest = fetch_json(gw, &cid, &mut prov)?;
    if !crate::publish::manifest_ok(&manifest) {
        return Err("this CID is not a published Basecamp document".into());
    }
    let mut steps = vec![format!("read the manifest {cid} (no chain record)")];
    let meta = fetch_json(gw, str_field(&manifest, "metadata"), &mut prov)?;
    let lock = fetch_json(gw, str_field(&manifest, "gate"), &mut prov)?;
    let secret = link_secret(&b64::decode(&key)?, &meta, &mut steps)?;
    let fk = open_link_lock(&lock, &secret, &mut steps)?;
    let rec = FileRecord {
        portal: String::new(),
        layout: chain::Layout::Logos,
        file_id: 0,
        app_file_id: str_field(&meta, "ddocId").to_string(),
        file_type: 2,
        metadata_cid: str_field(&manifest, "metadata").to_string(),
        content_cid: str_field(&manifest, "content").to_string(),
        gate_cid: str_field(&manifest, "gate").to_string(),
        version: 0,
        owner: String::new(),
    };
    let mut out = finish(gw, &rec, &meta, &fk, &secret, prov, steps, "IPFS / Logos Storage".into())?;
    out["manifest"] = Value::String(cid);
    Ok(out)
}

pub fn open_link(rpc: &dyn Rpc, gw: &Gateways, url: &str) -> Result<Value, String> {
    if url.trim().starts_with(crate::publish::LINK_PREFIX) {
        return open_published(gw, url);
    }
    let link = parse_link(url)?;
    let mut steps = vec![];
    let rec: FileRecord = chain::read_file(rpc, &link.portal, link.file_id)?;
    steps.push(format!("read files({}) on {} ({:?} portal)", rec.file_id, rpc.describe(), rec.layout));
    let mut prov = vec![];
    let meta = fetch_json(gw, &rec.metadata_cid, &mut prov)?;
    if meta.get("isDeleted").and_then(Value::as_bool) == Some(true) {
        return Err("this file was deleted by its owner".into());
    }
    let lock = if !rec.gate_cid.is_empty() {
        fetch_json(gw, &rec.gate_cid, &mut prov)?
    } else {
        meta.get("linkLock").cloned().unwrap_or(Value::Null)
    };
    let secret = link_secret(&b64::decode(&link.key)?, &meta, &mut steps)?;
    let fk = open_link_lock(&lock, &secret, &mut steps)?;
    finish(gw, &rec, &meta, &fk, &secret, prov, steps, rpc.describe())
}

/// Fetch and decrypt the content, then render it. Shared by every way of getting
/// a file key: a share link, a backup key, or a published draft.
pub(crate) fn finish(
    gw: &Gateways,
    rec: &FileRecord,
    meta: &Value,
    fk: &FileKey,
    title_secret: &[u8],
    mut prov: Vec<Provenance>,
    mut steps: Vec<String>,
    source: String,
) -> Result<Value, String> {
    let (blob, p) = gw.get(&rec.content_cid)?;
    prov.push(p);
    let plain = decrypt_content(&blob, fk)?;
    steps.push("content decrypted (AES-256-GCM)".into());
    let content: Value = serde_json::from_slice(&plain).map_err(|e| format!("decrypted content is not JSON: {e}"))?;
    let title = decrypt_title(meta, fk, title_secret);
    let kind = {
        let k = str_field(meta, "kind");
        let s = str_field(&content, "source");
        if k == "dsheet" || s == "dsheet" || meta.get("dsheetId").is_some() { "dsheet" } else { "ddoc" }
    };
    let ydoc = doc::load(&yjs_bytes(content.get("file").unwrap_or(&Value::Null))?)?;
    let mut out = json!({
        "ok": true,
        "kind": kind,
        "title": title,
        "portal": rec.portal,
        "fileId": rec.file_id,
        "layout": rec.layout,
        "chainId": chain::GNOSIS_CHAIN_ID,
        "record": rec,
        "permissionType": str_field(meta, "permissionType"),
        "createdAt": meta.get("createdAt").cloned().unwrap_or(Value::Null),
        "updatedAt": meta.get("contentUpdatedAt").or_else(|| meta.get("last_updated")).cloned().unwrap_or(Value::Null),
        "provenance": { "rpc": source, "blobs": prov, "steps": steps },
    });
    if kind == "dsheet" {
        let sheets = sheet::sheets(&ydoc, str_field(meta, "dsheetId"))?;
        out["sheets"] = serde_json::to_value(sheets).unwrap();
    } else {
        let tabs = doc::tabs(&ydoc);
        if title.is_empty() {
            out["title"] = Value::String(title_from_tabs(&tabs));
            out["titleFromText"] = Value::Bool(true);
        }
        out["markdown"] = Value::String(doc::markdown(&title, &tabs));
        // The body alone, for "Edit a copy" (the title is edited separately).
        out["markdownBody"] = Value::String(doc::markdown("", &tabs));
        out["tabs"] = serde_json::to_value(tabs).unwrap();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_text() {
        let b = |t: &str, text: &str| {
            let mut b = doc::Block::default();
            (b.t, b.text) = (t.into(), text.into());
            b
        };
        let tab = |blocks| doc::Tab { id: "t".into(), name: "".into(), depth: 0, blocks, slides: vec![] };
        assert_eq!(title_from_tabs(&[tab(vec![b("p", "intro line\nmore"), b("h2", "Real heading")])]), "Real heading");
        assert_eq!(title_from_tabs(&[tab(vec![b("code", "x = 1"), b("p", "  "), b("p", "first words\nsecond")])]), "first words");
        assert_eq!(title_from_tabs(&[tab(vec![b("p", &"a".repeat(100))])]).chars().count(), 80);
        assert_eq!(title_from_tabs(&[]), "");
    }

    #[test]
    fn links() {
        let l = parse_link("https://docs.fileverse.io/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/57#key=abc").unwrap();
        assert_eq!((l.portal.as_str(), l.file_id, l.key.as_str()), ("0x81fb962e2088De6925AffA4E068dd3FAF3EFE163", 57, "abc"));
        assert!(parse_link("https://docs.fileverse.io/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/57").is_err());
        let l = parse_link("https://ddocs.new/d/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/3?x=1#foo=1&key=k2").unwrap();
        assert_eq!((l.file_id, l.key.as_str()), (3, "k2"));
        let l = parse_link("https://docs.fileverse.io/0x2d133a10443a13957278e7dfeefbfee826c82fd8/115#k=raw32").unwrap();
        assert_eq!((l.file_id, l.key.as_str()), (115, "raw32"));
    }
}
