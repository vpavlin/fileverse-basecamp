// Builds offline test fixtures in the v3 (ddocs.new / dsheets.new) formats, using
// the same JS primitives Fileverse uses (@noble/*, tweetnacl, js-base64, yjs), so
// the Rust engine is checked against the reference implementations byte for byte.
// Run from a directory with those packages installed:
//   node make-v3-fixtures.mjs > fileverse_core/rust-lib/tests/fixtures/v3.json
import { gcm, cbc } from "@noble/ciphers/aes.js";
import { secp256k1 } from "@noble/curves/secp256k1.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { pbkdf2 } from "@noble/hashes/pbkdf2.js";
import { argon2id } from "@noble/hashes/argon2.js";
import { randomBytes } from "@noble/hashes/utils.js";
import nacl from "tweetnacl";
import { fromUint8Array as b64, toUint8Array } from "js-base64";
import * as Y from "yjs";

const enc = new TextEncoder();
const SEP = "__n__";
const blocks = {}; // raw CID -> base64 bytes

// CIDv1, raw codec, sha2-256, base32 — what Fileverse's v3 uploads are.
function rawCid(bytes) {
  const mh = new Uint8Array([0x01, 0x55, 0x12, 0x20, ...sha256(bytes)]);
  const A = "abcdefghijklmnopqrstuvwxyz234567";
  let s = "b", buf = 0, bits = 0;
  for (const x of mh) { buf = (buf << 8) | x; bits += 8; while (bits >= 5) { bits -= 5; s += A[(buf >> bits) & 31]; } buf &= (1 << bits) - 1; }
  if (bits > 0) s += A[(buf << (5 - bits)) & 31];
  return s;
}
const put = (bytes) => { const c = rawCid(bytes); blocks[c] = b64(bytes); return c; };
const putJson = (o) => put(enc.encode(JSON.stringify(o)));

function eciesEncrypt(pub, msg) {
  const eph = secp256k1.utils.randomSecretKey();
  const ephPub = secp256k1.getPublicKey(eph);
  const shared = secp256k1.getSharedSecret(eph, pub);
  const key = hkdf(sha256, shared, ephPub, enc.encode("ECIES-AES256-GCM-SHA256"), 32);
  const nonce = randomBytes(12);
  const ct = gcm(key, nonce).encrypt(msg);
  return [b64(ephPub), b64(nonce), b64(ct.slice(0, -16)), b64(ct.slice(-16))].join(SEP);
}

function encryptContent(obj) {
  const key = randomBytes(32), iv = randomBytes(12);
  const ct = gcm(key, iv).encrypt(enc.encode(JSON.stringify(obj)));
  return { key, blob: new Uint8Array([...ct, ...iv]) }; // ct already ends with the tag
}
const encryptTitle = (key, title) => { const n = randomBytes(12); return b64(new Uint8Array([...n, ...gcm(key, n).encrypt(enc.encode(title))])); };

function docUpdate() {
  const d = new Y.Doc();
  const f = d.getXmlFragment("default");
  const h = new Y.XmlElement("heading"); h.setAttribute("level", 2);
  const ht = new Y.XmlText(); ht.insert(0, "Hello <script>"); h.insert(0, [ht]);
  const p = new Y.XmlElement("paragraph");
  const pt = new Y.XmlText(); pt.insert(0, "plain "); pt.insert(6, "bold", { bold: true }); pt.insert(10, " link", { link: { href: "https://logos.co" } });
  p.insert(0, [pt]);
  const ul = new Y.XmlElement("bulletList"), li = new Y.XmlElement("listItem"), lp = new Y.XmlElement("paragraph"), lt = new Y.XmlText();
  lt.insert(0, "item one"); lp.insert(0, [lt]); li.insert(0, [lp]); ul.insert(0, [li]);
  const db = new Y.XmlElement("dBlock"); db.insert(0, [p]);
  f.insert(0, [h, db, ul]);
  // a second tab
  d.getArray("tabs_order").push(["default", "t2"]);
  d.getMap("tabs_name_registry").set("default", "Intro");
  d.getMap("tabs_name_registry").set("t2", "Second");
  const p2 = new Y.XmlElement("paragraph"), t2 = new Y.XmlText(); t2.insert(0, "on tab two"); p2.insert(0, [t2]);
  d.getXmlFragment("t2").insert(0, [p2]);
  return Y.encodeStateAsUpdate(d);
}

