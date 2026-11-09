//! Shared drafts: collaborative editing of a draft's Markdown over Logos Delivery.
//!
//! A draft is private until its owner shares it. Sharing gives it a random 32-byte id `T`,
//! the document's **shared secret**: whoever has it can read and write. Everything sent for
//! the document is sealed with a key derived from `T`, on two content topics derived from
//! `T`, so relays see neither the id nor the text.
//!
//! The text is a HashSeq (David Rusu's BFT sequence CRDT): every operation is identified by
//! the hash of its content and of the operations it saw, so ids cannot be forged and
//! history is a hash DAG. Sync follows the design of Reflection over Logos (vpavlin's fork
//! of p2panda Reflection, docs/logos/spec.md), adapted to HashSeq's frontier/delta frames:
//!
//! - **ops**: new operations, as a HashSeq delta frame (`0xDE`);
//! - **heads**: the sender's frontier (its clock, `0xC1`); a peer holding operations outside
//!   it repairs after a random 50-500 ms (skipping what someone else sent meanwhile); a peer
//!   that sees tips it lacks sends its own heads;
//! - **eph**: presence (hello, ping, bye) and cursors, JSON, never stored.
//!
//! Only the draft the user has open ("active") publishes anything of its own: heads 0.5, 5
//! and 30 s after opening, presence every 120 s, local edits merged into at most one ops
//! message every [`PUBLISH_EVERY`]. A draft in the background answers repairs and nothing
//! else (Reflection ADR 0008, the RLN message budget).
//!
//! This module is transport-free: it reads frames given to [`Collab::receive`], and leaves
//! what to send in an outbox ([`Collab::take_outbox`]). The glue moves both through
//! loam_core.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use aes_gcm::aead::{Aead, KeyInit, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, Nonce};
use hashseq::encoding::{decode_frontier, decode_id, decode_op, decode_varint, encode_delta, encode_frontier, EncodableOp};
use hashseq::hash_node::{Anchor, HashNode, Op, Payload};
use hashseq::{HashWeb, Id};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Changes the wire: bump with any incompatible change.
pub const PROTOCOL: &str = "fileverse-collab-v1";
/// The longest sealed message (Waku carries about 150 KB).
pub const MAX_MESSAGE: usize = 120 * 1024;
/// Local edits of the open draft go out at most this often (milliseconds).
pub const PUBLISH_EVERY: u64 = 2_000;
/// Presence ping while a draft is open (milliseconds); a peer silent for twice this is offline.
pub const PRESENCE_EVERY: u64 = 120_000;
/// Heads after a draft becomes active (milliseconds after).
pub const HEADS_AFTER: [u64; 3] = [500, 5_000, 30_000];
/// The longest text a shared draft may have (characters).
pub const MAX_TEXT: usize = 200_000;
/// Cursor updates go out at most this often (milliseconds).
pub const CURSOR_EVERY: u64 = 1_000;

const TAG_OPS: u8 = 0x01;
const TAG_HEADS: u8 = 0x02;
const TAG_EPH: u8 = 0x03;
const SEAL_VERSION: u8 = 0x01;
const KIND_SEQ: u8 = 1;

fn derive(label: &str, t: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PROTOCOL.as_bytes());
    h.update([0u8]);
    h.update(label.as_bytes());
    h.update([0u8]);
    h.update(t);
    h.finalize().into()
}

/// The content topics of a shared draft: (ops, ephemeral).
pub fn topics(t: &[u8; 32]) -> (String, String) {
    let name = hex::encode(&derive("topic", t)[..16]);
    (format!("/fileverse/1/{name}-ops/bin"), format!("/fileverse/1/{name}-eph/bin"))
}

/// `0x01 ‖ nonce(12) ‖ AES-256-GCM(key, nonce, plaintext, aad = 0x01)`. The nonce is
/// `sha256(key ‖ plaintext)[..12]`: the same frame always seals to the same bytes (relays
/// drop the repeat), different frames get different nonces.
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(key);
    h.update(plaintext);
    let digest = h.finalize();
    let nonce = &digest[..12];
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let ct = cipher
        .encrypt(Nonce::from_slice(nonce), AeadPayload { msg: plaintext, aad: &[SEAL_VERSION] })
        .expect("in-memory encryption");
    let mut out = Vec::with_capacity(1 + 12 + ct.len());
    out.push(SEAL_VERSION);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

pub fn open(key: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 1 + 12 + 16 || sealed[0] != SEAL_VERSION {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher
        .decrypt(Nonce::from_slice(&sealed[1..13]), AeadPayload { msg: &sealed[13..], aad: &[SEAL_VERSION] })
        .ok()
}

/// A shared draft's id as people pass it around: 64 hex characters.
pub fn parse_id(s: &str) -> Result<[u8; 32], String> {
    let cleaned: String = s.trim().chars().filter(|c| !c.is_whitespace()).collect();
    let cleaned = cleaned.strip_prefix("fvshare:").unwrap_or(&cleaned);
    let bytes = hex::decode(cleaned).map_err(|_| "a shared draft id is 64 hex characters".to_string())?;
    bytes.try_into().map_err(|_| "a shared draft id is 64 hex characters".to_string())
}

/// Someone else with the draft open, as last heard.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Peer {
    pub name: String,
    pub color: String,
    /// Element ids around the cursor: the char before the caret (`None`: at the start)
    /// and before the selection's other end.
    #[serde(skip)]
    pub caret: Option<Option<Id>>,
    #[serde(skip)]
    pub anchor: Option<Option<Id>>,
    pub last_seen: u64,
    pub online: bool,
}

