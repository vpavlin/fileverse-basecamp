# logos-pin: a public cache for published documents

Status: **v1 frozen 2026-11-26** (message format and signing bytes). The hub is generic: any
app may use it, not just Fileverse. jimmy-crib/vpavlin builds and runs it on the VPS
(128.140.55.128); Fileverse is the first client.

## The problem

Publish encrypts a draft and puts its blocks on this device's IPFS node and Logos Storage node.
At home both sit behind NAT: nobody can dial them, so a `fvbc://` link opens only while the
publisher's Basecamp is online, and then only if a relay happens to carry it. A reachable node
that copies every published document makes links work after the publisher closes Basecamp, and
between any two networks.

## Shape

```
client (publisher)                         VPS
  publish: blocks -> own Storage/IPFS
  pin request --- Delivery /logos-pin/1/requests/json --->  pin hub (logosctl: delivery_module
                                                              + pin_hub core)
                                                              | REST (mesh only)
                                                              v
  <-- blocks fetched from the publisher (relay/direct) ---  Storage gw :8499 (+ Kubo pin)
  "kept" <--- Delivery /logos-pin/1/status/json (hub-signed) ---
readers: connect() to the hub's Storage peer, then fetch by CID
```

The hub learns what to keep in two ways, into one store:

1. **Signed pin request** (v1, now): the client sends the CIDs right after publishing.
2. **LEZ portal watcher** (later): once Fileverse publishes to the LEZ portal program, every
   file's CIDs are on chain. The hub follows registered portals and keeps what they add. The
   chain is then the allow-list; this survives the hub being offline at publish time and lets
   anyone run a second hub from the same list.

## Pin request (v1)

Content topic `/logos-pin/1/requests/json`. Payload: the JSON object, UTF-8.

```json
{
  "v": 1,
  "type": "pin",
  "id": "<16 random bytes, lowercase hex>",
  "app": "fileverse",
  "storage": ["<Logos Storage manifest CID>"],
  "ipfs": ["<IPFS root CID>"],
  "from": { "storagePeer": "<libp2p peer id>", "addrs": ["/ip4/.../tcp/..."] },
  "at": 1796000000000,
  "key": "<ed25519 public key, 32 bytes, lowercase hex>",
  "sig": "<ed25519 signature, 64 bytes, lowercase hex>"
}
```

- `type`: `"pin"`, or `"unpin"` (same fields; `storage`/`ipfs` name what to drop; only the key
  that pinned a CID can unpin it).
- At most **64** CIDs in `storage` and `ipfs` together (keeps a message well under Delivery's
  ~150 KB). Larger documents: several requests.
- `at`: milliseconds since the epoch.
- No size field: the hub reads each manifest's `datasetSize` and charges that.
- Only CIDs and addresses travel; the documents are encrypted before they leave the device.

### Signing bytes (v1)

```
sig = ed25519_sign(key, "logos-pin/v1\n" || JCS(message without "sig"))
```

JCS is RFC 8785 (keys sorted by UTF-16 code units, no whitespace, minimal escaping). v1 messages
hold only strings, integers, arrays and objects, so a serializer that sorts keys and writes no
spaces produces it. The verifier recomputes the bytes from the parsed message; it never signs
or checks the raw text as received. Status messages use the same rule with the hub's key and
the tag `"logos-pin/v1/status\n"`.

## Status (v1)

Content topic `/logos-pin/1/status/json`, signed by the hub (its public key is published in the
hub's README; clients check it before showing "kept"):

```json
{ "v": 1, "id": "<request id>", "state": "kept", "why": "", "kept": ["<cid>"],
  "until": 1798600000000, "at": 1796000300000, "key": "<hub key>", "sig": "..." }
```

`state`: `kept`, `refused` (with `why`: unknown key, quota, too many CIDs, stale), `failed`
(could not fetch within 30 minutes), or `queued` (only when the queue is long). Sent on changes
only, coalesced, no progress heartbeats: every message costs RLN budget. `until`: when the
blocks expire unless renewed (see Retention).

## Hub rules

- **Who:** an allow-list of keys, each with a quota in bytes per 30 days; refuse others.
- **Replay:** refuse `at` more than 10 minutes off; dedupe `id` per key.
- **Dialing:** use only public addresses from `from.addrs`. Drop RFC 1918, 100.64.0.0/10,
  198.18.0.0/15 (the VPS mesh), fc00::/7, link-local and loopback.
- **Fetching:** `fetch()` while the publisher is online, newest requests first, retrying for 30
  minutes. A CID is **kept** only when a local `downloadToUrl` writes the whole file and its size
  matches the manifest's `datasetSize`; `exists()` is true as soon as the manifest arrives.
- **IPFS:** the hub's Kubo pins the `ipfs` roots, so `fvbc://ipfs/` links resolve from it.
- **Retention:** the Storage node keeps blocks 30 days (default block TTL). A pin request for a
  CID already kept renews it; `until` says when it lapses. Clients renew what they still want.
- **Findability:** a Storage node announces only about 500 CIDs in the DHT, and a manifest fetch
  finds holders through the DHT only. Clients therefore `connect()` to the hub's Storage peer
  (its peer id and address are in its README) before fetching.

## Client side (Fileverse)

- Delivery directly through `delivery_module` 0.3.3 (not loam_core: the minimal-dependency rule
  for catalogue apps). `createNode {"mode":"Core","preset":"logos.test"}`; if another app made
  the node, join it (`getNodeInfo`). Then `start`, `subscribe` both topics, `send(topic,
  {"_bytes": base64(payload)})`. Received: `[hash, topic, {"_bytes": b64}, "live"|"history", ts]`.
  Async calls only; never call Delivery from inside its own event callback.
- RLN: on testnet, sending needs an RLN membership. Without one Fileverse says so and keeps the
  document local; nothing else changes.
- One ed25519 key per installation, in the module's data directory (mode 600); its public key in
  Settings, to send to the hub's owner for the allow-list.
- After Publish (and "Keep on the public cache" for older ones): send the request, keep this
  device's Storage node connected to the hub until a hub-signed `kept` or `failed`, and show it on
  the document. Renew before `until`.
- Readers `connect()` to the hub, then fetch as from any source: every block is hash-checked, so
  the hub can refuse or watch, never alter.
