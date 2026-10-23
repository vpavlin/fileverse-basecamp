//! Logos module glue for `fileverse_core` (rust-first authoring).
//!
//! The builder derives the `.lidl` from the `FileverseCoreModule` trait below
//! (`codegen.rust = { trait, source: "src/glue.rs" }`). Every method returns a
//! JSON string, `{ "ok": true, ... }` or `{ "ok": false, "error": "<sentence>" }`,
//! and never panics across the boundary.
//!
//! Anything that touches the network (opening, listing a portal, publishing,
//! copying to Logos Storage) runs as a job: the method returns `{ ok, jobId }`
//! at once, the view polls `job(id)` and also hears `job_changed`.
//!
//! Where things come from:
//!   - Gnosis: eth_rpc_module (the device-wide RPC registry; chain 100 is seeded
//!     there only where absent);
//!   - IPFS: the user's own node first (ipfs_module when installed, else the node
//!     set in Settings), then gateways; every block is hash-checked;
//!   - Logos Storage (storage_module, host-owned on Basecamp 0.3): blocks copied
//!     there are read back from it when IPFS has nothing, hash-checked the same way.
//!
//! Documents are addressed by URL everywhere: a share link, `fvown://<portal>/<id>`
//! for a file opened with a backup key, or `fvbc://ipfs/<cid>#key=` for a draft
//! published from here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
// `Mutex` comes from the generated scaffold (provider_gen.rs imports std::sync::Mutex).
use std::time::{Duration, Instant};

use logos_rust_sdk::LogosError;
use serde_json::{json, Value};

use crate::backup::{self, PortalKeys};
use crate::chain::{Rpc, GNOSIS_CHAIN_ID};
use crate::ipfs::{BlockSource, Cid, Gateways, DEFAULT_GATEWAYS};
use crate::node::{self, Node};
use crate::{b64, doc, md, publish, viewer};

pub trait FileverseCoreModule: Send + Sync + 'static {
    /// Start opening a document by URL (share link, fvown://, fvbc://). Returns `{ ok, jobId }`.
    fn open(&self, url: String) -> String;
    /// A job's state: `pending`, `done` (with `result`) or `failed` (with `error`).
    fn job(&self, job_id: String) -> String;
    /// Settings, recent documents, portals and drafts, for the view to poll.
    fn snapshot(&self) -> String;
    /// Replace the IPFS gateway list (a JSON array of base URLs). An empty array restores the defaults.
    fn set_gateways(&self, gateways_json: String) -> String;
    /// Remove a document from the recent list.
    fn forget(&self, url: String) -> String;
    /// A manually configured IPFS node (Kubo RPC API and gateway), used when ipfs_module is not installed.
    fn set_ipfs_node(&self, api_url: String, gateway_url: String) -> String;
    /// Which IPFS node is in use and whether it answers: `{ reachable, source, api, gateway, peerId }`.
    fn node_status(&self) -> String;
    /// Copy a recent document's blocks into your IPFS node and pin them. Returns a job.
    fn keep(&self, url: String) -> String;
    /// Whether Logos Storage is available: `{ available, peerId }`.
    fn storage_status(&self) -> String;
    /// Copy a recent document's blocks into Logos Storage. Returns a job.
    fn copy_to_storage(&self, url: String) -> String;
    /// Store the keys of a dDocs/dSheets backup-key file: its JSON text, or a path / file:// URL to it.
    fn import_backup(&self, backup_json: String) -> String;
    /// Delete the stored keys of one portal.
    fn forget_portal(&self, portal: String) -> String;
    /// List a portal's files (titles decrypted with its backup key). Returns a job.
    fn list_portal(&self, portal: String) -> String;
    /// Render Markdown the way the document view and the slide view will show it.
    fn preview(&self, markdown: String) -> String;
    /// Create a draft. `source_url` names the document it was copied from (may be empty).
    fn draft_create(&self, title: String, markdown: String, source_url: String) -> String;
    /// Save a draft's title and Markdown. Returns it with a fresh preview.
    fn draft_save(&self, draft_id: String, title: String, markdown: String) -> String;
    /// One draft with its Markdown and preview.
    fn draft_get(&self, draft_id: String) -> String;
    /// Delete a draft (published copies stay where they are).
    fn draft_delete(&self, draft_id: String) -> String;
    /// Encrypt a draft and publish it to your IPFS node and Logos Storage. Returns a job.
    fn publish(&self, draft_id: String) -> String;

    fn on_context_ready(&self, _ctx: &RustModuleContext) {}
}

pub trait FileverseCoreModuleEvents {
    /// A job moved: re-read it with `job(id)`.
    fn job_changed(&self, job_id: String);
}

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

