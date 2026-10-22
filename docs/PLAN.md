# From viewer to self-sovereign: the incremental plan

Each stage is useful on its own and removes one more dependency on Fileverse Inc.
The background research (what Fileverse runs, what is open, what is on chain) is in
`../REPORT.md` one directory up.

## What Basecamp already offers (official catalog, 2026-10-22)

| Need | Module | Status for us |
|---|---|---|
| Gnosis JSON-RPC | `eth_rpc_module` 0.1.0 | **Used.** Per-chain endpoint, SOCKS/Tor, fail-closed. Gnosis is not built in; we seed chain 100 with `ensure_chain_config` (only absent fields). |
| Verified chain reads | `verified_proxy_module` 0.1.4 (nimbus light client) | Ethereum mainnet/Sepolia/Hoodi + OP only. **No Gnosis.** |
| EVM keys / signing / sending | `keystore_module`, `tx_sender_module`, `fee_module`, `evm_signer_ui` | Ready for the write path (stage 4): the user pays xDAI gas, no Pimlico. |
| Content storage | `storage_module` 3.0 (Logos Storage, Codex-derived) | Own protocol and CIDs; **does not speak IPFS**. Usable as a mirror, not as an IPFS replacement for Fileverse interop. |
| IPFS | none | **Gap.** Today: HTTP gateways, every block hash-checked. `libp2p_module` has Kademlia (`kadGetProviders`) and a custom-protocol bridge: a possible base for a native IPFS (bitswap) client. |
| Messaging / sync | `delivery_module`, `loam_core` | For live collaboration (stage 5). |
| Hardware identity | `keycard` | For key custody (stages 2, 6). |
| Web rendering | Basecamp 0.3.2's Linux bundle ships QtWebEngine + QtWebChannel QML modules (308 MB Chromium core) and QtWebView; nothing uses them and the build notes call WebView dead weight | Could host Fileverse's (AGPL) React editor in a view for stage 4. Not verified that the plugin sandbox allows `import QtWebEngine`; needs a Basecamp-team decision either way. |

## Stage 0: read share links (this repo, v0.1)

Done. Share link -> Gnosis (eth_rpc_module) -> hash-checked IPFS blocks -> local
decryption -> native rendering. No Fileverse API, gate, collab server or indexer.

