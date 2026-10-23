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
use crate::viewer::{self, fetch_json, file_key_of, str_field, FileKey};
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

/// The portal's files with their titles, newest first. Unreadable entries are
/// listed with the reason instead of being dropped.
pub fn list(rpc: &dyn Rpc, gw: &Gateways, keys: &PortalKeys, max: u64) -> Result<Vec<Value>, String> {
    let n = chain::file_count(rpc, &keys.portal)?;
    let mut out = vec![];
    for id in (0..n).rev().take(max as usize) {
        let mut entry = json!({ "portal": keys.portal, "fileId": id });
        let rec = match chain::read_file(rpc, &keys.portal, id) {
            Ok(r) => r,
            Err(_) => continue, // deleted slots read back empty
        };
        if rec.metadata_cid.is_empty() {
            continue;
        }
        let mut prov = vec![];
        match fetch_json(gw, &rec.metadata_cid, &mut prov) {
            Ok(meta) => {
                if meta.get("isDeleted").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let kind = if s(&meta, "kind") == "dsheet" || meta.get("dsheetId").is_some() { "dsheet" } else { "ddoc" };
                entry["kind"] = json!(kind);
                entry["updatedAt"] = meta.get("contentUpdatedAt").or_else(|| meta.get("last_updated")).cloned().unwrap_or(Value::Null);
                match unwrap(keys, &meta) {
                    Ok(fk) => entry["title"] = json!(viewer::title_with(&meta, &fk)),
                    Err(e) => entry["error"] = json!(e),
                }
            }
            Err(e) => entry["error"] = json!(e),
        }
        out.push(entry);
    }
    Ok(out)
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
    let _ = str_field(&meta, "title");
    viewer::finish(gw, &rec, &meta, &fk, &[], prov, steps, rpc.describe())
}
