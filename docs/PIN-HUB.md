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
- **IPFS: not kept** (Václav, 2026-11-26). The hub runs no Kubo: `ipfs` roots in a request are
  skipped and the status `why` says `ipfs not kept`; a request with only `ipfs` roots is refused
  with that reason. Clients send Logos Storage CIDs only.
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
- After Publish (and "Keep on the public cache" for older ones): send the Storage CIDs, keep this
  device's Storage node connected to the hub until a hub-signed `kept` or `failed`, and show it on
  the document. Renew before `until`.
- Readers `connect()` to the hub, then fetch as from any source: every block is hash-checked, so
  the hub can refuse or watch, never alter. This needs storage_module 3.0.3 (logos-storage PR
  #1540), whose connect() makes the peer a download source; Basecamp 0.3.2's bundled 3.0.2
  connects but never asks it. fileverse_core requires ~3.0.3.

## v1.1: the LNS tier (frozen 2026-11-26)

Anyone with an LNS name (jefdiesel/logos-names) may use the hub within small limits, without
being on the allow-list. v1 clients and hubs keep working: the field is optional.

- **Linking a key to a name.** The name's holder sets the text entry `logos-pin.key` to the
  installation's ed25519 public key, 64 lowercase hex. Several installations: several keys in
  the one value, separated by spaces or commas. Only the holder can set entries, so the entry
  is the proof (as WhisperBox's ADR 0019 does with `whisperbox.address`).
- **Request:** an optional `"name"`, already normalised as LNS spec section 2 (trim, lower
  case, drop `lns:`, then `@`, then a trailing `.lez`; 1-32 of `a-z0-9-`, not starting or
  ending with `-`). It is signed with the rest, like every field.
- **Hub:** a key on the allow-list is judged as in v1, whether or not a name comes with it.
  Otherwise, with a `name`: resolve it on the hub's configured network (below), require an
  active record whose `logos-pin.key` lists `key`, then apply the LNS tier:
  - each Storage dataset at most **1 MB** (`datasetSize` from the manifest, read before
    fetching);
  - per name: 50 MB and 100 datasets per 30 days (charged on real size, renewals too),
    shared by every key the name lists. The numbers are the hub's to change.
- **Refusals** (`state: "refused"`, `why`): `unknown key` (no name, not listed), `unknown name`,
  `name has no logos-pin.key`, `key not on name`, `too large`, `quota`, `name lookup failed`
  (the network could not be asked; try again later); v1's `stale` and `too many CIDs`. Partial
  results: `state: "kept"`, `kept` lists what was kept, and `why` joins the reasons for the rest
  with `"; "` (e.g. `"too large; ipfs not kept"`).
- **Live (2026-11-26):** Logos public LEZ testnet v0.3, RPC `https://testnet.lez.logos.co/`,
  registry `38jEfVV1RCyMzu1FGbYcCn5FqxCxKyY58VgzjruAh2gq` (hub.json `lns.rpc` / `registry` /
  `network`). Hub source and README: github.com/vpavlin/logos-pin.
- **Network: a hub setting, never in the message.** The hub's config names the LEZ RPC URL and
  the LNS registry program ID, so it can move between Logos's public LEZ testnet and the LNS
  Bridge test network (or mainnet) without a client change. The hub's README says which one it
  uses, so users register their names there. Answers for a name are cached ~10 minutes
  (negative ones too) and dropped when the network setting changes.
- **Cost on the hub:** a new name costs an Argon2id derivation (64 MiB, ~0.1 s) and an RPC read,
  so the hub derives only after a request's signature checks, at most one new name per key per
  minute, with a global cap.
- **Trust:** the hub believes the RPC it asks (LEZ reads carry no proof): its own node or one
  it trusts.
- **What it stops, and what not:** on testnet a name costs a refundable deposit in free faucet
  coins and ~20 minutes, so one person can hold many names. Names make abuse slower and
  per-name, not impossible; each request also needs an RLN membership on Delivery.