Remaining dependencies: the user's Gnosis RPC (trusted for CIDs); someone keeping the
blocks pinned (in practice Fileverse's Pinata account); any reachable gateway.

## Stage 1: keep what you can read (availability)

- Done: verified blocks are cached locally by CID; a cached document reopens with no
  gateway at all.
- Done (0.2.0): **"Keep a copy"** copies an opened file's verified blocks into the
  user's own Kubo node (`block/put`, then `pin/add`), and that node's gateway is read
  from first when it answers. Tested: after keeping, with the cache wiped and every
  public gateway dead, the document opens from the user's node alone.
- Next: the same via the IPFS Pinning Service API (remote pinning the user pays for),
  and an `ipfs_module` for Basecamp so users need not run Kubo themselves (see below).
- Next: **mirror to Logos Storage**: upload the ciphertext blocks to `storage_module`
  and keep a CID -> Logos-CID sidecar (two-id rule, see the `logos-storage` skill).
  Readers who have the sidecar fetch from Logos peers when IPFS has nothing.
- Exit test: a document opened once is still readable after Fileverse and Pinata stop
  serving it.

## Stage 2: your own files (walk away into Basecamp)

- Import the dDocs/dSheets **backup key** JSON (what `fileverse/walk-away` takes),
  holding the key in `keystore_module` or on a Keycard rather than in a file.
- List the portal: `getFileCount()` + `files(i)`, or `AddedFile`/`EditedFile`/`DeletedFile`
  logs via `eth_rpc_module.raw_rpc("eth_getLogs")`. No subgraph needed.
- Open with `appLock`/`ownerLock`: ECIES (implemented and tested) or RSA-OAEP (legacy).
- **Version history** from `EditedFile` logs (older CIDs, where still pinned).
- **Comments** from the `CommentModule` contract events (format still to be mapped).
- Blocker: new accounts lock keys with **post-quantum `FVPQ1`** from
  `@fileverse/crypto-pqc`, whose source is not public. Ask Fileverse to publish it;
  until then PQ-locked files open only by share link.
- Exit test: a Fileverse user with a backup key reads all their files, with history,
  in Basecamp.

## Stage 3: trust-minimised chain reads

- Cheap first step: ask two independent Gnosis RPCs and refuse on disagreement.
- Proper fix: Gnosis support in `verified_proxy_module` (nimbus has Gnosis beacon-chain
  presets; the module lists only mainnet/sepolia/hoodi today). Raise it with the
  module's maintainers.
- Exit test: a lying RPC produces an error, not a different document.

## Stage 4: write (Basecamp as a Fileverse client)

- Edit the Yjs state in the core (yrs) behind a native QML editor (Markdown-level to
  start), or behind Fileverse's AGPL React editor if Basecamp keeps a web view for
  modules. The second needs a Basecamp-team decision.
- Encrypt in the same formats (file key, link lock, title) and upload blobs to the
  user's own IPFS node or pinning service.
- `addFile`/`editFile` on the user's portal through `tx_sender_module` + `keystore_module`,
  paying xDAI directly. No Pimlico proxy, no `apps-storage.fileverse.io`.
- To check: whether minting a portal on the v3 AppRegistry is permissionless, and
  whether ddocs.new can read content that is not pinned on Fileverse's Pinata (its
  reads may go through Fileverse's gateway or proxy).
- Exit test: a document written in Basecamp opens in ddocs.new, and vice versa.

## Stage 5: live collaboration over Logos

- Carry the same encrypted Yjs updates over **Waku SDS channels** (`loam_core`)
  instead of Fileverse's socket.io + MongoDB server. Yjs updates commute, so the
  event-log/RBSR design from `logos-multiwriter-sync` fits. Batch updates for the RLN
  budget, and snapshot to Logos Storage.
- An always-on headless `fileverse_core` hub replaces the durable collab server.
- Fileverse clients will not join these sessions unless Fileverse adopts the
  transport. They already announce their collab servers over Waku
  (`/ddocs/1/server-discovery-*`), so this is a conversation worth having.
- Exit test: two Basecamp users edit one document offline and converge, with no server.

## Stage 6: sharing without the gate

- Private sharing today needs Fileverse's `/gate`: a 2-of-2 key split whose server
  half comes from a Fileverse-only HMAC master key, plus Privy for enrolment.
- Replace it with file keys wrapped directly to recipients' public keys (Loam/Keycard
  identities), stored with the metadata. Revoking means rotating the key and
  re-wrapping. No server half, no Privy.
- This is a protocol fork: ddocs.new will not understand these locks.
- Exit test: share with a named person, revoke them, and no service took part.

## Stage 7: registry choice

The portal contract is user-owned on a permissionless chain, so it is already the
least centralised part. What remains is the AppRegistry/forwarder admin (the v3
contract source is unpublished, so its powers cannot be checked). An optional LEZ
program could hold registries that should not be public.

## Dependency scoreboard

| Dependency | Stage 0 | 1 | 2 | 3 | 4 | 5 | 6 |
|---|---|---|---|---|---|---|---|
| Fileverse API / storage service | gone | | | | | | |
| Fileverse gateway / Pinata | untrusted, needed for availability | own pin + Logos mirror | | | own uploads | | |
| Gnosis RPC | trusted | | | verified | | | |
| Pimlico / forwarders | n/a | | | | gone (own gas) | | |
| Collab server (socket.io + Mongo) | n/a | | | | | Waku + hub | |
| Gate + Privy | blocks private shares | | | | | | gone (fork) |
| Closed PQ crypto | | | blocks PQ files | | | | |

## Ask Fileverse for

1. The source of `@fileverse/crypto-pqc` (`FVPQ1`) and of the v3 App/AppRegistry contracts.
2. A short spec of the share-link and lock formats (we reverse-engineered them).
3. Serving v3 content trustlessly from their gateway (`ipfs.fileverse.io` 403s on v3 CIDs).
4. Waku as a collaboration transport, since they already use it for discovery.
