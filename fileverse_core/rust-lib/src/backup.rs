//! "Your own files": a Fileverse backup-key JSON (what dDocs/dSheets export and
//! `fileverse/walk-away` reads) opens every file of the portals it names, with no
//! share links and no Fileverse service.
//!
//! Two lock families exist:
//!   - legacy portals: RSA-OAEP-SHA256 with `ownerPrivateKey` (base64 PKCS#8) opens
//!     `metadata.ownerLock.lockedFileKey`, giving `{key, iv, authTag}`;
//!   - current portals: secp256k1 ECIES with `appDecryptionKey` opens
//!     `ownerLock` (dDocs) or `appLock` (dSheets), giving the raw AES key.
//! Locks starting `FVPQ1` use Fileverse's post-quantum scheme, whose library is
//! not published; those files are reported, not opened.

use rsa::pkcs8::DecodePrivateKey;
use serde::Serialize;
use serde_json::{json, Value};

use crate::chain::{self, Rpc};
use crate::ipfs::Gateways;
use crate::viewer::{self, fetch_json, file_key_of, FileKey};
use crate::{b64, crypto};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortalKeys {
    pub portal: String,
    pub source: String,
    /// "legacy" (RSA) or "ec" (ECIES). Post-quantum-only portals are "pq".
    pub kind: String,
    #[serde(skip)]
    pub rsa_owner: Option<String>,
    #[serde(skip)]
    pub ec_private: Option<String>,
    pub has_pq: bool,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn one(v: &Value) -> Option<PortalKeys> {
    let portal = s(v, "portalAddress");
    if !chain::valid_address(portal) {
        return None;
    }
    let source = s(v, "source").to_string();
    let has_pq = v.get("pqKeys").and_then(Value::as_array).map(|a| !a.is_empty()).unwrap_or(false)
        || !s(v, "pqDecryptionKey").is_empty();
    // Old format: member keys set -> a legacy RSA portal.
    if !s(v, "memberPrivateKey").is_empty() && !s(v, "ownerPrivateKey").is_empty() {
        return Some(PortalKeys {
            portal: portal.into(),
            source,
            kind: "legacy".into(),
            rsa_owner: Some(s(v, "ownerPrivateKey").into()),
            ec_private: None,
            has_pq,
        });
    }
    let ec = if !s(v, "appDecryptionKey").is_empty() {
        Some(s(v, "appDecryptionKey").to_string())
    } else if v.get("memberPrivateKey").is_some() && !s(v, "ownerPrivateKey").is_empty() {
        // Transitional format: owner keys are the app (EC) pair.
        Some(s(v, "ownerPrivateKey").to_string())
    } else {
        None
    };
    if ec.is_none() && !has_pq {
        return None;
    }
    Some(PortalKeys {
        portal: portal.into(),
        source,
        kind: if ec.is_some() { "ec".into() } else { "pq".into() },
        rsa_owner: None,
        ec_private: ec,
        has_pq,
    })
}

/// Every portal a backup file holds keys for (a backup can be one object or a map of them).
pub fn parse(json_text: &str) -> Result<Vec<PortalKeys>, String> {
    let v: Value = serde_json::from_str(json_text).map_err(|e| format!("not a backup-key file (JSON): {e}"))?;
    if let Some(k) = one(&v) {
        return Ok(vec![k]);
    }
    let mut out = vec![];
    if let Value::Object(map) = &v {
        for item in map.values() {
            if let Some(k) = one(item) {
                out.push(k);
            }
        }
    }
    if out.is_empty() {
        return Err("no portal keys found in this file (is it a dDocs/dSheets backup key?)".into());
    }
    Ok(out)
}

fn rsa_open(der_b64: &str, ct_b64: &str) -> Result<Vec<u8>, String> {
    let key = rsa::RsaPrivateKey::from_pkcs8_der(&b64::decode(der_b64)?).map_err(|e| format!("bad RSA key: {e}"))?;
    key.decrypt(rsa::Oaep::new::<sha2::Sha256>(), &b64::decode(ct_b64)?)
        .map_err(|_| "RSA-OAEP decryption failed (key does not match this file)".to_string())
}

/// Unwrap a file's key from its owner/app lock.
pub(crate) fn unwrap(keys: &PortalKeys, meta: &Value) -> Result<FileKey, String> {
    let locks = [meta.get("ownerLock"), meta.get("appLock")];
    let mut pq = false;
    let mut last = String::from("this file has no owner lock");
    for lock in locks.into_iter().flatten() {
        let locked = s(lock, "lockedFileKey");
        if locked.is_empty() {
            continue;
        }
        if locked.starts_with("FVPQ1") {
            pq = true;
            continue;
        }
        let r = match (&keys.rsa_owner, &keys.ec_private) {
            (Some(der), _) => rsa_open(der, locked),
            (None, Some(ec)) => b64::decode(ec).and_then(|k| crypto::ecies_open(&k, locked)),
            _ => Err("no key for this lock".into()),
        };
        match r.and_then(|plain| file_key_of(&plain)) {
            Ok(fk) => return Ok(fk),
            Err(e) => last = e,
        }
    }
    if pq {
        return Err("locked with Fileverse's post-quantum scheme, whose library is not public; open it with a share link instead".into());
    }
    Err(last)
}

/// The share-link secret, when the owner lock carries the link key
/// (`ownerLock.lockedLinkKey`, written by newer dDocs). Older files do not have it.
pub(crate) fn owner_link_secret(keys: &PortalKeys, meta: &Value) -> Option<Vec<u8>> {
    let locked = s(meta.get("ownerLock")?, "lockedLinkKey");
    if locked.is_empty() || locked.starts_with("FVPQ1") {
        return None;
    }
    let plain = match (&keys.rsa_owner, &keys.ec_private) {
        (Some(der), _) => rsa_open(der, locked).ok()?,
        (None, Some(ec)) => crypto::ecies_open(&b64::decode(ec).ok()?, locked).ok()?,
        _ => return None,
    };
    // Legacy files lock the link key's text (as in the URL); current ones its bytes.
    let link_key = std::str::from_utf8(&plain).ok().and_then(|t| b64::decode(t.trim()).ok()).unwrap_or(plain);
    viewer::link_secret_of(&link_key, meta).ok()
}

/// The title of a file opened with the backup key: decrypted when the metadata
/// allows it, else taken from the document's text (`titleFromText`).
fn title_of(gw: &Gateways, keys: &PortalKeys, meta: &Value, fk: &FileKey, content_cid: &str, kind: &str, entry: &mut Value) {
    let secret = owner_link_secret(keys, meta).unwrap_or_default();
    let t = viewer::title_with(meta, fk, &secret);
    if !t.is_empty() || kind != "ddoc" {
        entry["title"] = json!(t);
        return;
    }
    if let Ok(t) = viewer::title_from_content(gw, content_cid, fk) {
        entry["title"] = json!(t);
        entry["titleFromText"] = json!(true);
    }
}

/// How many files are read at once: each costs a chain read and an IPFS fetch,
/// so a portal of a hundred files is minutes serially and seconds this way.
pub const PARALLEL: usize = 6;

/// Run `f` over `ids` on `PARALLEL` threads, keeping the input order.
fn par_map<T: Send>(ids: &[u64], f: impl Fn(u64) -> T + Sync, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<T> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<T>>> = Mutex::new((0..ids.len()).map(|_| None).collect());
    std::thread::scope(|sc| {
        for _ in 0..PARALLEL.min(ids.len().max(1)) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= ids.len() {
                    break;
                }
                let v = f(ids[i]);
                out.lock().unwrap()[i] = Some(v);
                progress(done.fetch_add(1, Ordering::SeqCst) + 1, ids.len());
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|v| v.unwrap()).collect()
}

fn list_one(rpc: &(dyn Rpc + Sync), gw: &Gateways, keys: &PortalKeys, id: u64) -> Option<Value> {
    let mut entry = json!({ "portal": keys.portal, "fileId": id });
    let rec = match chain::read_file(rpc, &keys.portal, id) {
        Ok(r) => r,
        // deleted slots read back empty
        Err(e) if e.contains("does not exist") => return None,
        Err(e) => {
            entry["error"] = json!(format!("could not read it on Gnosis: {e}"));
            return Some(entry);
        }
    };
    if rec.metadata_cid.is_empty() {
        return None;
    }
    entry["layout"] = json!(rec.layout);
    entry["cids"] = json!([rec.metadata_cid, rec.content_cid, rec.gate_cid]);
    let mut prov = vec![];
    match fetch_json(gw, &rec.metadata_cid, &mut prov) {
        Ok(meta) => {
            if meta.get("isDeleted").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let kind = if s(&meta, "kind") == "dsheet" || meta.get("dsheetId").is_some() { "dsheet" } else { "ddoc" };
            entry["kind"] = json!(kind);
            entry["updatedAt"] = meta.get("contentUpdatedAt").or_else(|| meta.get("last_updated")).cloned().unwrap_or(Value::Null);
            match unwrap(keys, &meta) {
                Ok(fk) => title_of(gw, keys, &meta, &fk, &rec.content_cid, kind, &mut entry),
                Err(e) => entry["error"] = json!(e),
            }
        }
        Err(e) => entry["error"] = json!(e),
    }
    Some(entry)
}

/// The portal's files with their titles, newest first. Unreadable entries are
/// listed with the reason instead of being dropped.
pub fn list(rpc: &(dyn Rpc + Sync), gw: &Gateways, keys: &PortalKeys, max: u64) -> Result<Vec<Value>, String> {
    list_with_progress(rpc, gw, keys, max, &|_, _| {})
}

pub fn list_with_progress(
    rpc: &(dyn Rpc + Sync),
    gw: &Gateways,
    keys: &PortalKeys,
    max: u64,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Value>, String> {
    let n = chain::file_count(rpc, &keys.portal)?;
    let ids: Vec<u64> = (0..n).rev().take(max as usize).collect();
    Ok(par_map(&ids, |id| list_one(rpc, gw, keys, id), progress).into_iter().flatten().collect())
}

/// Fetch every block of the given files (metadata, content, gate) through `gw`,
/// which keeps each verified block in its cache. Returns, per file, its root
/// CIDs and every block CID, or why it could not be fetched.
pub fn fetch_files(gw: &Gateways, files: &[Value], progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<Value> {
    let idx: Vec<u64> = (0..files.len() as u64).collect();
    par_map(
        &idx,
        |i| {
            let f = &files[i as usize];
            let roots: Vec<String> = f["cids"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| c.as_str()).filter(|c| !c.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
            let mut blocks = vec![];
            for r in &roots {
                match gw.get(r) {
                    Ok((_, p)) => blocks.extend(p.blocks),
                    Err(e) => return json!({ "fileId": f["fileId"], "error": e }),
                }
            }
            json!({ "fileId": f["fileId"], "roots": roots, "blocks": blocks })
        },
        progress,
    )
}

/// Open one of the portal's files with the backup key.
pub fn open(rpc: &dyn Rpc, gw: &Gateways, keys: &PortalKeys, file_id: u64) -> Result<Value, String> {
    let rec = chain::read_file(rpc, &keys.portal, file_id)?;
    let mut steps = vec![format!("read files({file_id}) on {} ({:?} portal)", rpc.describe(), rec.layout)];
    let mut prov = vec![];
    let meta = fetch_json(gw, &rec.metadata_cid, &mut prov)?;
    if meta.get("isDeleted").and_then(Value::as_bool) == Some(true) {
        return Err("this file was deleted".into());
    }
    let fk = unwrap(keys, &meta)?;
    steps.push(format!("file key unwrapped with the backup key ({})", if keys.rsa_owner.is_some() { "RSA-OAEP" } else { "ECIES" }));
    let secret = owner_link_secret(keys, &meta).unwrap_or_default();
    viewer::finish(gw, &rec, &meta, &fk, &secret, prov, steps, rpc.describe())
}