function sheetUpdate(id) {
  const d = new Y.Doc();
  const arr = d.getArray(id);
  const s = new Y.Map();
  arr.push([s]);
  s.set("name", "Budget"); s.set("order", 0);
  const cd = new Y.Map(); s.set("celldata", cd);
  cd.set("0_0", { r: 0, c: 0, v: { v: "Item", m: "Item" } });
  cd.set("0_1", { r: 0, c: 1, v: { v: "Cost", m: "Cost" } });
  cd.set("1_0", { r: 1, c: 0, v: { v: "Coffee" } });
  cd.set("1_1", { r: 1, c: 1, v: { v: 4.5, m: "$4.50", f: "=2+2.5" } });
  return Y.encodeStateAsUpdate(d);
}

const cases = [];

// 1. ddoc, web-app style gate: lockedFileKey = secretbox(fileKey bytes) with the raw link secret.
{
  const update = docUpdate();
  const { key, blob } = encryptContent({ file: b64(update), source: "ddoc" });
  const secret = randomBytes(32), n = randomBytes(24);
  const gate = { lockedFileKey: b64(n, true) + SEP + b64(nacl.secretbox(key, n, secret), true) };
  const meta = { title: encryptTitle(key, "Doc via raw link"), kind: "ddoc", ddocId: "ddocA", nonce: b64(randomBytes(24)), version: "4", linkKeyType: "raw" };
  cases.push({ name: "ddoc-raw-secretbox", kind: "ddoc", title: "Doc via raw link",
    record: { appFileId: "ddocA", fileType: 2, metadata: putJson(meta), content: put(blob), gate: putJson(gate) },
    linkKey: b64(secret, true) });
}

// 2. ddoc, Fileverse API style: link key = secretbox(secret) under Argon2id(ddocId, nonce, 16 MiB);
//    gate = buildLinklock (PBKDF2 + AES-CBC with a secretbox-wrapped ikm).
{
  const update = docUpdate();
  const { key, blob } = encryptContent({ file: b64(update), source: "ddoc" });
  const secret = randomBytes(32), nonce = randomBytes(24), ddocId = "apiDoc42";
  const wrap = hkdf(sha256, argon2id(ddocId, nonce, { t: 2, m: 16384, p: 8, dkLen: 32 }), undefined, enc.encode("encryptionKey"), 32);
  const linkKey = b64(nacl.secretbox(secret, nonce, wrap), true);
  const ikm = randomBytes(32), salt = randomBytes(32);
  const dk = pbkdf2(sha256, ikm, salt, { c: 32, dkLen: 32 });
  const iv = randomBytes(16);
  const lockedFileKey = b64(iv) + SEP + b64(cbc(dk, iv).encrypt(key));
  const bn = randomBytes(24);
  const keyMaterial = b64(salt) + SEP + b64(bn, true) + SEP + b64(nacl.secretbox(ikm, bn, secret), true);
  const meta = { title: encryptTitle(key, "Doc via API link"), ddocId, nonce: b64(nonce), version: "4", sourceApp: "fileverse-api" };
  cases.push({ name: "ddoc-api-argon-cbc", kind: "ddoc", title: "Doc via API link",
    record: { appFileId: ddocId, fileType: 2, metadata: putJson(meta), content: put(blob), gate: putJson({ lockedFileKey, keyMaterial, lockedChatKey: "x" }) },
    linkKey });
}

// 3. dsheet with gz1: compressed content and a raw link.
{
  const zlib = await import("node:zlib");
  const id = "sheet-xyz";
  const gz = "gz1:" + b64(zlib.gzipSync(sheetUpdate(id)));
  const { key, blob } = encryptContent({ file: gz, source: "dsheet" });
  const secret = randomBytes(32), n = randomBytes(24);
  const gate = { lockedFileKey: b64(n) + SEP + b64(nacl.secretbox(key, n, secret)) };
  const meta = { title: encryptTitle(key, "Sheet"), kind: "dsheet", dsheetId: id, ddocId: id, nonce: b64(randomBytes(24)), version: "4" };
  cases.push({ name: "dsheet-gz1", kind: "dsheet", title: "Sheet",
    record: { appFileId: id, fileType: 3, metadata: putJson(meta), content: put(blob), gate: putJson(gate) },
    linkKey: b64(secret, true) });
}

// 4. A bare ECIES vector (the appLock/ownerLock format) for the crypto unit test.
const priv = secp256k1.utils.randomSecretKey();
const eciesMsg = randomBytes(32);
const ecies = { privateKey: b64(priv), ciphertext: eciesEncrypt(secp256k1.getPublicKey(priv), eciesMsg), plaintext: b64(eciesMsg) };

console.log(JSON.stringify({ cases, blocks, ecies }, null, 1));
