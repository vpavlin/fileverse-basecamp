// Fixtures for "your own files": two portals opened by a backup key, made with the
// primitives Fileverse uses (WebCrypto RSA-OAEP, @noble ECIES/AES-GCM, js-base64, yjs).
//   node make-own-fixtures.mjs > fileverse_core/rust-lib/tests/fixtures/own.json
import { webcrypto as wc } from "node:crypto";
import { gcm } from "@noble/ciphers/aes.js";
import { secp256k1 } from "@noble/curves/secp256k1.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { argon2id } from "@noble/hashes/argon2.js";
import { randomBytes } from "@noble/hashes/utils.js";
import nacl from "tweetnacl";
import { fromUint8Array as b64 } from "js-base64";
import * as Y from "yjs";

const enc = new TextEncoder();
const SEP = "__n__";
const blocks = {};
function rawCid(bytes) {
  const mh = new Uint8Array([0x01, 0x55, 0x12, 0x20, ...sha256(bytes)]);
  const A = "abcdefghijklmnopqrstuvwxyz234567";
  let s = "b", buf = 0, bits = 0;
  for (const x of mh) { buf = (buf << 8) | x; bits += 8; while (bits >= 5) { bits -= 5; s += A[(buf >> bits) & 31]; } buf &= (1 << bits) - 1; }
  if (bits > 0) s += A[(buf << (5 - bits)) & 31];
  return s;
}
const put = (b) => { const c = rawCid(b); blocks[c] = b64(b); return c; };
const putJson = (o) => put(enc.encode(JSON.stringify(o)));
function ecies(pub, msg) {
  const eph = secp256k1.utils.randomSecretKey(), ephPub = secp256k1.getPublicKey(eph);
  const key = hkdf(sha256, secp256k1.getSharedSecret(eph, pub), ephPub, enc.encode("ECIES-AES256-GCM-SHA256"), 32);
  const nonce = randomBytes(12), ct = gcm(key, nonce).encrypt(msg);
  return [b64(ephPub), b64(nonce), b64(ct.slice(0, -16)), b64(ct.slice(-16))].join(SEP);
}
const docUpdate = (text) => {
  const d = new Y.Doc(); const p = new Y.XmlElement("paragraph"); const t = new Y.XmlText(); t.insert(0, text); p.insert(0, [t]);
  d.getXmlFragment("default").insert(0, [p]); return Y.encodeStateAsUpdate(d);
};
const sheetUpdate = (id) => {
  const d = new Y.Doc(); const s = new Y.Map(); d.getArray(id).push([s]); s.set("name", "S1");
  const cd = new Y.Map(); s.set("celldata", cd); cd.set("0_0", { r: 0, c: 0, v: { v: "own sheet" } }); return Y.encodeStateAsUpdate(d);
};
const titleCt = (key, t) => { const n = randomBytes(12); return b64(new Uint8Array([...n, ...gcm(key, n).encrypt(enc.encode(t))])); };

// ── current (EC) portal ──
const ecPriv = secp256k1.utils.randomSecretKey(), ecPub = secp256k1.getPublicKey(ecPriv);
const ecFiles = [];
function v3File(kind, title, lockName, opts = {}) {
  const key = randomBytes(32), iv = randomBytes(12);
  const file = kind === "dsheet" ? b64(sheetUpdate("sid")) : b64(docUpdate(title + " body"));
  const ct = gcm(key, iv).encrypt(enc.encode(JSON.stringify({ file, source: kind })));
  const meta = { title: titleCt(key, title), kind, version: "4", ddocId: "d" + ecFiles.length, nonce: b64(randomBytes(24)) };
  if (kind === "dsheet") meta.dsheetId = "sid";
  meta[lockName] = { lockedFileKey: opts.pq ? "FVPQ1" + SEP + b64(randomBytes(64)) : ecies(ecPub, key) };
  if (opts.deleted) meta.isDeleted = true;
  ecFiles.push({ appFileId: meta.ddocId, fileType: kind === "dsheet" ? 3 : 2, metadata: putJson(meta), content: put(new Uint8Array([...ct, ...iv])), gate: "" });
}
v3File("ddoc", "My notes", "ownerLock");
v3File("dsheet", "My sheet", "appLock");
v3File("ddoc", "Quantum", "ownerLock", { pq: true });
v3File("ddoc", "Gone", "ownerLock", { deleted: true });

