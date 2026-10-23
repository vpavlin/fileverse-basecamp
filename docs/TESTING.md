# What was tested, and how to repeat it

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