const RPC_TIMEOUT: Duration = Duration::from_secs(25);
const CONFIG_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RECENT: usize = 30;
const NODE_PROBE: Duration = Duration::from_millis(800);
/// ipfs_module starts its node on first use; that can take a while.
const NODE_START: Duration = Duration::from_secs(60);
/// A dependency that is not installed fails its call only after this; remember it.
const ABSENT_FOR: Duration = Duration::from_secs(60);
const STORAGE_CALL: Duration = Duration::from_secs(30);
const STORAGE_WAIT: Duration = Duration::from_secs(90);
const MAX_PORTAL_FILES: u64 = 200;

struct ModuleRpc;

impl Rpc for ModuleRpc {
    fn eth_call(&self, to: &str, data: &str) -> Result<String, String> {
        let params = json!([{ "to": to, "data": data }, "latest"]).to_string();
        let raw = modules()
            .eth_rpc_module
            .raw_rpc_with_timeout(GNOSIS_CHAIN_ID as i64, "eth_call", &params, RPC_TIMEOUT)
            .map_err(|e| format!("eth_rpc_module did not answer: {e}"))?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| format!("eth_rpc_module: {e}"))?;
        if v.get("ok").and_then(Value::as_bool) != Some(true) {
            let why = v.get("error").and_then(Value::as_str).unwrap_or("eth_call failed");
            return Err(format!("Gnosis RPC: {why}"));
        }
        v.get("result")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "Gnosis RPC returned no result".into())
    }

    fn describe(&self) -> String {
        "eth_rpc_module (chain 100)".into()
    }
}

/// `{success, value, error}` from a `result`-typed call.
fn unwrap_result(v: Result<Value, LogosError>, what: &str) -> Result<Value, String> {
    let v = v.map_err(|e| format!("{what}: {e}"))?;
    if v.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(v.get("value").cloned().unwrap_or(Value::Null));
    }
    Err(format!("{what}: {}", v.get("error").and_then(Value::as_str).unwrap_or("failed")))
}

// ── Logos Storage ─────────────────────────────────────────────────────────────

/// IPFS block CID (v1) -> where its copy lives in Logos Storage.
type StorageMap = HashMap<String, (String, u64)>;

struct StorageSource {
    map: StorageMap,
    tmp: PathBuf,
}

impl BlockSource for StorageSource {
    fn name(&self) -> String {
        "Logos Storage".into()
    }

    fn block(&self, cid: &Cid) -> Option<Vec<u8>> {
        let (ls_cid, size) = self.map.get(&cid.to_v1())?.clone();
        let _ = std::fs::create_dir_all(&self.tmp);
        let path = self.tmp.join(format!("dl-{}", cid.to_v1()));
        let _ = std::fs::remove_file(&path);
        let client = storage_module::StorageModuleClient::new();
        unwrap_result(
            client.download_to_url_with_timeout(&ls_cid, &path.to_string_lossy(), false, 65536, false, true, STORAGE_CALL),
            "Logos Storage download",
        )
        .ok()?;
        let t0 = Instant::now();
        while t0.elapsed() < STORAGE_WAIT {
            if std::fs::metadata(&path).map(|m| m.len() == size).unwrap_or(false) {
                let b = std::fs::read(&path).ok();
                let _ = std::fs::remove_file(&path);
                return b;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        None
    }
}

/// Upload blocks to Logos Storage, one file per block named by its IPFS CID, and
/// learn each one's Storage CID from `manifests()` once its upload completes.
fn copy_blocks_to_storage(gw: &Gateways, blocks: &[String], known: &StorageMap, tmp: &Path) -> Result<StorageMap, String> {
    let client = storage_module::StorageModuleClient::new();
    std::fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    let mut added = StorageMap::new();
    let mut waiting = vec![];
    for b in blocks {
        if known.contains_key(b) {
            continue;
        }
        let bytes = gw.block_bytes(b)?;
        let path = tmp.join(b);
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        unwrap_result(client.upload_url_with_timeout(&path.to_string_lossy(), 65536, true, STORAGE_CALL), "Logos Storage upload")?;
        waiting.push((b.clone(), bytes.len() as u64));
    }
    let t0 = Instant::now();
    while !waiting.is_empty() {
        if t0.elapsed() > STORAGE_WAIT {
            return Err(format!("Logos Storage did not finish {} upload(s) in time", waiting.len()));
        }
        std::thread::sleep(Duration::from_millis(500));
        let list = unwrap_result(client.manifests_with_timeout(STORAGE_CALL), "Logos Storage manifests")?;
        let list = list.as_array().cloned().unwrap_or_default();
        waiting.retain(|(name, size)| {
            match list.iter().find(|m| m["filename"].as_str() == Some(name.as_str()) && m["datasetSize"].as_u64() == Some(*size)) {
                Some(m) => {
                    added.insert(name.clone(), (m["cid"].as_str().unwrap_or("").to_string(), *size));
                    let _ = std::fs::remove_file(tmp.join(name));
                    false
                }
                None => true,
            }
        });
    }
    Ok(added)
}

// ── state ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Job {
    state: &'static str,
    result: Value,
    error: String,
    started: u64,
}

#[derive(Default)]
struct State {
    dir: Option<PathBuf>,
    gateways: Vec<String>,
    node_api: String,
    node_gateway: String,
    recent: Vec<Value>,
    storage: StorageMap,
    keys: Vec<PortalKeys>,
    jobs: HashMap<String, Job>,
    seq: u64,
    chain_seeded: bool,
    ipfs_module_absent_until: Option<Instant>,
}

#[derive(Default)]
struct FileverseCoreModuleImpl {
    state: Arc<Mutex<State>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn err(e: impl std::fmt::Display) -> String {
    json!({ "ok": false, "error": e.to_string() }).to_string()
}

fn write_private(path: &Path, text: &str) {
    let _ = std::fs::write(path, text);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

impl State {
    fn path(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name))
    }