// ── legacy (RSA) portal ──
const kp = await wc.subtle.generateKey({ name: "RSA-OAEP", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["encrypt", "decrypt"]);
const pkcs8 = b64(new Uint8Array(await wc.subtle.exportKey("pkcs8", kp.privateKey)));
const legacyFiles = [];
{
  const key = randomBytes(32), iv = randomBytes(12);
  const ct = gcm(key, iv).encrypt(enc.encode(JSON.stringify({ file: b64(docUpdate("Legacy body")), source: "ddoc" })));
  const fileKey = { key: b64(key), iv: b64(iv), authTag: b64(ct.slice(-16)) };
  const locked = b64(new Uint8Array(await wc.subtle.encrypt({ name: "RSA-OAEP" }, kp.publicKey, enc.encode(JSON.stringify(fileKey)))));
  const title = b64(gcm(key, iv).encrypt(enc.encode("Old doc")));
  const meta = { title, version: "3", archVersion: "v3", ownerLock: { lockedFileKey: locked } };
  legacyFiles.push({ legacy: true, metadata: putJson(meta), content: put(ct.slice(0, -16)), gate: "" });
}
// Older (version "2") docs.fileverse.io files seal the title with the share-link
// secret (secretbox, metadata nonce), not the file key. Newer writes also put the
// link key (URL text) in ownerLock.lockedLinkKey; older ones do not.
for (const [i, withLinkKey] of [[1, false], [2, true]]) {
  const key = randomBytes(32), iv = randomBytes(12);
  const d = new Y.Doc(); const h = new Y.XmlElement("heading"); h.setAttribute("level", 1); const t = new Y.XmlText(); t.insert(0, "Heading of doc " + i); h.insert(0, [t]);
  const p = new Y.XmlElement("paragraph"); const pt = new Y.XmlText(); pt.insert(0, "Body " + i); p.insert(0, [pt]);
  d.getXmlFragment("default").insert(0, [h, p]);
  const ct = gcm(key, iv).encrypt(enc.encode(JSON.stringify({ file: b64(Y.encodeStateAsUpdate(d)), source: "fileverse_document" })));
  const fileKey = { key: b64(key), iv: b64(iv), authTag: b64(ct.slice(-16)) };
  const rsa = async (bytes) => b64(new Uint8Array(await wc.subtle.encrypt({ name: "RSA-OAEP" }, kp.publicKey, bytes)));
  const secret = randomBytes(32), nonce = randomBytes(24), ddocId = "legacyDoc" + i;
  const wrap = hkdf(sha256, argon2id(ddocId, nonce, { t: 2, m: 16384, p: 8, dkLen: 32 }), undefined, enc.encode("encryptionKey"), 32);
  const linkKey = b64(nacl.secretbox(secret, nonce, wrap), true);
  const ownerLock = { lockedFileKey: await rsa(enc.encode(JSON.stringify(fileKey))) };
  if (withLinkKey) ownerLock.lockedLinkKey = await rsa(enc.encode(linkKey));
  const title = b64(nacl.secretbox(enc.encode("Sealed title " + i), nonce, secret));
  const meta = { title, version: "2", ddocId, nonce: b64(nonce), ownerLock };
  legacyFiles.push({ legacy: true, metadata: putJson(meta), content: put(ct.slice(0, -16)), gate: "" });
}

const backup = {
  current: { portalAddress: "0x2222222222222222222222222222222222222222", ownerDid: "did:key:x", ownerSecret: "s", source: "docs.fileverse.io", appEncryptionKey: b64(ecPub), appDecryptionKey: b64(ecPriv) },
  legacy: { portalAddress: "0x3333333333333333333333333333333333333333", ownerPublicKey: "p", ownerPrivateKey: pkcs8, portalPublicKey: "x", portalPrivateKey: "x", memberPublicKey: "m", memberPrivateKey: "m", source: "docs.fileverse.io" },
};
console.log(JSON.stringify({ backup, portals: { "0x2222222222222222222222222222222222222222": ecFiles, "0x3333333333333333333333333333333333333333": legacyFiles }, blocks }, null, 1));
