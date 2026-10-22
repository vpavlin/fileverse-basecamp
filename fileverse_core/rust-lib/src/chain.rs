//! The on-chain half: a Fileverse portal contract on Gnosis records, per file,
//! the IPFS CIDs of its metadata, content and (v3) gate. Nothing secret is on chain.

use serde::Serialize;

use crate::abi::{self, Kind};

pub const GNOSIS_CHAIN_ID: u64 = 100;

/// `eth_call` against a chain. The module answers it through eth_rpc_module; the
/// CLI and tests through a plain JSON-RPC endpoint ([`HttpRpc`]).
pub trait Rpc {
    /// Returns the call's raw hex result.
    fn eth_call(&self, to: &str, data: &str) -> Result<String, String>;
    /// A short name for where the answer came from, shown to the user.
    fn describe(&self) -> String;
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// The 2023-2025 Portal contract (docs.fileverse.io links).
    Legacy,
    /// The App contract minted by the v3 AppRegistry (ddocs.new / dsheets.new).
    V3,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub layout: Layout,
    pub file_id: u64,
    pub app_file_id: String,
    pub file_type: u8,
    pub metadata_cid: String,
    pub content_cid: String,
    pub gate_cid: String,
    pub version: u128,
    pub owner: String,
}

pub fn valid_address(a: &str) -> bool {
    a.len() == 42 && a.starts_with("0x") && a[2..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn file_count(rpc: &dyn Rpc, portal: &str) -> Result<u64, String> {
    let raw = rpc.eth_call(portal, &abi::calldata("getFileCount()", None))?;
    let v = abi::decode(&abi::hex_bytes(&raw)?, &[Kind::Uint])?;
    Ok(v[0].as_u128() as u64)
}

/// Read `files(id)`, trying the v3 layout first. Both are plain getters on the
/// same selector, so the layout is told apart by which decode makes sense.
pub fn read_file(rpc: &dyn Rpc, portal: &str, id: u64) -> Result<FileRecord, String> {
    if !valid_address(portal) {
        return Err(format!("not a contract address: {portal}"));
    }
    let raw = rpc.eth_call(portal, &abi::calldata("files(uint256)", Some(id)))?;
    let data = abi::hex_bytes(&raw)?;
    if data.is_empty() {
        return Err(format!("{portal} has no files() getter; is it a Fileverse portal?"));
    }
    use Kind::*;
    if let Ok(v) = abi::decode(&data, &[Str, Uint, Str, Str, Str, Uint, Address]) {
        if looks_like_cid(v[2].as_str()) {
            return Ok(FileRecord {
                layout: Layout::V3,
                file_id: id,
                app_file_id: v[0].as_str().into(),
                file_type: v[1].as_u128() as u8,
                metadata_cid: v[2].as_str().into(),
                content_cid: v[3].as_str().into(),
                gate_cid: v[4].as_str().into(),
                version: v[5].as_u128(),
                owner: v[6].as_str().into(),
            });
        }
    }
    let v = abi::decode(&data, &[Str, Str, Str, Uint, Uint])?;
    if v[0].as_str().is_empty() && v[1].as_str().is_empty() {
        return Err(format!("file {id} does not exist in {portal}"));
    }
    Ok(FileRecord {
        layout: Layout::Legacy,
        file_id: id,
        app_file_id: String::new(),
        file_type: v[3].as_u128() as u8,
        metadata_cid: v[0].as_str().into(),
        content_cid: v[1].as_str().into(),
        gate_cid: v[2].as_str().into(),
        version: v[4].as_u128(),
        owner: String::new(),
    })
}

fn looks_like_cid(s: &str) -> bool {
    (s.starts_with("Qm") && s.len() == 46) || (s.starts_with('b') && s.len() > 50)
}

/// Plain JSON-RPC over HTTPS, for the CLI and tests.
pub struct HttpRpc {
    pub url: String,
    client: reqwest::blocking::Client,
}

impl HttpRpc {
    pub fn new(url: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http client");
        HttpRpc { url: url.into(), client }
    }
}

impl Rpc for HttpRpc {
    fn eth_call(&self, to: &str, data: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [{ "to": to, "data": data }, "latest"],
        });
        let v: serde_json::Value = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .and_then(|r| r.json())
            .map_err(|e| format!("RPC {}: {e}", self.url))?;
        if let Some(e) = v.get("error") {
            return Err(format!("RPC error: {e}"));
        }
        v.get("result")
            .and_then(|r| r.as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("RPC returned no result: {v}"))
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}