    fn save(&self) {
        if let Some(p) = self.path("settings.json") {
            let storage: HashMap<&String, Value> = self.storage.iter().map(|(k, (c, n))| (k, json!([c, n]))).collect();
            let v = json!({
                "gateways": self.gateways, "recent": self.recent, "nodeApi": self.node_api,
                "nodeGateway": self.node_gateway, "storage": storage,
            });
            // Recent entries hold share-link keys: private to this user.
            write_private(&p, &v.to_string());
        }
    }

    fn save_keys(&self) {
        if let Some(p) = self.path("backup-keys.json") {
            let v: Vec<Value> = self
                .keys
                .iter()
                .map(|k| json!({ "portal": k.portal, "source": k.source, "kind": k.kind, "hasPq": k.has_pq, "rsa": k.rsa_owner, "ec": k.ec_private }))
                .collect();
            write_private(&p, &Value::Array(v).to_string());
        }
    }

    fn load(&mut self, dir: &Path) {
        if let Ok(b) = std::fs::read(dir.join("settings.json")) {
            if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                self.gateways = v["gateways"].as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect()).unwrap_or_default();
                self.recent = v["recent"].as_array().cloned().unwrap_or_default();
                self.node_api = v["nodeApi"].as_str().unwrap_or("").to_string();
                self.node_gateway = v["nodeGateway"].as_str().unwrap_or("").to_string();
                if let Some(m) = v["storage"].as_object() {
                    for (k, e) in m {
                        if let (Some(c), Some(n)) = (e[0].as_str(), e[1].as_u64()) {
                            self.storage.insert(k.clone(), (c.to_string(), n));
                        }
                    }
                }
            }
        }
        if let Ok(b) = std::fs::read(dir.join("backup-keys.json")) {
            if let Ok(Value::Array(a)) = serde_json::from_slice::<Value>(&b) {
                self.keys = a
                    .iter()
                    .map(|k| PortalKeys {
                        portal: k["portal"].as_str().unwrap_or("").into(),
                        source: k["source"].as_str().unwrap_or("").into(),
                        kind: k["kind"].as_str().unwrap_or("").into(),
                        has_pq: k["hasPq"].as_bool().unwrap_or(false),
                        rsa_owner: k["rsa"].as_str().map(str::to_string),
                        ec_private: k["ec"].as_str().map(str::to_string),
                    })
                    .collect();
            }
        }
    }

    fn gateway_list(&self) -> Vec<String> {
        if self.gateways.is_empty() {
            DEFAULT_GATEWAYS.iter().map(|s| s.to_string()).collect()
        } else {
            self.gateways.clone()
        }
    }

    fn drafts_dir(&self) -> Option<PathBuf> {
        self.path("drafts")
    }
}

/// Gnosis is not one of eth_rpc_module's built-in chains. `ensure_chain_config`
/// writes only absent fields, so a user's own endpoint is never replaced.
fn seed_gnosis() -> Result<(), String> {
    let cfg = json!({
        "endpoint": "https://rpc.gnosischain.com",
        "name": "Gnosis",
        "nativeSymbol": "xDAI",
        "nativeDecimals": 18,
        "testnet": false,
        "enabled": true,
    })
    .to_string();
    modules()
        .eth_rpc_module
        .ensure_chain_config_with_timeout(GNOSIS_CHAIN_ID as i64, &cfg, CONFIG_TIMEOUT)
        .map(|_| ())
        .map_err(|e| format!("could not configure Gnosis in eth_rpc_module: {e}"))
}