/// What the view sent last, kept to rebase an edit made on it after remote changes.
struct Version {
    n: u64,
    ids: Vec<Id>,
    text: String,
}

struct Repair {
    due: u64,
    clock: hashseq::HashWebClock,
    seen_since: HashSet<Id>,
}

/// One shared draft.
pub struct Shared {
    pub t: [u8; 32],
    key: [u8; 32],
    pub ops_topic: String,
    pub eph_topic: String,
    web: HashWeb,
    obj: Id,
    versions: VecDeque<Version>,
    next_version: u64,
    /// The clock after our last ops message: what local edits since then add goes next.
    published: hashseq::HashWebClock,
    last_publish: u64,
    active: bool,
    activated_at: u64,
    heads_sent: usize,
    last_heads: u64,
    want_heads: bool,
    last_presence: u64,
    last_cursor_sent: u64,
    cursor_pending: Option<(Option<Id>, Option<Id>)>,
    /// Someone new said hello: answer with our presence (and cursor) at this time.
    presence_reply: Option<u64>,
    /// Operations received from others since our last ops message: not ours to send.
    from_others: HashSet<Id>,
    /// Heads after a burst of local edits, so that a peer who lost a message asks for it
    /// (relays drop messages; a lost edit would otherwise only be noticed on reopening).
    heads_after_burst: Option<u64>,
    repairs: Vec<Repair>,
    pub peers: BTreeMap<String, Peer>,
    /// The text changed (locally or from others) since the glue last saved it.
    pub dirty: bool,
}

impl Shared {
    fn new(t: [u8; 32], web: Option<HashWeb>) -> Shared {
        let key = derive("key", &t);
        let (ops_topic, eph_topic) = topics(&t);
        let origin = Id(derive("origin", &t));
        let mut web = web.unwrap_or_default();
        web.enable_delta_sync();
        let obj = web.create_seq(origin);
        let published = web.clock();
        Shared {
            t,
            key,
            ops_topic,
            eph_topic,
            web,
            obj,
            versions: VecDeque::new(),
            next_version: 1,
            published,
            last_publish: 0,
            active: false,
            activated_at: 0,
            heads_sent: HEADS_AFTER.len(),
            last_heads: 0,
            want_heads: false,
            last_presence: 0,
            last_cursor_sent: 0,
            cursor_pending: None,
            presence_reply: None,
            from_others: HashSet::new(),
            heads_after_burst: None,
            repairs: vec![],
            peers: BTreeMap::new(),
            dirty: false,
        }
    }

    pub fn id_hex(&self) -> String {
        hex::encode(self.t)
    }

    fn seq(&self) -> &hashseq::HashSeq {
        self.web.seq(&self.obj).expect("the draft's sequence")
    }

    pub fn text(&self) -> String {
        self.seq().iter().collect()
    }

    fn ids(&self) -> Vec<Id> {
        self.seq().iter_ids().copied().collect()
    }

    /// The encoded document, to keep on disk.
    pub fn snapshot(&self) -> Vec<u8> {
        hashseq::encode_hashweb(&self.web)
    }

    /// Remember the text the view is about to get, as version `n`.
    fn record_version(&mut self) -> u64 {
        let text = self.text();
        if let Some(v) = self.versions.back() {
            if v.text == text {
                return v.n;
            }
        }
        let n = self.next_version;
        self.next_version += 1;
        self.versions.push_back(Version { n, ids: self.ids(), text });
        while self.versions.len() > 64 {
            self.versions.pop_front();
        }
        n
    }

    /// The text of version `base` (the current text if it is not remembered).
    pub fn text_of(&self, base: Option<u64>) -> String {
        base.and_then(|b| self.versions.iter().find(|v| v.n == b)).map(|v| v.text.clone()).unwrap_or_else(|| self.text())
    }

    pub fn current_version(&mut self) -> u64 {
        self.record_version()
    }

