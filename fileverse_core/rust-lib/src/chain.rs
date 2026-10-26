//! The on-chain half: a Fileverse portal contract on Gnosis records, per file,
//! the IPFS CIDs of its metadata, content and (v3) gate. Nothing secret is on chain.

use serde::Serialize;
use sha3::Digest;

use crate::abi::{self, Kind};

pub const GNOSIS_CHAIN_ID: u64 = 100;

/// `eth_call` against a chain. The module answers it through eth_rpc_module; the
/// CLI and tests through a plain JSON-RPC endpoint ([`HttpRpc`]).
pub trait Rpc {
    /// Returns the call's raw hex result.
    fn eth_call(&self, to: &str, data: &str) -> Result<String, String>;
    /// A short name for where the answer came from, shown to the user.
    fn describe(&self) -> String;
    /// `eth_getLogs` with this filter; the raw JSON array of logs.
    fn get_logs(&self, _filter: &serde_json::Value) -> Result<serde_json::Value, String> {
        Err("this RPC cannot search logs".into())
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// The 2023-2025 Portal contract (docs.fileverse.io links).
    Legacy,
    /// The App contract minted by the v3 AppRegistry (ddocs.new / dsheets.new).
    V3,
    /// Published from Basecamp: no chain record, a manifest on IPFS / Logos Storage.
    Logos,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub portal: String,
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

/// The v3 AppRegistry on Gnosis (ddocs.new / dsheets.new portals) and the block it
/// was deployed in.
pub const APP_REGISTRY: &str = "0xdb0def9f0992e68d7170e15c4788ff62a0210ca1";
const APP_REGISTRY_FROM_BLOCK: u64 = 42_309_834;

/// A ddocs.new short link, `/d/<12 hex>`: a registry version byte, a 3-byte portal
/// token id and a 2-byte file id.
#[derive(Debug, PartialEq)]
pub struct ShortLink {
    pub version: u8,
    pub token_id: u32,
    pub file_id: u16,
}

pub fn parse_short(hex12: &str) -> Option<ShortLink> {
    if hex12.len() != 12 {
        return None;
    }
    let b = hex::decode(hex12).ok()?;
    Some(ShortLink {
        version: b[0],
        token_id: u32::from_be_bytes([0, b[1], b[2], b[3]]),
        file_id: u16::from_be_bytes([b[4], b[5]]),
    })
}

/// The portal a short link's token id names, read from the registry's
/// `Mint(app, tokenId, account)` log and confirmed with `appInfo(app)`. Fileverse's
/// web app asks its own indexer instead; this needs only a Gnosis RPC that serves
/// `eth_getLogs` over the registry's lifetime.
pub fn resolve_token(rpc: &dyn Rpc, version: u8, token_id: u32) -> Result<String, String> {
    if version != 2 {
        return Err(format!(
            "this short link points into Fileverse's registry v{version}, which can only be searched through Fileverse's indexer; \
             open the document in Fileverse and share the long link (…/<portal address>/<file number>#key=…) instead"
        ));
    }
    let mut token = [0u8; 32];
    token[28..].copy_from_slice(&token_id.to_be_bytes());
    let mint_topic = format!("0x{}", hex::encode(sha3::Keccak256::digest(b"Mint(address,uint256,address)")));
    let filter = serde_json::json!({
        "address": APP_REGISTRY,
        "fromBlock": format!("0x{:x}", APP_REGISTRY_FROM_BLOCK),
        "toBlock": "latest",
        "topics": [mint_topic, null, format!("0x{}", hex::encode(token))],
    });
    let logs = rpc.get_logs(&filter)?;
    let topic = logs
        .as_array()
        .and_then(|a| a.first())
        .and_then(|l| l.get("topics"))
        .and_then(|t| t.get(1))
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("no Fileverse portal has token id {token_id} in the registry"))?;
    let portal = format!("0x{}", topic.trim_start_matches("0x").get(24..).ok_or("bad Mint log")?);
    // Do not take the log's word for it: the registry must say the same.
    let mut data = abi::selector("appInfo(address)").to_vec();
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&hex::decode(&portal[2..]).map_err(|e| e.to_string())?);
    let raw = rpc.eth_call(APP_REGISTRY, &format!("0x{}", hex::encode(data)))?;
    let v = abi::decode(&abi::hex_bytes(&raw)?, &[abi::Kind::Address, abi::Kind::Uint, abi::Kind::Uint, abi::Kind::Address])?;
    if !v[0].as_str().eq_ignore_ascii_case(&portal) || v[2].as_u128() != token_id as u128 {
        return Err(format!("the registry does not confirm portal {portal} for token id {token_id}"));
    }
    Ok(portal)
}

/// Which file of a v3 portal carries this document id (`/document/<id>` links), read
/// with `appFileIdToFileId` and confirmed by the file record itself.
pub fn find_app_file(rpc: &dyn Rpc, portal: &str, app_file_id: &str) -> Result<Option<FileRecord>, String> {
    if !valid_address(portal) || app_file_id.is_empty() || app_file_id.len() > 64 {
        return Ok(None);
    }
    let mut data = abi::selector("appFileIdToFileId(string)").to_vec();
    let mut word = [0u8; 32];
    word[31] = 0x20;
    data.extend_from_slice(&word);
    word = [0u8; 32];
    word[24..].copy_from_slice(&(app_file_id.len() as u64).to_be_bytes());
    data.extend_from_slice(&word);
    let mut b = app_file_id.as_bytes().to_vec();
    b.resize(app_file_id.len().div_ceil(32) * 32, 0);
    data.extend_from_slice(&b);
    // Legacy portals have no such function: the call reverts, which just means "not here".
    let Ok(raw) = rpc.eth_call(portal, &format!("0x{}", hex::encode(data))) else { return Ok(None) };
    let v = abi::decode(&abi::hex_bytes(&raw)?, &[Kind::Uint])?;
    let id = v[0].as_u128() as u64;
    match read_file(rpc, portal, id) {
        Ok(rec) if rec.app_file_id == app_file_id => Ok(Some(rec)),
        _ => Ok(None),
    }
}

pub fn file_count(rpc: &dyn Rpc, portal: &str) -> Result<u64, String> {
    if !valid_address(portal) {
        return Err(format!("not a contract address: {portal}"));
    }
    let raw = rpc.eth_call(portal, &abi::calldata("getFileCount()", None))?;
    let data = abi::hex_bytes(&raw)?;
    if data.is_empty() {
        return Err(format!("{portal} is not a Fileverse portal on Gnosis (no contract answers there)"));
    }
    let v = abi::decode(&data, &[Kind::Uint])?;
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
                portal: portal.to_string(),
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
        portal: portal.to_string(),
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

    fn get_logs(&self, filter: &serde_json::Value) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_getLogs", "params": [filter] });
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
        v.get("result").cloned().ok_or_else(|| format!("RPC returned no result: {v}"))
    }
}
