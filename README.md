# Fileverse viewer for Logos Basecamp

Read Fileverse dDocs and dSheets share links inside Basecamp, without any
Fileverse server.

```
share link  ->  files(id) on the portal contract (Gnosis, via eth_rpc_module)
            ->  metadata / gate / content CIDs
            ->  IPFS blocks from any gateway, each checked against its CID
            ->  link key -> link lock -> file key -> AES-GCM  (all local)
            ->  Yjs -> blocks / sheets  ->  QML view
```

| Package | Type | What it does |
|---|---|---|
| `fileverse_core` | core, Rust (`cdylib`) | The engine: chain reads, trustless IPFS fetch, decryption, Yjs rendering, backup keys, Markdown drafts and publishing, copies on IPFS and Logos Storage, AI chat history and AI Gateway sessions. Depends on the official `eth_rpc_module` and `reflection_core`; uses `ipfs_module`, `storage_module` and `ai_gateway_core` when present. |
| `fileverse_ui` | ui_qml, pure QML | Recent / My files / Drafts, document and sheet views, slide decks, the editor with live preview, settings. |
| `ipfs_module` | core, Rust (`cdylib`) | An IPFS node for every Logos app: a bundled static Kubo run on loopback ports, started on first use, stopped with its host. Fileverse starts it only when it needs it (Settings → IPFS node). |

What it does (core 0.17, view 0.21):

- **Read share links** from the legacy portal (`docs.fileverse.io`) and the v3 App contract
  (`ddocs.new`, `dsheets.new`), in every form Fileverse hands out: `…/<portal>/<fileId>#key=…`,
  short links `…/d/<code>#k=…` (portal looked up in the registry's logs on Gnosis) and
  `…/document/<id>#k=…` (looked up in the portals you know, on chain). Raw and
  Argon2id-wrapped link keys, secretbox and PBKDF2/AES-CBC link locks, version-4 titles,
  `gz1:` sheets, document tabs.
- **Live documents**: an open document is checked against its record on Gnosis every minute
  and reloads when it was edited in Fileverse.
- **Your own files**: import a dDocs/dSheets backup-key file; the portal is listed from Gnosis
  (titles decrypted locally) and every file opens with its owner/app lock (ECIES, or RSA-OAEP
  on legacy portals). Post-quantum (`FVPQ1`) locks are reported, not opened: that library is
  not public. Keys stay in the module's data directory (mode 600).
- **Present** any dDoc (or draft) as a slide deck, cut the way dDocs' presentation mode cuts it.
- **Edit**: "Edit a copy" of any document, or a new one, in a visual editor (toolbar, Ctrl+B/I/K,
  typed shortcuts like `# ` and `- `), or as Markdown with a live preview. Saved on the device
  as you type, as Markdown. **Publish** encrypts it in the
  Fileverse v3 format (Yjs content, GCM, secretbox link lock), pins it on your IPFS node and
  copies it into Logos Storage, and gives a `fvbc://ipfs/<manifest>#key=…` link that any
  Basecamp with this app opens. There is no chain record; ddocs.new cannot open these links.
- **Edit together**: "Share…" turns a draft into a Reflection note (through Reflection's
  engine, `reflection_core`, a required dependency) and copies its id; others join it with
  "Join a shared draft…" in Fileverse, or open it in the Reflection apps. Live text,
  presence and cursors, published on Logos Delivery, p2panda or both (Settings). See
  [docs/COLLAB.md](docs/COLLAB.md).
- **Keep on my IPFS** pins a document's blocks on your node (`ipfs_module`, or a Kubo you set).
- **IPFS node: Off / When needed / Always on** (Settings). A running node takes part in the
  IPFS network and uses bandwidth. *When needed* (the default) starts it only for Keep on my
  IPFS, Back up and Publish, and stops it 10 minutes after the last of them; documents are read
  through it only while it runs, otherwise from public gateways and Logos Storage. *Off* never
  starts it (publishing then goes to Logos Storage only). *Always on* starts it whenever
  Fileverse runs, as before.
- **Download Markdown** saves a document or draft as a `.md` file in your Downloads folder.
- **Write with AI** (through the AI Gateway app, `ai.ask`): continue, rewrite (clearer, shorter,
  more formal), fix grammar, summarize, translate or ask about the selection or the paragraph
  you are in. The answer is shown first; nothing changes until you choose Replace, Insert below
  or Insert at cursor (one undo step each).
- **AI chat** (through AI Gateway, `ai.session`): a panel beside the editor to talk with an AI
  about the open document. You allow the chat once in AI Gateway (and pick the model); after
  that each message goes without a dialog, and the document only when it changed. Answers
  appear as they are written. The AI
  proposes edits as cards you Apply or Dismiss. Without AI Gateway these buttons say how to get
  it.
- **Your own AI prompts** (Settings → AI prompts): edit any prompt Write with AI and AI chat
  send, reset it to Fileverse's, and add AI actions of your own. Kept on this device only.
- **Copy to Logos Storage** uploads each block to `storage_module`; when IPFS has nothing,
  blocks are read back from Storage and hash-checked like any other source.
- Rendering: headings, paragraphs, lists, tasks, quotes, callouts, code, tables, links;
  images and embeds are placeholders and **never fetched**.

Tested: see [docs/TESTING.md](docs/TESTING.md).

Not yet: comments, version history, private (gate) shares, workspace files on Fileverse's
private IPFS lane, images, writing back to Fileverse's chain records, pictures in shared drafts. See
[docs/PLAN.md](docs/PLAN.md).

## Trust model

- **Chain:** whatever endpoint the user set for chain 100 in the Ethereum RPC app
  (`eth_rpc_module`, SOCKS/Tor capable). The viewer seeds `https://rpc.gnosischain.com`
  there only if chain 100 is missing. The RPC is trusted for *which* CIDs a file has;
  there is no light client for Gnosis in Basecamp yet.
- **IPFS:** not trusted. Blocks are requested as trustless raw blocks
  (`?format=raw`), hashed and compared to the CID; UnixFS files are reassembled from
  verified leaves. Order: your own node (ipfs_module, when it runs), then gateways, then Logos Storage.
  A source can refuse or watch, never alter.
- **Keys:** the `#key=` stays on this device. Recent links (with their keys) are kept
  in the module's own data directory so documents reopen; decrypted text is not
  written to disk. Verified ciphertext blocks are cached by CID.
- **Rendering:** the core escapes every character of document text; the only markup
  that reaches Qt's StyledText is markup the core wrote (`b i u s code br`, and `a`
  limited to http/https/mailto). Links open only on click.