    /// Apply the user's whole text, edited from version `base`. Returns whether anything
    /// changed. An edit made on an older version is rebased: its inserts attach to the
    /// elements they followed then, its removals name the elements, so concurrent changes
    /// by others are kept.
    pub fn set_text(&mut self, base: Option<u64>, text: &str) -> Result<bool, String> {
        if text.chars().count() > MAX_TEXT {
            return Err(format!("a shared draft holds at most {MAX_TEXT} characters"));
        }
        let current = self.text();
        let base_v = base.and_then(|b| self.versions.iter().find(|v| v.n == b));
        let (base_text, base_ids) = match base_v {
            Some(v) if v.text != current => (v.text.clone(), Some(v.ids.clone())),
            _ => (current.clone(), None),
        };
        if base_text == text {
            return Ok(false);
        }
        let old: Vec<char> = base_text.chars().collect();
        let new: Vec<char> = text.chars().collect();
        let (start, old_end, new_end) = diff_span(&old, &new);
        let inserted: Vec<char> = new[start..new_end].to_vec();
        match base_ids {
            None => {
                let seq = self.web.seq_mut(&self.obj).expect("seq");
                if old_end > start {
                    seq.remove_batch(start, old_end - start);
                }
                if !inserted.is_empty() {
                    seq.insert_batch(start, inserted);
                }
            }
            Some(ids) => {
                if old_end > start {
                    let targets: BTreeSet<Id> = ids[start..old_end].iter().copied().collect();
                    let pins = self.pins_except(&targets);
                    let node = HashNode { pins, op: Op::Remove(targets) };
                    self.web.apply_to(self.obj, node).map_err(|e| format!("{e:?}"))?;
                }
                if !inserted.is_empty() {
                    let at = if start > 0 {
                        Anchor::After(ids[start - 1])
                    } else if let Some(first) = ids.first() {
                        Anchor::Before(*first)
                    } else {
                        // the text was empty then: insert where an empty text starts now
                        let seq = self.web.seq_mut(&self.obj).expect("seq");
                        seq.insert_batch(0, inserted);
                        self.dirty = true;
                        return Ok(true);
                    };
                    let mut named = BTreeSet::new();
                    named.insert(*at.id());
                    let pins = self.pins_except(&named);
                    let mut prev: Option<Id> = None;
                    for ch in inserted {
                        let node = match prev {
                            None => HashNode { pins: pins.clone(), op: Op::Insert { at: at.clone(), payload: Payload::Char(ch) } },
                            Some(p) => HashNode { pins: BTreeSet::new(), op: Op::Insert { at: Anchor::After(p), payload: Payload::Char(ch) } },
                        };
                        let id = node.id();
                        self.web.apply_to_with_id(self.obj, id, node).map_err(|e| format!("{e:?}"))?;
                        prev = Some(id);
                    }
                }
            }
        }
        self.dirty = true;
        Ok(true)
    }

    /// The tips an op pins: every current tip it does not already name.
    fn pins_except(&self, named: &BTreeSet<Id>) -> BTreeSet<Id> {
        self.seq().tips().into_iter().filter(|t| !named.contains(t)).collect()
    }

    /// Element ids around a cursor given as a char offset into version `base`'s text.
    fn id_before(&self, base: Option<u64>, pos: usize) -> Option<Id> {
        if pos == 0 {
            return None;
        }
        match base.and_then(|b| self.versions.iter().find(|v| v.n == b)) {
            Some(v) => v.ids.get(pos - 1).copied(),
            None => self.seq().id_at(pos - 1),
        }
    }

    /// Where an element-anchored cursor is now, as a char offset in the current text.
    fn offset_after(&self, id: &Option<Id>) -> Option<usize> {
        match id {
            None => Some(0),
            Some(id) => self.seq().position_of(id).map(|p| p + 1).or_else(|| {
                // the char was removed: the nearest char before it that is still there
                None
            }),
        }
    }

    pub fn set_cursor(&mut self, base: Option<u64>, caret: usize, anchor: usize) {
        let c = (self.id_before(base, caret), self.id_before(base, anchor));
        self.cursor_pending = Some(c);
    }

    /// Others' cursors as char offsets into the current text.
    pub fn cursors(&self) -> Vec<(String, Peer, Option<usize>, Option<usize>)> {
        self.peers
            .iter()
            .filter(|(_, p)| p.online)
            .map(|(k, p)| {
                let caret = p.caret.as_ref().and_then(|c| self.offset_after(c));
                let anchor = p.anchor.as_ref().and_then(|c| self.offset_after(c));
                (k.clone(), p.clone(), caret, anchor)
            })
            .collect()
    }

    /// Apply a received delta frame. Returns the ids of the operations in it.
    fn apply_delta(&mut self, bytes: &[u8]) -> Result<Vec<Id>, String> {
        let mut ids = vec![];
        if bytes.first() != Some(&hashseq::encoding::DELTA_TAG) {
            return Err("not a delta".into());
        }
        let mut pos = 1;
        let varint = |pos: &mut usize| -> Result<usize, String> {
            let (v, used) = decode_varint(bytes.get(*pos..).unwrap_or(&[])).map_err(|e| format!("{e:?}"))?;
            *pos += used;
            Ok(v)
        };
        while pos < bytes.len() {
            let kind = bytes[pos];
            pos += 1;
            let (origin, used) = decode_id(&bytes[pos..]).map_err(|e| format!("{e:?}"))?;
            pos += used;
            let n = varint(&mut pos)?;
            if kind != KIND_SEQ || Id(derive("origin", &self.t)) != origin {
                return Err("a delta for another document".into());
            }
            for _ in 0..n {
                let len = varint(&mut pos)?;
                let slice = bytes.get(pos..pos + len).ok_or("truncated delta")?;
                pos += len;
                let (op, used) = decode_op(slice).map_err(|e| format!("{e:?}"))?;
                if used != len {
                    return Err("bad node length".into());
                }
                match op {
                    EncodableOp::Node(node) => {
                        let id = node.id();
                        ids.push(id);
                        if let Err(e) = self.web.apply_to_with_id(self.obj, id, node) {
                            eprintln!("fileverse-collab: refused op {}: {e:?}", hex::encode(&id.0[..4]));
                        }
                    }
                    EncodableOp::Run(_) => return Err("run form in a delta".into()),
                }
            }
        }
        Ok(ids)
    }

    /// Delta frames holding `nodes`, each sealed message under [`MAX_MESSAGE`].
    fn delta_frames(&self, nodes: Vec<HashNode>) -> Vec<Vec<u8>> {
        let origin = self.seq().origin();
        let mut frames = vec![];
        let mut batch: Vec<HashNode> = vec![];
        let mut size = 0usize;
        for n in nodes {
            let mut b = vec![];
            hashseq::encoding::encode_hash_node(&n, &mut b);
            if !batch.is_empty() && size + b.len() + 8 > MAX_MESSAGE - 128 {
                frames.push(encode_delta(&[(KIND_SEQ, origin, std::mem::take(&mut batch))]));
                size = 0;
            }
            size += b.len() + 8;
            batch.push(n);
        }
        if !batch.is_empty() {
            frames.push(encode_delta(&[(KIND_SEQ, origin, batch)]));
        }
        frames
    }

