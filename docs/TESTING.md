# What was tested, and how to repeat it

State on 2026-10-22, fileverse_core / fileverse_ui 0.2.0.

## 1. Engine (cargo, no Logos runtime)

```sh
cd fileverse_core/rust-lib
cargo test --no-default-features                     # 16 tests
cargo test --no-default-features -- --ignored live   # the real legacy share link
```

| Test | What it proves |
|---|---|
| `v3_fixtures::ddoc_raw_link_secretbox_gate` | v3 web-app links: raw link secret, secretbox gate, tag+IV-suffixed content, nonce-prefixed title, tabs |
| `v3_fixtures::ddoc_api_link_argon_and_cbc_gate` | Fileverse API links: Argon2id(16 MiB)-wrapped link key, PBKDF2 + AES-CBC link lock |
| `v3_fixtures::dsheet_gz1` | dSheets: `gz1:` gzip content, Y.Map celldata, formatted values |
| `v3_fixtures::ecies_matches_fileverse_crypto` | secp256k1 ECIES byte-compatible with `@fileverse/crypto` |
| `v3_fixtures::wrong_key_is_a_clear_error` | wrong key gives a readable error, not garbage |
| `doc::tests::*` | dDocs presentation-mode slide rules (H1 title slides, H2 breaks, page breaks, `===`, 8 lines / 1000 chars / 250 words, no orphan headings) |
| `live_legacy_share_link` (ignored) | the public Fileverse Agents SDK doc, over Gnosis + IPFS |

The fixtures (`tests/fixtures/v3.json`) are made by `tools/make-v3-fixtures.mjs` with the JS libraries
Fileverse uses (`@noble/*`, `tweetnacl`, `js-base64`, `yjs`). The v3 formats are verified against those
reference implementations, **not against a real ddocs.new share link**: none was available. Opening
one real v3 link is the first thing to do when someone has one.

Network behaviour, checked with `fvview` (the CLI over the same engine):

- A hostile gateway answering with other bytes is rejected ("block does not match its CID") and the
  next gateway is used.
- With every gateway dead, a document opened before reopens from the verified block cache.
- `--keep <kubo api>` copies the blocks into a Kubo node. After wiping the cache, the document
  opens with that node's gateway as the only source.

## 2. Module, headless (logosctl 0.3.2)

Session with the official `eth_rpc_module` 0.1.0 .lgx and our core:

- `module load fileverse_core` loads `eth_rpc_module` as its dependency.
- `open` + `job`: the real link resolves through eth_rpc_module (chain 100 seeded by `ensure_chain_config`).
- `set_ipfs_node`, `node_status`, `keep` against Kubo 0.43.1.
- With the module's block cache deleted and the gateways set to a dead address, the reopen is served
  by the local node (`provenance.blobs[].gateway = http://127.0.0.1:18081`, verified).
- `keep` with the node down returns "your IPFS node at … is not answering (is it running?)".

## 3. View, offscreen

`tools/qml-harness/run.sh MODE OUT.png` renders `fileverse_ui/Main.qml` with Qt 6.9.2 and the design
system revision Basecamp bundles, against a mock `logos`. Modes: `doc`, `slides N`, `sheet`, `error`,
`settings`. It caught a QML load failure (`short` is a reserved word) before Basecamp ever saw it.

## 4. Basecamp 0.3.2 GUI (Xvfb, xdotool, scrot)

Packages unpacked into a fresh `HOME` the way the package manager does. Walked through:

- The view opens; the core answers.
- Paste link → document; Recent entry; restart → Recent persists and reopens.
- Present → title slide → arrow keys → Esc.
- Wrong key → "The link key does not open this file…"; a non-Fileverse URL → "The link has no #key= part…".
- Settings: gateways list; IPFS node fields → "Connected: kubo/0.43.1 …".
- Keep a copy → "Kept on your IPFS node: 2 blocks, 2 pinned"; button turns "Kept ✓"; Kubo lists both pins.

Not tested: macOS/ARM builds (only linux-amd64 was built), a real ddocs.new v3 link, a real dSheets
link, private/workspace files (unsupported by design), very large documents.

## Machine notes (this test host)

The host's glibc is 2.36; logosctl and Basecamp 0.3.2 AppImages need 2.38. They were run from their
extracted contents with their ELF interpreters pointed at nix's glibc (`tools-bin/` outside this repo:
`logosctl/ctl.sh`, `basecamp/bc.sh`, `gui/{env.sh,launch.sh,install-lgx.sh}`). This is a test-host
workaround, nothing the packages need.