- **IPFS reads:** with the IPFS node set to *When needed* or *Off*, blocks come from public
  gateways when your node is not running, so those gateways can see which CIDs you fetch
  (not what they contain: the content stays encrypted). Choose *Always on* to read through
  your own node.
- **AI:** Fileverse never talks to a model itself. Write with AI and AI chat go through the AI
  Gateway app, which shows exactly what is sent, lets you pick the model and its privacy tier,
  and asks you first (once per chat). What goes out: the selection or paragraph (Write with AI),
  or your messages and the document's Markdown (AI chat). AI edits change the document only when
  you apply them. The chat history is kept per document on this device only, unencrypted like
  the drafts themselves (`<module data>/chats/`), and is never shared or published.

## Agents

`skills/fileverse-drafts/` is an agent skill (Claude Code `SKILL.md` format, usable by
any agent with a shell) for writing and editing drafts that appear in this app's Drafts
list: where drafts live, the `fvdraft.py` helper (list, show, new, save with a change
check, add-image, lint) and the Markdown the app understands (slides, columns, pictures,
Mermaid diagrams). It runs on the machine where Basecamp keeps its data. To install it
for Claude Code: copy or link the directory into `~/.claude/skills/`. Agents do not
publish; the app reloads a draft changed on disk, and asks before your unsaved edits and
an agent's change meet.

## Build

```sh
cd ipfs_module    && nix build .#lgx-portable    # bundles a static Kubo (pkgsStatic.kubo)
cd fileverse_core && nix build .#lgx-portable
cd fileverse_ui   && nix build .#lgx-portable
```

Note: never put a `;` in a `metadata.json` string. CMake reads the file as a list and
the build fails with "value, object or array expected".

Engine tests and the CLI need only cargo (stage the SDK first, see `rust-lib/Cargo.toml`):

```sh
cd fileverse_core/rust-lib
cargo test --no-default-features                         # offline, incl. v3 fixtures
cargo test --no-default-features -- --ignored live       # the real legacy link
cargo run --no-default-features --features cli --bin fvview -- '<share link>'
```

`tests/fixtures/v3.json` is generated by `tools/make-v3-fixtures.mjs` with the same
JS libraries Fileverse uses, so the Rust code is checked against the reference
implementations.

## Licence

MIT or Apache-2.0. This is a clean-room implementation of the file formats; it
contains no Fileverse (AGPL) code.