    fn has_orphans(&self) -> bool {
        self.web.orphans().next().is_some()
    }
}

/// Something to send: a content topic and the sealed bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub topic: String,
    pub sealed: Vec<u8>,
    pub kind: &'static str,
}

#[derive(Serialize, Deserialize)]
struct Eph {
    /// "hello" | "ping" | "bye" | "cursor"
    k: String,
    a: String,
    n: String,
    c: String,
    #[serde(default)]
    caret: Option<String>,
    #[serde(default)]
    anchor: Option<String>,
    t: u64,
}

fn id_opt_hex(id: &Option<Id>) -> String {
    id.map(|i| hex::encode(i.0)).unwrap_or_default()
}

fn hex_id_opt(s: &str) -> Option<Option<Id>> {
    if s.is_empty() {
        return Some(None);
    }
    let b: [u8; 32] = hex::decode(s).ok()?.try_into().ok()?;
    Some(Some(Id(b)))
}

/// Every shared draft on this device, and who we are.
pub struct Collab {
    pub me: String,
    pub name: String,
    pub color: String,
    pub docs: BTreeMap<String, Shared>,
    outbox: Vec<Outgoing>,
    rng: u64,
    pub sent: BTreeMap<&'static str, u64>,
    pub received: BTreeMap<&'static str, u64>,
}

impl Collab {
    /// `me`: this device's author id (hex), stable across restarts.
    pub fn new(me: String, name: String, seed: u64) -> Collab {
        let color = color_of(&me);
        Collab { me, name, color, docs: BTreeMap::new(), outbox: vec![], rng: seed | 1, sent: BTreeMap::new(), received: BTreeMap::new() }
    }

    fn rand(&mut self, lo: u64, hi: u64) -> u64 {
        // xorshift: delays only, nothing secret
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        lo + self.rng % (hi - lo).max(1)
    }

    /// Start sharing a draft whose text is `text`: a new shared document.
    pub fn share(&mut self, t: [u8; 32], text: &str) -> &mut Shared {
        let mut s = Shared::new(t, None);
        if !text.is_empty() {
            s.web.seq_mut(&s.obj).expect("seq").insert_batch(0, text.chars().take(MAX_TEXT));
        }
        // the starting text reaches joiners by repair: it is not an edit to broadcast
        s.published = s.web.clock();
        let hex = s.id_hex();
        self.docs.insert(hex.clone(), s);
        self.docs.get_mut(&hex).unwrap()
    }

    /// Restore (or join, with no snapshot) a shared document.
    pub fn load(&mut self, t: [u8; 32], snapshot: Option<&[u8]>) -> Result<&mut Shared, String> {
        let web = match snapshot {
            Some(b) => Some(hashseq::decode_hashweb(b).map_err(|e| format!("the stored shared draft is unreadable: {e:?}"))?),
            None => None,
        };
        let s = Shared::new(t, web);
        let hex = s.id_hex();
        self.docs.insert(hex.clone(), s);
        Ok(self.docs.get_mut(&hex).unwrap())
    }

    pub fn remove(&mut self, id: &str, now: u64) {
        if self.docs.get(id).is_some_and(|d| d.active) {
            self.set_active(None, now);
        }
        self.docs.remove(id);
    }

    /// The draft the user has open (or none). Only it publishes.
    pub fn set_active(&mut self, id: Option<&str>, now: u64) {
        let ids: Vec<String> = self.docs.keys().cloned().collect();
        for k in ids {
            let on = id == Some(k.as_str());
            let was = self.docs[&k].active;
            if on && !was {
                let d = self.docs.get_mut(&k).unwrap();
                d.active = true;
                d.activated_at = now;
                d.heads_sent = 0;
                d.last_presence = now;
                self.eph(&k, "hello", now);
            } else if !on && was {
                self.flush_local(&k, now, true);
                self.eph(&k, "bye", now);
                let d = self.docs.get_mut(&k).unwrap();
                d.active = false;
                d.heads_sent = HEADS_AFTER.len();
            }
        }
    }

    fn push(&mut self, topic: String, key: &[u8; 32], plaintext: &[u8], kind: &'static str) {
        *self.sent.entry(kind).or_default() += 1;
        self.outbox.push(Outgoing { topic, sealed: seal(key, plaintext), kind });
    }

    fn eph(&mut self, id: &str, k: &str, now: u64) {
        let Some(d) = self.docs.get(id) else { return };
        let (caret, anchor) = match (&d.cursor_pending, k) {
            (Some((c, a)), "cursor") => (Some(id_opt_hex(c)), Some(id_opt_hex(a))),
            _ => (None, None),
        };
        let e = Eph { k: k.into(), a: self.me.clone(), n: self.name.clone(), c: self.color.clone(), caret, anchor, t: now };
        let mut pt = vec![TAG_EPH];
        pt.extend_from_slice(&serde_json::to_vec(&e).unwrap_or_default());
        let (topic, key) = (d.eph_topic.clone(), d.key);
        let kind = if k == "cursor" { "cursor" } else { "presence" };
        self.push(topic, &key, &pt, kind);
    }

