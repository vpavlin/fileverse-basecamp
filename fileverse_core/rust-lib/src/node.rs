//! The user's own IPFS node (Kubo RPC API), as a place to keep copies.
//!
//! Fileverse pins its users' files on its own Pinata account and unpins them when
//! it chooses. `keep` copies a document's verified blocks into a node the user runs
//! and pins them there, so the document stays available (and is provided to the
//! IPFS network by the user) whatever Fileverse does. Reading through the node's
//! own gateway first also means no third party sees which CIDs are being read.

use std::time::Duration;

use serde_json::Value;

use crate::ipfs::{Cid, Gateways};

pub const DEFAULT_API: &str = "http://127.0.0.1:5001";
pub const DEFAULT_GATEWAY: &str = "http://127.0.0.1:8080";

fn unreachable(api: &str, e: reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        format!("your IPFS node at {api} is not answering (is it running?)")
    } else {
        format!("IPFS node at {api}: {e}")
    }
}

pub struct Node {
    pub api: String,
    client: reqwest::blocking::Client,
}

impl Node {
    pub fn new(api: &str, timeout: Duration) -> Self {
        let client = reqwest::blocking::Client::builder().timeout(timeout).build().expect("http client");
        Node { api: api.trim_end_matches('/').to_string(), client }
    }

    fn rpc(&self, path: &str) -> Result<Value, String> {
        let r = self
            .client
            .post(format!("{}/api/v0/{path}", self.api))
            .send()
            .map_err(|e| unreachable(&self.api, e))?;
        let status = r.status();
        let v: Value = r.json().map_err(|e| format!("IPFS node: {e}"))?;
        if !status.is_success() {
            return Err(format!("IPFS node: {}", v.get("Message").and_then(Value::as_str).unwrap_or("request failed")));
        }
        Ok(v)
    }

    /// `{ peerId, agent }` if a node answers.
    pub fn id(&self) -> Result<Value, String> {
        let v = self.rpc("id")?;
        Ok(serde_json::json!({ "peerId": v["ID"], "agent": v["AgentVersion"] }))
    }

    pub fn put_block(&self, cid: &Cid, bytes: &[u8]) -> Result<(), String> {
        let codec = match cid.codec {
            0x55 => "raw",
            0x70 => "dag-pb",
            c => return Err(format!("unsupported codec 0x{c:x}")),
        };
        let form = reqwest::blocking::multipart::Form::new()
            .part("file", reqwest::blocking::multipart::Part::bytes(bytes.to_vec()).file_name("block"));
        let r = self
            .client
            .post(format!("{}/api/v0/block/put?cid-codec={codec}&mhtype=sha2-256", self.api))
            .multipart(form)
            .send()
            .map_err(|e| unreachable(&self.api, e))?;
        let v: Value = r.json().map_err(|e| format!("IPFS node: {e}"))?;
        let key = v.get("Key").and_then(Value::as_str).ok_or_else(|| format!("IPFS node refused a block: {v}"))?;
        // The node hashed the bytes itself; it must arrive at the same multihash.
        if Cid::parse(key)?.digest != cid.digest {
            return Err(format!("IPFS node stored {key} for {}", cid.to_v1()));
        }
        Ok(())
    }

    pub fn pin(&self, root: &str) -> Result<(), String> {
        self.rpc(&format!("pin/add?arg={root}&recursive=true")).map(|_| ())
    }

    /// Copy every block into the node, then pin the roots. Blocks come from the
    /// verified cache (or a gateway) and are checked again by the node's own hash.
    pub fn keep(&self, gw: &Gateways, roots: &[String], blocks: &[String]) -> Result<usize, String> {
        for b in blocks {
            let cid = Cid::parse(b)?;
            self.put_block(&cid, &gw.block_bytes(b)?)?;
        }
        for r in roots {
            self.pin(r)?;
        }
        Ok(blocks.len())
    }
}
