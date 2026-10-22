//! Fetching Fileverse blobs from IPFS without trusting the gateway.
//!
//! Every block is asked for as a trustless raw block (`?format=raw`), hashed, and
//! compared to the CID before it is used; UnixFS files spread over several blocks
//! are reassembled from their verified leaves. A gateway can therefore refuse or
//! stall, but it cannot hand us different bytes. Gateways that only serve whole
//! files are used as a last resort, and the result is marked unverified unless the
//! CID is a single raw block whose hash we can still check.

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

pub const DEFAULT_GATEWAYS: &[&str] = &[
    // Serves Fileverse v3 uploads (they are pinned on Pinata) and trustless blocks.
    "https://gateway.pinata.cloud",
    // Fileverse's own dedicated gateway: legacy portal content only.
    "https://ipfs.fileverse.io",
    "https://ipfs.io",
    "https://dweb.link",
];

const MAX_BLOCKS: usize = 4096;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Cid {
    pub codec: u64,
    pub digest: [u8; 32],
}

const RAW: u64 = 0x55;
const DAG_PB: u64 = 0x70;

fn varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*i).ok_or("truncated varint")?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err("varint too long".into());
        }
    }
}

fn base58(s: &str) -> Result<Vec<u8>, String> {
    const ALPHA: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out: Vec<u8> = vec![];
    for c in s.bytes() {
        let mut carry = ALPHA.iter().position(|a| *a == c).ok_or("bad base58")? as u32;
        for b in out.iter_mut().rev() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.bytes().take_while(|c| *c == b'1').count();
    let mut v = vec![0u8; zeros];
    v.extend(out);
    Ok(v)
}

fn base32(s: &str) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return Err("bad base32".into()),
        } as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn multihash(b: &[u8], i: &mut usize) -> Result<[u8; 32], String> {
    let code = varint(b, i)?;
    let len = varint(b, i)? as usize;
    if code != 0x12 || len != 32 {
        return Err(format!("unsupported multihash 0x{code:x}/{len}"));
    }
    let d = b.get(*i..*i + 32).ok_or("truncated multihash")?;
    *i += 32;
    Ok(d.try_into().unwrap())
}

impl Cid {
    pub fn parse(s: &str) -> Result<Cid, String> {
        let s = s.trim();
        if s.starts_with("Qm") {
            let b = base58(s)?;
            let mut i = 0;
            return Ok(Cid { codec: DAG_PB, digest: multihash(&b, &mut i)? });
        }
        if let Some(rest) = s.strip_prefix('b') {
            return Cid::from_bytes(&base32(rest)?);
        }
        Err(format!("unsupported CID encoding: {s}"))
    }

    /// A CID as embedded in a dag-pb link: v0 is the bare multihash.
    pub fn from_bytes(b: &[u8]) -> Result<Cid, String> {
        let mut i = 0;
        if b.first() == Some(&0x12) {
            return Ok(Cid { codec: DAG_PB, digest: multihash(b, &mut i)? });
        }
        if varint(b, &mut i)? != 1 {
            return Err("unsupported CID version".into());
        }
        let codec = varint(b, &mut i)?;
        Ok(Cid { codec, digest: multihash(b, &mut i)? })
    }

    /// CIDv1 base32, the form every gateway accepts.
    pub fn to_v1(&self) -> String {
        let mut b = vec![1u8];
        let mut c = self.codec;
        loop {
            let byte = (c & 0x7f) as u8;
            c >>= 7;
            if c == 0 {
                b.push(byte);
                break;
            }
            b.push(byte | 0x80);
        }
        b.extend([0x12, 0x20]);
        b.extend(self.digest);
        const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut s = String::from("b");
        let (mut buf, mut bits) = (0u32, 0);
        for byte in b {
            buf = (buf << 8) | byte as u32;
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                s.push(A[((buf >> bits) & 31) as usize] as char);
            }
            buf &= (1 << bits) - 1;
        }
        if bits > 0 {
            s.push(A[((buf << (5 - bits)) & 31) as usize] as char);
        }
        s
    }
}

/// Minimal protobuf reader for dag-pb nodes and their UnixFS payload.
fn pb_fields(b: &[u8]) -> Result<Vec<(u64, Vec<u8>)>, String> {
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => out.push((field, varint(b, &mut i)?.to_le_bytes().to_vec())),
            2 => {
                let len = varint(b, &mut i)? as usize;
                let v = b.get(i..i + len).ok_or("truncated protobuf")?.to_vec();
                i += len;
                out.push((field, v));
            }
            _ => return Err(format!("unsupported protobuf wire type {wire}")),
        }
    }
    Ok(out)
}

struct PbNode {
    links: Vec<Cid>,
    data: Vec<u8>,
}

fn parse_dag_pb(block: &[u8]) -> Result<PbNode, String> {
    let mut links = vec![];
    let mut unixfs = vec![];
    for (f, v) in pb_fields(block)? {
        match f {
            2 => {
                for (lf, lv) in pb_fields(&v)? {
                    if lf == 1 {
                        links.push(Cid::from_bytes(&lv)?);
                    }
                }
            }
            1 => unixfs = v,
            _ => {}
        }
    }
    let mut data = vec![];
    for (f, v) in pb_fields(&unixfs)? {
        if f == 2 {
            data = v;
        }
    }
    Ok(PbNode { links, data })
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub cid: String,
    pub gateway: String,
    pub verified: bool,
    pub bytes: usize,
    /// Every block the file is made of (CIDv1), for pinning a copy elsewhere.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
}