    fn send_heads(&mut self, id: &str, now: u64) {
        let Some(d) = self.docs.get_mut(id) else { return };
        d.last_heads = now;
        d.want_heads = false;
        let mut pt = vec![TAG_HEADS];
        // makes each heads unique: a transport that drops repeats would drop the request
        pt.extend_from_slice(&now.to_be_bytes());
        pt.extend_from_slice(&encode_frontier(&d.web));
        let (topic, key) = (d.ops_topic.clone(), d.key);
        self.push(topic, &key, &pt, "heads");
    }

    /// Send what local edits added since the last ops message.
    fn flush_local(&mut self, id: &str, now: u64, force: bool) {
        let Some(d) = self.docs.get_mut(id) else { return };
        if !force && now < d.last_publish + PUBLISH_EVERY {
            return;
        }
        let nodes: Vec<HashNode> = d
            .web
            .deltas_for(&d.published)
            .into_iter()
            .flat_map(|(_, _, n)| n)
            .filter(|n| !d.from_others.contains(&n.id()))
            .collect();
        if nodes.is_empty() {
            // only others' operations since the last send: nothing of ours to send
            if !d.from_others.is_empty() {
                d.published = d.web.clock();
                d.from_others.clear();
            }
            return;
        }
        d.published = d.web.clock();
        d.from_others.clear();
        d.last_publish = now;
        d.heads_after_burst = Some(now + 3_000);
        eprintln!("fileverse-collab: ops out: {} ops", nodes.len());
        let frames = d.delta_frames(nodes);
        let (topic, key) = (d.ops_topic.clone(), d.key);
        for f in frames {
            let mut pt = vec![TAG_OPS];
            pt.extend_from_slice(&f);
            self.push(topic.clone(), &key, &pt, "ops");
        }
    }

    /// Everything time-driven: publishing local edits, heads after opening, repairs that
    /// are due, presence and cursors. Call often (every 100-250 ms).
    pub fn tick(&mut self, now: u64) {
        let ids: Vec<String> = self.docs.keys().cloned().collect();
        for id in ids {
            // repairs (also for drafts in the background)
            let due: Vec<Repair> = {
                let d = self.docs.get_mut(&id).unwrap();
                let (due, keep): (Vec<Repair>, Vec<Repair>) = std::mem::take(&mut d.repairs).into_iter().partition(|r| r.due <= now);
                d.repairs = keep;
                due
            };
            for r in due {
                let d = &self.docs[&id];
                let nodes: Vec<HashNode> = d
                    .web
                    .deltas_for(&r.clock)
                    .into_iter()
                    .flat_map(|(_, _, n)| n)
                    .filter(|n| !r.seen_since.contains(&n.id()))
                    .collect();
                if nodes.is_empty() {
                    continue;
                }
                let frames = d.delta_frames(nodes);
                let (topic, key) = (d.ops_topic.clone(), d.key);
                for f in frames {
                    let mut pt = vec![TAG_OPS];
                    pt.extend_from_slice(&f);
                    self.push(topic.clone(), &key, &pt, "repair");
                }
            }
            // Edits made here always go out (only the open draft is edited, but never let a
            // missed "open" keep them back); heads, presence and cursors only while open.
            self.flush_local(&id, now, false);
            if !self.docs[&id].active {
                continue;
            }
            let (heads_due, want, presence_due, cursor_due) = {
                let d = &self.docs[&id];
                let heads_due = d.heads_sent < HEADS_AFTER.len() && now >= d.activated_at + HEADS_AFTER[d.heads_sent];
                let want = d.want_heads && now >= d.last_heads + 500;
                let presence_due = now >= d.last_presence + PRESENCE_EVERY || d.presence_reply.is_some_and(|t| now >= t);
                let cursor_due = d.cursor_pending.is_some() && now >= d.last_cursor_sent + CURSOR_EVERY;
                (heads_due, want, presence_due, cursor_due)
            };
            let burst_due = {
                let d = &self.docs[&id];
                d.heads_after_burst.is_some_and(|t| now >= t && now >= d.last_publish + 3_000)
            };
            if heads_due {
                self.docs.get_mut(&id).unwrap().heads_sent += 1;
                self.send_heads(&id, now);
            } else if want || burst_due {
                self.send_heads(&id, now);
            }
            if burst_due {
                self.docs.get_mut(&id).unwrap().heads_after_burst = None;
            }
            if presence_due {
                let d = self.docs.get_mut(&id).unwrap();
                d.last_presence = now;
                d.presence_reply = None;
                self.eph(&id, "ping", now);
            }
            if cursor_due {
                self.eph(&id, "cursor", now);
                let d = self.docs.get_mut(&id).unwrap();
                d.cursor_pending = None;
                d.last_cursor_sent = now;
            }
            // peers gone quiet
            let d = self.docs.get_mut(&id).unwrap();
            for p in d.peers.values_mut() {
                if p.online && now > p.last_seen + 2 * PRESENCE_EVERY {
                    p.online = false;
                }
            }
        }
    }

    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    /// The content topics to listen on.
    pub fn topics(&self) -> Vec<String> {
        self.docs.values().flat_map(|d| [d.ops_topic.clone(), d.eph_topic.clone()]).collect()
    }

