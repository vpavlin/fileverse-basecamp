# What was tested, and how to repeat it

## fileverse_core 0.5.1 (2026-10-26)

- `/document/<id>#k=` links (what docs.fileverse.io's Share dialog gives now): the id is the
  document's `appFileId`. Fileverse's web app asks its indexer (`/ddoc/by-id/<id>`, refused
  without the app's Origin header) for the portal; we look in the portals this app knows (backup
  keys imported, portals opened before) with `appFileIdToFileId` and confirm with the file
  record. Real link from the user, headless runtime: cold → clear error; after one open by long
  link → the `/document/` link opens ("This is a test!").
- Version-4 files lock the file key as `{"key","iv":"","authTag":""}`: a bare key in the legacy
  JSON shape (it failed with "GCM nonce must be 12 bytes, got 0"). Unit test `file_key_wrappers`.

## 0.5.0 / ipfs_module 0.1.2 (2026-10-26)

- New UI: sidebar with navigation and a status card, kind tiles instead of `¶`/`▦` prefixes,
  status chips (IPFS, Storage), relative dates, a filter for long portals, a readable-width
  document page, an "Opening…" state that counts up. Rendered offscreen in every harness mode,
  then checked in Basecamp 0.3.2 with OpenGL (llvmpipe): icons draw there (they are tinted by a
  shader effect, which the offscreen harness and Qt's software renderer skip).
- Short links `…/d/<12 hex>#k=…`: the portal token id is looked up in the v3 AppRegistry's
  `Mint` log on Gnosis and confirmed with `appInfo` (unit test with a mock RPC that lies;
  live test `live_short_link_token`: token 1 → 0x6a45…74e5). rpc.gnosischain.com serves the
  whole log range; publicnode (50k blocks), drpc (10k) and 1rpc (50) refuse it, so the core
  tries every endpoint. Registry v1 short links are refused with a hint (only Fileverse's
  indexer can resolve them). In the GUI: a `/d/` link to token 5, file 0, with a dummy key
  resolved the portal on chain, read the file record and its blocks, and refused the key.
- Opaque `/document/<id>#k=` links: refused with an explanation (only Fileverse's coordinator
  knows which file they name).
- ipfs_module 0.1.2: a starting node reached 318 peers despite ConnMgr 10-20 and the first open
  after Basecamp started timed out on every Gnosis RPC. With a resource-manager cap (64
  connections) a 2-minute online run peaked at 44 peers with RPC calls at 60-150 ms
  (`online_connection_count_stays_bounded`); in the GUI the same open succeeded 4 s after start
  with 32 peers.

## fileverse_core 0.4.1 (2026-10-23)

- Titles of older dDocs (metadata `version: "2"`) are sealed with the share-link secret, not
  the file key, so a backup key alone cannot open them (checked on the real public legacy doc:
  the link secret opens its title, the file key does not). Fileverse's own app then shows
  "Untitled" unless the browser remembers the title. Now: the title is opened with
  `ownerLock.lockedLinkKey` when the file has one, else the first heading (or first line) of the
  document is used and marked `titleFromText`. `tests/own_files.rs::titles_sealed_with_the_link_secret`
  covers both; fixtures regenerated with `tools/make-own-fixtures.mjs`.
- Not tested against the user's real portal (this host has no copy of their key).

## 0.4.0 / ipfs_module 0.1.1 (2026-10-23)

- First real-data test by the user: a real dDocs backup key listed and opened their portal.
- Portal list cached on disk (`portals.json`): shown at once on return, refreshed in the
  background when older than 15 min. GUI: open a file, go back, the 77-file list is still there.
- Listing reads 6 files in parallel with progress; chain calls capped at 2 in flight, retried,
  and switched to other public Gnosis RPCs when the configured one fails (rpc.gnosischain.com
  timed out from the test host); a file that cannot be read is listed with the reason.
  81-slot portal: 175 s on a degraded link, 77/77 read (was 443-521 s with 27-31 lost).
- Back up everything: 77 files, 658 blocks to disk, 155 roots pinned on ipfs_module, 658 blocks
  into Logos Storage in 133 s (headless), and the same from the Basecamp GUI ("77 backed up").
- Reads: your node first for 3 s, then gateways/Storage; blocks found elsewhere are added to the
  node (`tests/node_cache.rs`: a node that never answers costs 3 s and then gets the block).
- ipfs_module 0.1.1: Kubo's defaults flooded the test host's NAT (every request timed out until
  the node stopped). Measured: UDP transports were the cause; TCP-only kept ~220 ms with 130+
  peers. Now: lowpower profile (DHT client), TCP only, 10-20 connections, announce pinned only.
  Some latency spikes remain during heavy backup/pinning; the host's baseline also spikes.
- `#k=` share links in the `/<portal>/<file>` form.


State on 2026-10-23: fileverse_core / fileverse_ui 0.3.0, ipfs_module 0.1.0.

## 1. Engines (cargo, no Logos runtime)

```sh
cd fileverse_core/rust-lib
cargo test --no-default-features                     # 28 tests
cargo test --no-default-features -- --ignored live   # the real legacy share link
cd ../../ipfs_module/rust-lib
FV_KUBO_BIN=/path/to/static/ipfs cargo test --no-default-features -- --ignored
```

| Suite | What it proves |
|---|---|
| `tests/v3_fixtures.rs` | v3 share links (raw + secretbox, API-style Argon2id + PBKDF2/CBC), gz1 dSheets, ECIES vs `@fileverse/crypto`, wrong key |
| `tests/own_files.rs` | backup keys: mixed backup parsing; current portal listed (titles, newest first, deleted skipped, post-quantum flagged) and opened (dDoc + dSheet via ECIES); legacy portal via RSA-OAEP; wrong key |
| `tests/publish.rs` | draft → Fileverse-format files + manifest → `fvbc://` link opens from a mirror with no chain; a tampering mirror is refused; wrong key; size limit |
| `md::tests` | Markdown → Yjs → renderer round trip (marks, lists, nesting, tasks, quotes, code, tables, links); source markup stays text; Markdown → Yjs → Markdown is stable |
| `doc::tests` | dDocs presentation slide rules; export marks hug text; lists end before the next block |
| `kubo::tests` (ignored) | the supervisor starts Kubo from a thread that then exits (as an IPC worker does) and the daemon survives; block round trip via API and gateway; restart keeps the peer id |

Fixtures come from `tools/make-v3-fixtures.mjs` and `tools/make-own-fixtures.mjs`, made with the
JS primitives Fileverse uses. **Not tested against real data:** a real ddocs.new v3 link, a real
dSheets link, a real backup-key file (we have none); the formats are checked against those
reference implementations instead.

## 2. Modules, headless (logosctl 0.3.2)

Sessions with the official `eth_rpc_module` 0.1.0, `ipfs_module` and `fileverse_core` (the host
bundles `storage_module` 3.0.2):

- `ipfs_module`: `endpoints` autostarts the bundled Kubo (repo in the module's data dir, random
  loopback ports); after ~1 min it has ~40 peers and fetches Fileverse's own v3 and legacy CIDs
  from the IPFS network in 0–2 s each, with no gateway. Found and fixed: PR_SET_PDEATHSIG is
  per-thread, so the daemon died when the IPC worker that started it exited.
- The real share link opens through the module's node (`gateway = 127.0.0.1:<port>`, verified).
- `copy_to_storage`: 2 blocks into Logos Storage. Then with the IPFS node stopped (autostart off),
  gateways set to a dead address and the block cache deleted, the document opens from
  **Logos Storage** alone, every block hash-checked.
- `keep`: 2 roots pinned on the module's node.
- Drafts: `draft_create`, `draft_save` (preview + slides), `publish` → pinned on ipfs_module and
  4 blocks in Logos Storage. The `fvbc://` link opens from Logos Storage only; a wrong key fails.
- **Second node** (separate session, own ipfs_module, empty cache, no Storage map, no gateways)
  opens the first node's published document over IPFS, P2P, verified.
- Public gateways (ipfs.io, dweb.link, Pinata) could **not** fetch the published document: this
  test host is behind NAT, so its node is not dialable from the internet. Reach beyond peers that
  can dial you needs a reachable node or relay (as with Logos Storage).

## 3. View, offscreen

`tools/qml-harness/run.sh MODE [N] OUT.png` (Qt 6.9.2 + the design system Basecamp bundles; mock
`logos`). Modes: doc, slides, sheet, error, settings, portal, import, edit, editslides. It caught
two load failures before Basecamp did: `short` (a reserved word) and `LogosButton.highlighted`
(no such property).

## 4. Basecamp 0.3.2 GUI (Xvfb, xdotool, scrot)

- The app loads; the footer shows `IPFS: ipfs_module · Storage` (node started inside Basecamp;
  the daemon died with Basecamp when it was stopped).
- Recent carries over from 0.2.0; open → Copy to Logos Storage ("In Logos Storage ✓");
  repeat → "0 copied, 2 already there".
- Edit a copy → editor with live preview → title and text edited → autosaved → Publish →
  "published here · IPFS · Storage" → Open published ("published from Basecamp (no chain
  record)") → Keep and Copy marks persist across reopen.
- My files → paste a backup key → keys stored (mode 600) → portal listed → a non-portal address
  says "is not a Fileverse portal on Gnosis (no contract answers there)".
- Found and fixed during the walk: stale "On my IPFS" from another node (now tied to the node's
  peer id), lost marks after reopening a published doc (CID sets compared in order), `**bold **`
  in exported Markdown, a list running into the next heading, the title duplicated in "Edit a
  copy", and an unhelpful ABI error.

Not tested: the backup-key file picker (paste and a path are; the Qt dialog under Xvfb was not
driven), macOS/ARM builds, very large documents (publishing is limited to 1 MiB per file).

## Machine notes (this test host)

The host's glibc is 2.36; logosctl and Basecamp 0.3.2 AppImages need 2.38. They were run from
their extracted contents with their ELF interpreters pointed at nix's glibc (`tools-bin/` outside
this repo: `logosctl/ctl.sh`, `basecamp/bc.sh`, `gui/{env.sh,launch.sh,install-lgx.sh}`,
`fvcall.sh`, `fvjob.sh`). A test-host workaround, nothing the packages need.