#[derive(Clone, Default)]
struct NodeInfo {
    source: String,
    api: String,
    gateway: String,
    peer_id: String,
}

impl FileverseCoreModuleImpl {
    /// The IPFS node to use: ipfs_module's (started on demand) when installed,
    /// else the one configured in Settings, if it answers.
    fn node(&self, probe: Duration) -> Option<NodeInfo> {
        let try_module = {
            let s = self.state.lock().unwrap();
            s.ipfs_module_absent_until.map(|t| Instant::now() > t).unwrap_or(true)
        };
        if try_module {
            match ipfs_module::IpfsModuleClient::new().endpoints_with_timeout(NODE_START) {
                Ok(raw) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                        if v["ok"] == true {
                            return Some(NodeInfo {
                                source: "ipfs_module".into(),
                                api: v["api"].as_str().unwrap_or("").into(),
                                gateway: v["gateway"].as_str().unwrap_or("").into(),
                                peer_id: v["peerId"].as_str().unwrap_or("").into(),
                            });
                        }
                    }
                }
                Err(_) => self.state.lock().unwrap().ipfs_module_absent_until = Some(Instant::now() + ABSENT_FOR),
            }
        }
        let (api, gw) = {
            let s = self.state.lock().unwrap();
            (
                if s.node_api.is_empty() { node::DEFAULT_API.to_string() } else { s.node_api.clone() },
                if s.node_gateway.is_empty() { node::DEFAULT_GATEWAY.to_string() } else { s.node_gateway.clone() },
            )
        };
        Node::new(&api, probe).id().ok().map(|id| NodeInfo {
            source: "settings".into(),
            api,
            gateway: gw,
            peer_id: id["peerId"].as_str().unwrap_or("").into(),
        })
    }

    /// Readers for this request: your node first, then gateways, then Logos Storage.
    fn gateways(&self, node: Option<&NodeInfo>) -> Gateways {
        let s = self.state.lock().unwrap();
        let mut list = s.gateway_list();
        if let Some(n) = node {
            if !n.gateway.is_empty() && !list.contains(&n.gateway) {
                list.insert(0, n.gateway.clone());
            }
        }
        let mut gw = Gateways::new(list, s.path("blocks"));
        if !s.storage.is_empty() {
            gw.extra.push(Box::new(StorageSource { map: s.storage.clone(), tmp: s.path("tmp").unwrap_or_else(std::env::temp_dir) }));
        }
        gw
    }

    fn new_job(&self) -> String {
        let mut s = self.state.lock().unwrap();
        s.seq += 1;
        let id = format!("j{}", s.seq);
        s.jobs.insert(id.clone(), Job { state: "pending", result: Value::Null, error: String::new(), started: now_ms() });
        id
    }

    /// Run `work` on its own thread; `done` updates state with the result.
    fn run_job<W, D>(&self, work: W, done: D) -> String
    where
        W: FnOnce(&FileverseCoreModuleImpl) -> Result<Value, String> + Send + 'static,
        D: FnOnce(&mut State, &Value) + Send + 'static,
    {
        let id = self.new_job();
        let me = FileverseCoreModuleImpl { state: self.state.clone() };
        let job_id = id.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&me)))
                .unwrap_or_else(|_| Err("internal error while running this job".into()));
            let mut s = me.state.lock().unwrap();
            if let Ok(v) = &r {
                done(&mut s, v);
            }
            let job = s.jobs.get_mut(&job_id).unwrap();
            match r {
                Ok(v) => {
                    job.state = "done";
                    job.result = v;
                }
                Err(e) => {
                    job.state = "failed";
                    job.error = e;
                }
            }
            drop(s);
            emit_job_changed(&job_id);
        });
        json!({ "ok": true, "jobId": id }).to_string()
    }

    fn seed_chain_once(&self) {
        let seed = !self.state.lock().unwrap().chain_seeded;
        if seed {
            match seed_gnosis() {
                Ok(()) => self.state.lock().unwrap().chain_seeded = true,
                Err(e) => eprintln!("[fileverse_core] {e}"),
            }
        }
    }

    fn recent_blocks(&self, url: &str) -> Result<(Vec<String>, Vec<String>), String> {
        let s = self.state.lock().unwrap();
        let entry = s.recent.iter().find(|r| r["url"].as_str() == Some(url)).cloned().ok_or("open the document first")?;
        let strs = |k: &str| -> Vec<String> {
            entry[k].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        let (roots, blocks) = (strs("roots"), strs("blocks"));
        if blocks.is_empty() {
            return Err("reopen the document once with this version first".into());
        }
        Ok((roots, blocks))
    }

    fn load_draft(&self, id: &str) -> Result<(PathBuf, Value), String> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("bad draft id".into());
        }
        let dir = self.state.lock().unwrap().drafts_dir().ok_or("module not ready")?;
        let p = dir.join(format!("{id}.json"));
        let v: Value = serde_json::from_slice(&std::fs::read(&p).map_err(|_| "no such draft")?).map_err(|e| e.to_string())?;
        Ok((p, v))
    }
}