    /// A message from the network. Returns the shared draft it changed (its text), if any.
    pub fn receive(&mut self, topic: &str, sealed: &[u8], now: u64) -> Option<String> {
        let id = self.docs.iter().find(|(_, d)| d.ops_topic == topic || d.eph_topic == topic).map(|(k, _)| k.clone())?;
        let key = self.docs[&id].key;
        let pt = open(&key, sealed)?;
        match pt.first()? {
            &TAG_OPS => {
                *self.received.entry("ops").or_default() += 1;
                let d = self.docs.get_mut(&id).unwrap();
                let before = d.text();
                let ids = match d.apply_delta(&pt[1..]) {
                    Ok(ids) => ids,
                    Err(e) => {
                        eprintln!("fileverse-collab: bad ops frame: {e}");
                        return None;
                    }
                };
                eprintln!("fileverse-collab: ops in: {} ops, orphans now {}, text {} -> {} chars", ids.len(), d.has_orphans(), before.chars().count(), d.text().chars().count());
                for r in d.repairs.iter_mut() {
                    r.seen_since.extend(ids.iter().copied());
                }
                d.from_others.extend(ids.iter().copied());
                if d.has_orphans() && d.active {
                    d.want_heads = true;
                }
                let changed = d.text() != before;
                if changed {
                    d.dirty = true;
                }
                changed.then_some(id)
            }
            &TAG_HEADS => {
                *self.received.entry("heads").or_default() += 1;
                if pt.len() < 9 {
                    return None;
                }
                let theirs = decode_frontier(&pt[9..]).ok()?;
                let delay = self.rand(50, 500);
                let d = self.docs.get_mut(&id).unwrap();
                let they_lack = !d.web.deltas_for(&theirs).is_empty();
                let same = |a: &hashseq::HashWebClock, b: &hashseq::HashWebClock| {
                    a.objects().map(|(k, c)| (*k, c.0.clone())).collect::<Vec<_>>() == b.objects().map(|(k, c)| (*k, c.0.clone())).collect::<Vec<_>>()
                };
                if they_lack && !d.repairs.iter().any(|r| same(&r.clock, &theirs)) {
                    d.repairs.push(Repair { due: now + delay, clock: theirs.clone(), seen_since: HashSet::new() });
                }
                // tips we do not hold: we are behind, ask (only for the open draft)
                let seq = d.web.seq(&d.obj).expect("seq");
                let we_lack = theirs.objects().any(|(_, c)| c.0.iter().any(|t| !seq.contains_node(t)));
                if we_lack && d.active {
                    d.want_heads = true;
                }
                None
            }
            &TAG_EPH => {
                let e: Eph = serde_json::from_slice(&pt[1..]).ok()?;
                if e.a == self.me {
                    return None;
                }
                *self.received.entry(if e.k == "cursor" { "cursor" } else { "presence" }).or_default() += 1;
                let delay = self.rand(100, 1_000);
                let d = self.docs.get_mut(&id).unwrap();
                let new_here = !d.peers.get(&e.a).is_some_and(|p| p.online);
                // someone new here (their hello, or the first we hear of them): say we are here too
                if e.k != "bye" && new_here && d.active && d.presence_reply.is_none() {
                    d.presence_reply = Some(now + delay);
                }
                let p = d.peers.entry(e.a.clone()).or_default();
                p.name = e.n.chars().take(40).collect();
                p.color = if e.c.len() == 7 && e.c.starts_with('#') { e.c } else { color_of(&e.a) };
                p.last_seen = now;
                p.online = e.k != "bye";
                if e.k == "cursor" {
                    p.caret = e.caret.as_deref().and_then(hex_id_opt);
                    p.anchor = e.anchor.as_deref().and_then(hex_id_opt);
                }
                if e.k == "bye" {
                    p.caret = None;
                    p.anchor = None;
                }
                Some(id).filter(|_| false)
            }
            _ => None,
        }
    }

    /// The state the view shows for one shared draft (text offsets in UTF-16, as Qt counts).
    pub fn state(&mut self, id: &str) -> Option<Value> {
        let d = self.docs.get_mut(id)?;
        let version = d.current_version();
        let text = d.text();
        let utf16 = |chars: usize| -> usize { text.chars().take(chars).map(char::len_utf16).sum() };
        let peers: Vec<Value> = d
            .cursors()
            .into_iter()
            .map(|(k, p, caret, anchor)| {
                json!({ "id": k, "name": p.name, "color": p.color, "online": p.online,
                        "caret": caret.map(&utf16), "anchor": anchor.map(&utf16) })
            })
            .collect();
        Some(json!({
            "id": id, "version": version.to_string(), "text": text, "peers": peers,
            "active": d.active, "pending": d.has_orphans(),
        }))
    }
}

/// A stable colour for an author id.
pub fn color_of(author: &str) -> String {
    const PALETTE: [&str; 8] = ["#E8743B", "#19A979", "#945ECF", "#13A4B4", "#ED4A7B", "#6C8893", "#D6BA00", "#5899DA"];
    let h = Sha256::digest(author.as_bytes());
    PALETTE[(h[0] as usize) % PALETTE.len()].to_string()
}

/// The span where two texts differ: (start, end in old, end in new), by common prefix and
/// suffix. Typing between two views of the text is one span.
fn diff_span(old: &[char], new: &[char]) -> (usize, usize, usize) {
    let mut start = 0;
    while start < old.len() && start < new.len() && old[start] == new[start] {
        start += 1;
    }
    let (mut oe, mut ne) = (old.len(), new.len());
    while oe > start && ne > start && old[oe - 1] == new[ne - 1] {
        oe -= 1;
        ne -= 1;
    }
    (start, oe, ne)
}

