//! Logos module glue for `fileverse_core` (rust-first authoring).
//!
//! The builder derives the `.lidl` from the `FileverseCoreModule` trait below
//! (`codegen.rust = { trait, source: "src/glue.rs" }`). Every method returns a
//! JSON string, `{ "ok": true, ... }` or `{ "ok": false, "error": "<sentence>" }`,
//! and never panics across the boundary.
//!
//! Opening a document takes seconds (a chain read, up to four IPFS blocks, an
//! Argon2id unwrap), so `open` only starts a job and returns its id; the view
//! polls `job(id)` and also hears `job_changed`. `concurrency: "multi"` keeps a
//! running open from stalling `snapshot` polls.
//!
//! Gnosis is reached through eth_rpc_module, the device-wide RPC registry, so the
//! endpoint (and a SOCKS proxy or Tor, if the user set one) is the user's choice,
//! not ours. Chain 100 is seeded there only where absent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
// `Mutex` comes from the generated scaffold (provider_gen.rs imports std::sync::Mutex).
use std::time::Duration;

use serde_json::{json, Value};

use crate::chain::{Rpc, GNOSIS_CHAIN_ID};
use crate::ipfs::{Gateways, DEFAULT_GATEWAYS};
use crate::viewer;

pub trait FileverseCoreModule: Send + Sync + 'static {
    /// Start opening a share link. Returns `{ ok, jobId }` at once.
    fn open(&self, url: String) -> String;
    /// A job's state: `pending` (with the steps so far), `done` (with the document) or `failed`.
    fn job(&self, job_id: String) -> String;
    /// Settings, recent documents and module versions, for the view to poll.
    fn snapshot(&self) -> String;
    /// Replace the IPFS gateway list (a JSON array of base URLs). An empty array restores the defaults.
    fn set_gateways(&self, gateways_json: String) -> String;
    /// Remove a document from the recent list.
    fn forget(&self, portal: String, file_id: i64) -> String;

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
    recent: Vec<Value>,
    jobs: HashMap<String, Job>,
    seq: u64,
    chain_seeded: bool,
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

impl State {
    fn settings_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("settings.json"))
    }

    fn save(&self) {
        if let Some(p) = self.settings_path() {
            let v = json!({ "gateways": self.gateways, "recent": self.recent });
            let _ = std::fs::write(p, v.to_string());
        }
    }

    fn gateway_list(&self) -> Vec<String> {
        if self.gateways.is_empty() {
            DEFAULT_GATEWAYS.iter().map(|s| s.to_string()).collect()
        } else {
            self.gateways.clone()
        }
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

impl FileverseCoreModule for FileverseCoreModuleImpl {
    fn on_context_ready(&self, ctx: &RustModuleContext) {
        // Local state only: the 0.3 runtime refuses calls to other modules from here.
        let dir = PathBuf::from(&ctx.instance_persistence_path);
        let _ = std::fs::create_dir_all(&dir);
        let mut s = self.state.lock().unwrap();
        s.dir = Some(dir.clone());
        if let Ok(b) = std::fs::read(dir.join("settings.json")) {
            if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                s.gateways = v["gateways"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                s.recent = v["recent"].as_array().cloned().unwrap_or_default();
            }
        }
    }

    fn open(&self, url: String) -> String {
        let link = match viewer::parse_link(&url) {
            Ok(l) => l,
            Err(e) => return err(e),
        };
        let (id, gateways, cache, seed) = {
            let mut s = self.state.lock().unwrap();
            s.seq += 1;
            let id = format!("j{}", s.seq);
            s.jobs.insert(id.clone(), Job { state: "pending", result: Value::Null, error: String::new(), started: now_ms() });
            let cache = s.dir.as_ref().map(|d| d.join("blocks"));
            (id, s.gateway_list(), cache, !s.chain_seeded)
        };
        let state = self.state.clone();
        let job_id = id.clone();
        std::thread::spawn(move || {
            if seed {
                match seed_gnosis() {
                    Ok(()) => state.lock().unwrap().chain_seeded = true,
                    Err(e) => eprintln!("fileverse_core: {e}"),
                }
            }
            let gw = Gateways::new(gateways, cache);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| viewer::open_link(&ModuleRpc, &gw, &url)))
                .unwrap_or_else(|_| Err("the viewer crashed while opening this file".into()));
            let mut s = state.lock().unwrap();
            let job = s.jobs.get_mut(&job_id).unwrap();
            match r {
                Ok(v) => {
                    job.state = "done";
                    let entry = json!({
                        "url": url, "portal": link.portal, "fileId": link.file_id,
                        "title": v["title"], "kind": v["kind"], "openedAt": now_ms(),
                    });
                    job.result = v;
                    s.recent.retain(|r| !(r["portal"] == entry["portal"] && r["fileId"] == entry["fileId"]));
                    s.recent.insert(0, entry);
                    s.recent.truncate(MAX_RECENT);
                    s.save();
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
        // Recent entries carry their link keys; the view gets them so it can reopen,
        // and they never leave this device.
        json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "gateways": s.gateway_list(),
            "customGateways": !s.gateways.is_empty(),
            "rpc": "eth_rpc_module (chain 100)",
            "recent": s.recent,
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

    fn forget(&self, portal: String, file_id: i64) -> String {
        let mut s = self.state.lock().unwrap();
        let before = s.recent.len();
        s.recent.retain(|r| !(r["portal"].as_str() == Some(portal.as_str()) && r["fileId"].as_i64() == Some(file_id)));
        s.save();
        json!({ "ok": true, "removed": before - s.recent.len() }).to_string()
    }
}

#[no_mangle]
pub extern "Rust" fn logos_module_install() {
    install::<FileverseCoreModuleImpl>();
}