fn record_recent(s: &mut State, url: &str, v: &Value) {
    let blobs = v["provenance"]["blobs"].as_array().cloned().unwrap_or_default();
    let roots: Vec<Value> = blobs.iter().map(|b| b["cid"].clone()).collect();
    let blocks: Vec<Value> = blobs.iter().flat_map(|b| b["blocks"].as_array().cloned().unwrap_or_default()).collect();
    let prev = s.recent.iter().find(|r| r["url"].as_str() == Some(url)).cloned().unwrap_or(Value::Null);
    // A kept/stored copy is still that document only while it has the same CIDs
    // (compared as sets: publish and open list the same blocks in different orders).
    let set = |v: &Value| -> Vec<String> {
        let mut x: Vec<String> = v.as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
        x.sort();
        x
    };
    let same = !roots.is_empty() && set(&prev["roots"]) == set(&json!(roots));
    let entry = json!({
        "url": url, "portal": v["portal"], "fileId": v["fileId"], "layout": v["layout"],
        "title": v["title"], "kind": v["kind"], "openedAt": now_ms(),
        "roots": roots, "blocks": blocks,
        "kept": if same { prev["kept"].clone() } else { Value::Null },
        "keptOn": if same { prev["keptOn"].clone() } else { Value::Null },
        "inStorage": if same { prev["inStorage"].clone() } else { Value::Null },
    });
    s.recent.retain(|r| r["url"].as_str() != Some(url));
    s.recent.insert(0, entry);
    s.recent.truncate(MAX_RECENT);
    s.save();
}

fn preview_json(markdown: &str) -> Value {
    match doc::load(&md::to_yjs(markdown)) {
        Ok(d) => json!(doc::tabs(&d)),
        Err(e) => json!({ "error": e }),
    }
}

fn draft_summary(d: &Value) -> Value {
    json!({
        "id": d["id"], "title": d["title"], "updatedAt": d["updatedAt"], "sourceUrl": d["sourceUrl"],
        "link": d["published"]["link"], "publishedAt": d["published"]["at"],
    })
}

fn new_id() -> String {
    hex::encode(crate::crypto::random::<8>())
}

impl FileverseCoreModule for FileverseCoreModuleImpl {
    fn on_context_ready(&self, ctx: &RustModuleContext) {
        // Local state only: the 0.3 runtime refuses calls to other modules from here.
        let dir = PathBuf::from(&ctx.instance_persistence_path);
        let _ = std::fs::create_dir_all(dir.join("drafts"));
        let mut s = self.state.lock().unwrap();
        s.dir = Some(dir.clone());
        s.load(&dir);
    }

    fn open(&self, url: String) -> String {
        let url = url.trim().to_string();
        let own = url.strip_prefix("fvown://").map(|r| r.to_string());
        if own.is_none() && !url.starts_with(publish::LINK_PREFIX) {
            if let Err(e) = viewer::parse_link(&url) {
                return err(e);
            }
        }
        let u2 = url.clone();
        self.run_job(
            move |me| {
                me.seed_chain_once();
                let node = me.node(NODE_PROBE);
                let gw = me.gateways(node.as_ref());
                match own {
                    Some(rest) => {
                        let (portal, id) = rest.split_once('/').ok_or("bad fvown:// link")?;
                        let id: u64 = id.parse().map_err(|_| "bad file number")?;
                        let keys = {
                            let s = me.state.lock().unwrap();
                            s.keys.iter().find(|k| k.portal.eq_ignore_ascii_case(portal)).cloned()
                        }
                        .ok_or("no backup key stored for this portal")?;
                        backup::open(&ModuleRpc, &gw, &keys, id)
                    }
                    None => viewer::open_link(&ModuleRpc, &gw, &url),
                }
            },
            move |s, v| record_recent(s, &u2, v),
        )
    }

    fn job(&self, job_id: String) -> String {
        let s = self.state.lock().unwrap();
        match s.jobs.get(&job_id) {
            None => err("no such job (the core was restarted?)"),
            Some(j) => json!({
                "ok": true, "jobId": job_id, "state": j.state, "error": j.error,
                "elapsedMs": now_ms().saturating_sub(j.started), "result": j.result,
            })
            .to_string(),
        }
    }