/// UTF-16 offset (Qt) into `text` to a char offset.
pub fn char_offset(text: &str, utf16: usize) -> usize {
    let mut n = 0;
    for (i, c) in text.chars().enumerate() {
        if n >= utf16 {
            return i;
        }
        n += c.len_utf16();
    }
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peers on one in-memory relay: everything sent reaches every other peer subscribed.
    fn deliver(from: &mut Collab, to: &mut [&mut Collab], now: u64) -> usize {
        let out = from.take_outbox();
        for o in &out {
            for peer in to.iter_mut() {
                peer.receive(&o.topic, &o.sealed, now);
            }
        }
        out.len()
    }

    fn run(peers: &mut [Collab], from_ms: u64, to_ms: u64) {
        let mut now = from_ms;
        while now <= to_ms {
            for i in 0..peers.len() {
                peers[i].tick(now);
            }
            for i in 0..peers.len() {
                let (left, rest) = peers.split_at_mut(i);
                let (me, right) = rest.split_first_mut().unwrap();
                let mut others: Vec<&mut Collab> = left.iter_mut().chain(right.iter_mut()).collect();
                deliver(me, &mut others, now);
            }
            now += 50;
        }
    }

    const T: [u8; 32] = [7u8; 32];

    #[test]
    fn sealed_frames_open_only_with_the_key() {
        let k = derive("key", &T);
        let s = seal(&k, b"hello");
        assert_eq!(open(&k, &s).as_deref(), Some(&b"hello"[..]));
        assert_eq!(seal(&k, b"hello"), s, "deterministic: repeats dedup");
        assert!(open(&derive("key", &[8u8; 32]), &s).is_none());
        let (ops, eph) = topics(&T);
        assert!(ops.starts_with("/fileverse/1/") && ops.ends_with("-ops/bin") && !ops.contains(&hex::encode(T)));
        assert_ne!(ops, eph);
        assert_eq!(parse_id(&hex::encode(T)).unwrap(), T);
        assert!(parse_id("abc").is_err());
    }

    #[test]
    fn two_editors_converge_and_a_late_joiner_catches_up() {
        let mut a = Collab::new("aa".into(), "Alice".into(), 1);
        let mut b = Collab::new("bb".into(), "Bob".into(), 2);
        let id = hex::encode(T);
        a.share(T, "# Plan\n\nhello\n");
        a.set_active(Some(&id), 0);
        // Bob joins with nothing: his heads bring Alice's repair
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 2_000);
        assert_eq!(peers[1].docs[&id].text(), "# Plan\n\nhello\n");

        // both type at once, each from the version they saw
        let va = peers[0].docs.get_mut(&id).unwrap().current_version();
        let vb = peers[1].docs.get_mut(&id).unwrap().current_version();
        peers[0].docs.get_mut(&id).unwrap().set_text(Some(va), "# Plan for today\n\nhello\n").unwrap();
        peers[1].docs.get_mut(&id).unwrap().set_text(Some(vb), "# Plan\n\nhello world\n").unwrap();
        run(&mut peers, 2_050, 6_000);
        let ta = peers[0].docs[&id].text();
        assert_eq!(ta, peers[1].docs[&id].text());
        assert_eq!(ta, "# Plan for today\n\nhello world\n");

        // a third peer opens it later and catches up from either
        let mut c = Collab::new("cc".into(), "Carol".into(), 3);
        c.load(T, None).unwrap();
        c.set_active(Some(&id), 10_000);
        peers.push(c);
        run(&mut peers, 10_000, 12_000);
        assert_eq!(peers[2].docs[&id].text(), ta);
        // presence: Carol sees both
        let names: Vec<String> = peers[2].docs[&id].peers.values().filter(|p| p.online).map(|p| p.name.clone()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn an_edit_on_a_stale_version_is_rebased() {
        let mut a = Collab::new("aa".into(), "A".into(), 1);
        let d = a.share(T, "one two three");
        let v1 = d.current_version();
        // someone else's change arrives (simulated locally) before the view's edit
        d.set_text(None, "zero one two three").unwrap();
        let _v2 = d.current_version();
        // the view edited version 1: replaced "two" with "2"
        d.set_text(Some(v1), "one 2 three").unwrap();
        assert_eq!(d.text(), "zero one 2 three");
        // and inserted at the very start, from version 1
        d.set_text(Some(v1), ">one two three").unwrap();
        // ">" was typed before "one" in the version the view had: it stays before "one"
        assert_eq!(d.text(), "zero >one 2 three");
    }

    #[test]
    fn background_drafts_send_nothing_but_their_edits() {
        let mut a = Collab::new("aa".into(), "A".into(), 1);
        a.share(T, "x");
        let id = hex::encode(T);
        for now in (0..300_000).step_by(100) {
            a.tick(now);
        }
        assert!(a.take_outbox().is_empty(), "idle in the background: silent");
        // an edit made while the draft is not marked open still goes out (once), nothing else
        a.docs.get_mut(&id).unwrap().set_text(None, "xy").unwrap();
        for now in (300_000..600_000).step_by(100) {
            a.tick(now);
        }
        let kinds: Vec<&str> = a.take_outbox().iter().map(|o| o.kind).collect();
        assert_eq!(kinds, ["ops"]);
        a.tick(600_000);
        a.set_active(Some(&id), 300_000);
        a.tick(300_000);
        a.tick(300_600);
        let kinds: Vec<&str> = a.take_outbox().iter().map(|o| o.kind).collect();
        // opened: presence and heads (the edit already went out)
        assert!(kinds.contains(&"presence") && kinds.contains(&"heads") && !kinds.contains(&"ops"), "{kinds:?}");
    }

    /// vpavlin's report on 0.12.0: the sharer's edits did not reach the joiner (the sharer's
    /// draft was never marked open). Edits must flow both ways whatever is marked open.
    #[test]
    fn the_sharers_edits_reach_the_joiner_even_if_not_marked_open() {
        let mut a = Collab::new("aa".into(), "Sharer".into(), 1);
        let mut b = Collab::new("bb".into(), "Joiner".into(), 2);
        let id = hex::encode(T);
        a.share(T, "start\n");
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 2_000);
        assert_eq!(peers[1].docs[&id].text(), "start\n");
        peers[0].docs.get_mut(&id).unwrap().set_text(None, "start\nSharer wrote this\n").unwrap();
        peers[1].docs.get_mut(&id).unwrap().set_text(None, "start\nJoiner wrote that\n").unwrap();
        run(&mut peers, 2_050, 6_000);
        let (ta, tb) = (peers[0].docs[&id].text(), peers[1].docs[&id].text());
        assert_eq!(ta, tb);
        assert!(ta.contains("Sharer wrote this\n") && ta.contains("Joiner wrote that\n"), "{ta:?}");
    }

    /// Relays lose messages: an edit whose only message was lost still arrives (the heads
    /// sent after the burst make the peer ask), and received operations are not echoed.
    #[test]
    fn a_lost_edit_is_recovered_and_nothing_is_echoed() {
        let mut a = Collab::new("aa".into(), "A".into(), 1);
        let mut b = Collab::new("bb".into(), "B".into(), 2);
        let id = hex::encode(T);
        a.share(T, "base\n");
        a.set_active(Some(&id), 0);
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 40_000);
        assert_eq!(peers[1].docs[&id].text(), "base\n");
        let echoes = peers[1].sent.get("ops").copied().unwrap_or(0);
        assert_eq!(echoes, 0, "B made no edits: it sends no ops");
        // A types; its ops message is lost on the way
        peers[0].docs.get_mut(&id).unwrap().set_text(None, "base\nlost line\n").unwrap();
        let mut now = 40_050;
        while peers[0].sent.get("ops").copied().unwrap_or(0) == 0 {
            peers[0].tick(now);
            now += 50;
        }
        let dropped = peers[0].take_outbox();
        assert!(dropped.iter().any(|o| o.kind == "ops"));
        run(&mut peers, now, now + 10_000);
        assert_eq!(peers[1].docs[&id].text(), "base\nlost line\n");
    }

    /// HashSeq ids are content hashes: the same characters typed at the same place from the
    /// same state by two people are the same operations, and merge into one copy.
    #[test]
    fn identical_concurrent_typing_collapses_in_hashseq() {
        let mut a = Collab::new("aa".into(), "A".into(), 1);
        let mut b = Collab::new("bb".into(), "B".into(), 2);
        let id = hex::encode(T);
        a.share(T, "start\n");
        a.set_active(Some(&id), 0);
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 2_000);
        peers[0].docs.get_mut(&id).unwrap().set_text(None, "start\nfrom the A\n").unwrap();
        peers[1].docs.get_mut(&id).unwrap().set_text(None, "start\nfrom the B\n").unwrap();
        run(&mut peers, 2_050, 6_000);
        let t = peers[0].docs[&id].text();
        assert_eq!(t.matches("from the ").count(), 1, "{t:?}");
    }

    #[test]
    fn cursors_follow_the_text() {
        let mut a = Collab::new("aa".into(), "Alice".into(), 1);
        let mut b = Collab::new("bb".into(), "Bob".into(), 2);
        let id = hex::encode(T);
        a.share(T, "hello world");
        a.set_active(Some(&id), 0);
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 2_000);
        // Bob puts his caret after "hello"
        let vb = peers[1].docs.get_mut(&id).unwrap().current_version();
        peers[1].docs.get_mut(&id).unwrap().set_cursor(Some(vb), 5, 5);
        run(&mut peers, 2_050, 4_000);
        // Alice inserts before it: Bob's caret moves with the text
        peers[0].docs.get_mut(&id).unwrap().set_text(None, ">> hello world").unwrap();
        run(&mut peers, 4_050, 7_000);
        let st = peers[0].state(&id).unwrap();
        let bob = st["peers"].as_array().unwrap().iter().find(|p| p["name"] == "Bob").unwrap().clone();
        assert_eq!(bob["caret"], 8, "{st}");
    }

    #[test]
    fn a_big_document_crosses_in_several_messages() {
        let mut a = Collab::new("aa".into(), "A".into(), 1);
        let mut b = Collab::new("bb".into(), "B".into(), 2);
        let id = hex::encode(T);
        let text: String = (0..150_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        a.share(T, &text);
        b.load(T, None).unwrap();
        b.set_active(Some(&id), 0);
        let mut peers = vec![a, b];
        run(&mut peers, 0, 3_000);
        assert_eq!(peers[1].docs[&id].text().len(), text.len());
        assert!(peers[0].sent.get("repair").copied().unwrap_or(0) >= 1);
    }
}