pub struct Gateways {
    pub urls: Vec<String>,
    /// Verified blocks are cached by CID: they are immutable, so a cache is always valid.
    pub cache_dir: Option<PathBuf>,
    /// Accept whole-file answers we cannot hash-check (dag-pb files from a gateway
    /// that does not serve raw blocks). Off by default.
    pub allow_unverified: bool,
    client: reqwest::blocking::Client,
}

impl Gateways {
    pub fn new(urls: Vec<String>, cache_dir: Option<PathBuf>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("fileverse-basecamp-viewer")
            .build()
            .expect("http client");
        Gateways { urls, cache_dir, allow_unverified: false, client }
    }

    fn cached(&self, cid: &Cid) -> Option<Vec<u8>> {
        let p = self.cache_dir.as_ref()?.join(cid.to_v1());
        let b = std::fs::read(p).ok()?;
        (Sha256::digest(&b).as_slice() == cid.digest).then_some(b)
    }

    fn store(&self, cid: &Cid, b: &[u8]) {
        if let Some(d) = &self.cache_dir {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::write(d.join(cid.to_v1()), b);
        }
    }

    fn block(&self, cid: &Cid, errors: &mut Vec<String>) -> Option<(Vec<u8>, String)> {
        if let Some(b) = self.cached(cid) {
            return Some((b, "cache".into()));
        }
        let name = cid.to_v1();
        for g in &self.urls {
            let url = format!("{}/ipfs/{}?format=raw", g.trim_end_matches('/'), name);
            let r = self
                .client
                .get(&url)
                .header("Accept", "application/vnd.ipld.raw")
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes());
            match r {
                Ok(b) if Sha256::digest(&b).as_slice() == cid.digest => {
                    self.store(cid, &b);
                    return Some((b.to_vec(), g.clone()));
                }
                Ok(_) => errors.push(format!("{g}: block does not match its CID")),
                Err(e) => errors.push(format!("{g}: {}", short(&e))),
            }
        }
        None
    }

    /// One verified block, from the cache or a gateway.
    pub fn block_bytes(&self, cid_str: &str) -> Result<Vec<u8>, String> {
        let cid = Cid::parse(cid_str)?;
        let mut errors = vec![];
        self.block(&cid, &mut errors)
            .map(|(b, _)| b)
            .ok_or_else(|| format!("could not fetch block {cid_str}: {}", errors.join("; ")))
    }

    /// The bytes of the file `cid` names, checked against it block by block.
    pub fn get(&self, cid_str: &str) -> Result<(Vec<u8>, Provenance), String> {
        let root = Cid::parse(cid_str)?;
        let mut errors = vec![];
        let mut used = String::new();
        let mut out = vec![];
        let mut stack = vec![root.clone()];
        let mut names = vec![];
        let mut ok = true;
        while let Some(cid) = stack.pop() {
            names.push(cid.to_v1());
            if names.len() > MAX_BLOCKS || out.len() > MAX_BYTES {
                return Err(format!("{cid_str} is larger than the viewer accepts"));
            }
            let Some((b, g)) = self.block(&cid, &mut errors) else {
                ok = false;
                break;
            };
            if used.is_empty() {
                used = g;
            }
            match cid.codec {
                RAW => out.extend(b),
                DAG_PB => {
                    let n = parse_dag_pb(&b)?;
                    out.extend(n.data);
                    stack.extend(n.links.into_iter().rev());
                }
                c => return Err(format!("unsupported IPLD codec 0x{c:x}")),
            }
        }
        if ok {
            let bytes = out.len();
            return Ok((out, Provenance { cid: cid_str.into(), gateway: used, verified: true, bytes, blocks: names }));
        }
        if self.allow_unverified {
            for g in &self.urls {
                let url = format!("{}/ipfs/{}", g.trim_end_matches('/'), cid_str);
                if let Ok(b) = self.client.get(&url).send().and_then(|r| r.error_for_status()).and_then(|r| r.bytes()) {
                    let verified = root.codec == RAW && Sha256::digest(&b).as_slice() == root.digest;
                    let bytes = b.len();
                    return Ok((b.to_vec(), Provenance { cid: cid_str.into(), gateway: g.clone(), verified, bytes, blocks: vec![] }));
                }
            }
        }
        Err(format!("could not fetch {cid_str}: {}", errors.join("; ")))
    }
}

fn short(e: &reqwest::Error) -> String {
    if let Some(s) = e.status() {
        return format!("HTTP {}", s.as_u16());
    }
    if e.is_timeout() {
        return "timed out".into();
    }
    if e.is_connect() {
        return "could not connect".into();
    }
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_and_v1_agree() {
        let v0 = Cid::parse("QmcD5coUfyr1TK9ULWsAXZx4GWCau5BFSugyaU7WLhHdUa").unwrap();
        assert_eq!(v0.codec, DAG_PB);
        let again = Cid::parse(&v0.to_v1()).unwrap();
        assert_eq!(again, v0);
        let raw = Cid::parse("bafkreifnijnw7kdzhmpt7gyapavoxf3p4lf4nsodukwj5tgg67e2wcdvse").unwrap();
        assert_eq!(raw.codec, RAW);
        assert_eq!(raw.to_v1(), "bafkreifnijnw7kdzhmpt7gyapavoxf3p4lf4nsodukwj5tgg67e2wcdvse");
    }
}