    fn snapshot(&self) -> String {
        let s = self.state.lock().unwrap();
        let portals: Vec<Value> = s.keys.iter().map(|k| json!(k)).collect();
        let drafts: Vec<Value> = s
            .drafts_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|it| {
                let mut v: Vec<Value> = it
                    .filter_map(|e| e.ok())
                    .filter_map(|e| std::fs::read(e.path()).ok())
                    .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
                    .map(|d| draft_summary(&d))
                    .collect();
                v.sort_by(|a, b| b["updatedAt"].as_u64().cmp(&a["updatedAt"].as_u64()));
                v
            })
            .unwrap_or_default();
        // Recent entries carry their link keys; the view gets them so it can reopen,
        // and they never leave this device.
        json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "gateways": s.gateway_list(),
            "customGateways": !s.gateways.is_empty(),
            "rpc": "eth_rpc_module (chain 100)",
            "node": { "api": s.node_api, "gateway": s.node_gateway },
            "recent": s.recent,
            "portals": portals,
            "drafts": drafts,
            "storedBlocks": s.storage.len(),
            "pending": s.jobs.values().filter(|j| j.state == "pending").count(),
        })
        .to_string()
    }

    fn set_gateways(&self, gateways_json: String) -> String {
        let list: Vec<String> = match serde_json::from_str::<Vec<String>>(&gateways_json) {
            Ok(l) => l.into_iter().map(|g| g.trim().trim_end_matches('/').to_string()).filter(|g| !g.is_empty()).collect(),
            Err(_) => return err("gateways must be a JSON array of URLs"),
        };
        if let Some(bad) = list.iter().find(|g| !(g.starts_with("https://") || g.starts_with("http://"))) {
            return err(format!("not an http(s) URL: {bad}"));
        }
        let mut s = self.state.lock().unwrap();
        s.gateways = list;
        s.save();
        json!({ "ok": true, "gateways": s.gateway_list() }).to_string()
    }

    fn forget(&self, url: String) -> String {
        let mut s = self.state.lock().unwrap();
        let before = s.recent.len();
        s.recent.retain(|r| r["url"].as_str() != Some(url.as_str()));
        s.save();
        json!({ "ok": true, "removed": before - s.recent.len() }).to_string()
    }

    fn set_ipfs_node(&self, api_url: String, gateway_url: String) -> String {
        let (api, gw) = (api_url.trim().trim_end_matches('/').to_string(), gateway_url.trim().trim_end_matches('/').to_string());
        for u in [&api, &gw] {
            if !u.is_empty() && !(u.starts_with("http://") || u.starts_with("https://")) {
                return err(format!("not an http(s) URL: {u}"));
            }
        }
        let mut s = self.state.lock().unwrap();
        s.node_api = api;
        s.node_gateway = gw;
        s.save();
        json!({ "ok": true, "api": s.node_api, "gateway": s.node_gateway }).to_string()
    }

    fn node_status(&self) -> String {
        match self.node(Duration::from_secs(2)) {
            Some(n) => json!({ "ok": true, "reachable": true, "source": n.source, "api": n.api, "gateway": n.gateway, "peerId": n.peer_id }),
            None => json!({ "ok": true, "reachable": false, "error": "no IPFS node: install the IPFS Node module, or run Kubo and set it in Settings" }),
        }
        .to_string()
    }

    fn keep(&self, url: String) -> String {
        let (roots, blocks) = match self.recent_blocks(&url) {
            Ok(x) => x,
            Err(e) => return err(e),
        };
        let u2 = url.clone();
        self.run_job(
            move |me| {
                let n = me.node(Duration::from_secs(2)).ok_or("no IPFS node is available")?;
                let gw = me.gateways(Some(&n));
                let count = Node::new(&n.api, Duration::from_secs(60)).keep(&gw, &roots, &blocks)?;
                Ok(json!({ "ok": true, "blocks": count, "pinned": roots.len(), "node": n.source, "peerId": n.peer_id, "roots": roots }))
            },
            move |s, v| {
                if let Some(r) = s.recent.iter_mut().find(|r| r["url"].as_str() == Some(u2.as_str())) {
                    r["kept"] = v["roots"].clone();
                    r["keptOn"] = v["peerId"].clone();
                    r["keptAt"] = json!(now_ms());
                }
                s.save();
            },
        )
    }

    fn storage_status(&self) -> String {
        let client = storage_module::StorageModuleClient::new();
        match unwrap_result(client.peer_id_with_timeout(Duration::from_secs(3)), "Logos Storage") {
            Ok(id) => json!({ "ok": true, "available": true, "peerId": id }),
            Err(e) => json!({ "ok": true, "available": false, "error": e }),
        }
        .to_string()
    }

    fn copy_to_storage(&self, url: String) -> String {
        let (roots, blocks) = match self.recent_blocks(&url) {
            Ok(x) => x,
            Err(e) => return err(e),
        };
        let u2 = url.clone();
        self.run_job(
            move |me| {
                let (known, tmp) = {
                    let s = me.state.lock().unwrap();
                    (s.storage.clone(), s.path("tmp").unwrap_or_else(std::env::temp_dir))
                };
                let gw = me.gateways(None);
                let added = copy_blocks_to_storage(&gw, &blocks, &known, &tmp)?;
                let map: HashMap<&String, &String> = added.iter().map(|(k, (c, _))| (k, c)).collect();
                Ok(json!({ "ok": true, "copied": added.len(), "alreadyThere": blocks.len() - added.len(), "roots": roots, "added": map }))
            },
            move |s, v| {
                if let Some(m) = v["added"].as_object() {
                    for (k, c) in m {
                        // sizes come from the block cache, which holds every copied block
                        let size = s.path("blocks").and_then(|d| std::fs::metadata(d.join(k)).ok()).map(|m| m.len()).unwrap_or(0);
                        s.storage.insert(k.clone(), (c.as_str().unwrap_or("").to_string(), size));
                    }
                }
                if let Some(r) = s.recent.iter_mut().find(|r| r["url"].as_str() == Some(u2.as_str())) {
                    r["inStorage"] = v["roots"].clone();
                }
                s.save();
            },
        )
    }

    fn import_backup(&self, backup_json: String) -> String {
        let t = backup_json.trim();
        let text = if t.starts_with('{') {
            t.to_string()
        } else {
            let path = t.strip_prefix("file://").unwrap_or(t);
            match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => return err(format!("could not read {path}: {e}")),
            }
        };
        let keys = match backup::parse(&text) {
            Ok(k) => k,
            Err(e) => return err(e),
        };
        let mut s = self.state.lock().unwrap();
        for k in &keys {
            s.keys.retain(|x| !x.portal.eq_ignore_ascii_case(&k.portal));
            s.keys.push(k.clone());
        }
        s.save_keys();
        json!({ "ok": true, "portals": keys }).to_string()
    }

    fn forget_portal(&self, portal: String) -> String {
        let mut s = self.state.lock().unwrap();
        let before = s.keys.len();
        s.keys.retain(|k| !k.portal.eq_ignore_ascii_case(&portal));
        s.save_keys();
        json!({ "ok": true, "removed": before - s.keys.len() }).to_string()
    }

    fn list_portal(&self, portal: String) -> String {
        let keys = {
            let s = self.state.lock().unwrap();
            s.keys.iter().find(|k| k.portal.eq_ignore_ascii_case(&portal)).cloned()
        };
        let Some(keys) = keys else { return err("no backup key stored for this portal") };
        self.run_job(
            move |me| {
                me.seed_chain_once();
                let node = me.node(NODE_PROBE);
                let gw = me.gateways(node.as_ref());
                let files = backup::list(&ModuleRpc, &gw, &keys, MAX_PORTAL_FILES)?;
                let files: Vec<Value> = files
                    .into_iter()
                    .map(|mut f| {
                        f["url"] = json!(format!("fvown://{}/{}", keys.portal, f["fileId"]));
                        f
                    })
                    .collect();
                Ok(json!({ "ok": true, "portal": keys.portal, "files": files }))
            },
            |_, _| {},
        )
    }

    fn preview(&self, markdown: String) -> String {
        json!({ "ok": true, "tabs": preview_json(&markdown) }).to_string()
    }

    fn draft_create(&self, title: String, markdown: String, source_url: String) -> String {
        let dir = match self.state.lock().unwrap().drafts_dir() {
            Some(d) => d,
            None => return err("module not ready"),
        };
        let id = new_id();
        let d = json!({
            "id": id, "title": if title.trim().is_empty() { "Untitled".to_string() } else { title },
            "markdown": markdown, "sourceUrl": source_url, "createdAt": now_ms(), "updatedAt": now_ms(),
            "secret": b64::encode(&crate::crypto::random::<32>()),
        });
        write_private(&dir.join(format!("{id}.json")), &d.to_string());
        let mut out = d.clone();
        out.as_object_mut().unwrap().remove("secret");
        out["preview"] = preview_json(d["markdown"].as_str().unwrap_or(""));
        json!({ "ok": true, "draft": out }).to_string()
    }

    fn draft_save(&self, draft_id: String, title: String, markdown: String) -> String {
        let (p, mut d) = match self.load_draft(&draft_id) {
            Ok(x) => x,
            Err(e) => return err(e),
        };
        d["title"] = json!(if title.trim().is_empty() { "Untitled".to_string() } else { title });
        d["markdown"] = json!(markdown);
        d["updatedAt"] = json!(now_ms());
        write_private(&p, &d.to_string());
        let mut out = d.clone();
        out.as_object_mut().unwrap().remove("secret");
        out["preview"] = preview_json(d["markdown"].as_str().unwrap_or(""));
        json!({ "ok": true, "draft": out }).to_string()
    }

    fn draft_get(&self, draft_id: String) -> String {
        match self.load_draft(&draft_id) {
            Ok((_, mut d)) => {
                d.as_object_mut().unwrap().remove("secret");
                d["preview"] = preview_json(d["markdown"].as_str().unwrap_or(""));
                json!({ "ok": true, "draft": d }).to_string()
            }
            Err(e) => err(e),
        }
    }

    fn draft_delete(&self, draft_id: String) -> String {
        match self.load_draft(&draft_id) {
            Ok((p, _)) => {
                let _ = std::fs::remove_file(p);
                json!({ "ok": true }).to_string()
            }
            Err(e) => err(e),
        }
    }

    fn publish(&self, draft_id: String) -> String {
        let (path, d) = match self.load_draft(&draft_id) {
            Ok(x) => x,
            Err(e) => return err(e),
        };
        let secret: [u8; 32] = match b64::decode(d["secret"].as_str().unwrap_or("")).ok().and_then(|v| v.try_into().ok()) {
            Some(s) => s,
            None => return err("this draft has no link secret"),
        };
        let built = match publish::build(&draft_id, d["title"].as_str().unwrap_or(""), d["markdown"].as_str().unwrap_or(""), &secret, now_ms()) {
            Ok(b) => b,
            Err(e) => return err(e),
        };
        let link = built.link.clone();
        let link2 = link.clone();
        self.run_job(
            move |me| {
                let node = me.node(Duration::from_secs(2));
                let gw = me.gateways(node.as_ref());
                for (cid, bytes) in &built.blocks {
                    gw.cache_block(&Cid::parse(cid)?, bytes);
                }
                let names: Vec<String> = built.blocks.iter().map(|(c, _)| c.clone()).collect();
                let mut out = json!({ "ok": true, "link": built.link, "manifest": built.manifest, "blocks": names });
                let mut notes = vec![];
                match &node {
                    Some(n) => match Node::new(&n.api, Duration::from_secs(60)).keep(&gw, &names, &names) {
                        Ok(_) => {
                            out["ipfs"] = json!(n.source);
                            out["peerId"] = json!(n.peer_id);
                        }
                        Err(e) => notes.push(format!("IPFS: {e}")),
                    },
                    None => notes.push("IPFS: no node available, so only this device and Logos Storage hold it".into()),
                }
                let (known, tmp) = {
                    let s = me.state.lock().unwrap();
                    (s.storage.clone(), s.path("tmp").unwrap_or_else(std::env::temp_dir))
                };
                match copy_blocks_to_storage(&gw, &names, &known, &tmp) {
                    Ok(added) => {
                        let map: HashMap<&String, Value> = added.iter().map(|(k, (c, n))| (k, json!([c, n]))).collect();
                        out["storage"] = json!(map);
                    }
                    Err(e) => notes.push(format!("Logos Storage: {e}")),
                }
                if out.get("ipfs").is_none() && out.get("storage").is_none() {
                    return Err(format!("published nowhere: {}", notes.join("; ")));
                }
                out["notes"] = json!(notes);
                Ok(out)
            },
            move |s, v| {
                if let Some(m) = v["storage"].as_object() {
                    for (k, e) in m {
                        if let (Some(c), Some(n)) = (e[0].as_str(), e[1].as_u64()) {
                            s.storage.insert(k.clone(), (c.to_string(), n));
                        }
                    }
                }
                let blocks = v["blocks"].clone();
                let entry = json!({
                    "url": link2, "layout": "logos", "title": d["title"], "kind": "ddoc", "openedAt": now_ms(),
                    "roots": blocks, "blocks": blocks,
                    "kept": if v.get("ipfs").is_some() { blocks.clone() } else { Value::Null },
                    "keptOn": v["peerId"].clone(),
                    "inStorage": if v.get("storage").is_some() { blocks.clone() } else { Value::Null },
                });
                s.recent.retain(|r| r["url"] != entry["url"]);
                s.recent.insert(0, entry);
                s.recent.truncate(MAX_RECENT);
                s.save();
                let mut d = d.clone();
                d["published"] = json!({ "link": link, "manifest": v["manifest"], "at": now_ms() });
                write_private(&path, &d.to_string());
            },
        )
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<FileverseCoreModuleImpl>();
}
