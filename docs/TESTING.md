# What was tested, and how to repeat it

## 0.12.1: shared drafts sync both ways (2026-11-09)

vpavlin's test of 0.12.0: the joiner's edits reached the sharer, not the other way round.

- Causes: (1) "Share…" never marked the shared draft open, and a draft not open sent none
  of its edits (it still answered repairs, so the joiner got the starting text); (2) the
  relay loses messages ("No send stream" in delivery_module's log), and a lost edit was only
  noticed when someone reopened the draft (heads went out only on opening or on a gap);
  (3) a device re-sent others' operations as its own after receiving them (twice the
  traffic).
- Fixes: edits always go out (heads/presence/cursors still only for the open draft);
  sharing marks the draft open; heads 3 s after each burst of edits, so a peer that lost a
  message asks for it; received operations are not sent again. Joiners see "Leave".
- cargo: `the_sharers_edits_reach_the_joiner_even_if_not_marked_open`,
  `a_lost_edit_is_recovered_and_nothing_is_echoed`, and
  `identical_concurrent_typing_collapses_in_hashseq` (HashSeq: the same characters typed at
  the same place from the same state by two people are the same operations).
- Two Basecamps (t, u), logging ops in/out: a fresh share; three lines typed by u while t was
  closed arrived when t opened the draft; then t and u typed six lines in turn: identical
  on both, in order, carets shown.

## 0.12.0: shared drafts, edited together over Logos Delivery (2026-11-09)

- Design: docs/COLLAB.md. CRDT: HashSeq (davidrusu/hashseq @ 94f5e3c); sync after Reflection
  over Logos (vpavlin/reflection feat/logos-delivery); transport loam_core 0.7.3 (relay).
- cargo (collab.rs, an in-memory relay): sealing and topics; two editors typing at once
  converge and a late joiner catches up (repair from either peer); a stale edit is rebased;
  a draft in the background sends nothing; cursors follow inserted text; a 150,000-char draft
  crosses in several messages.
- Two Basecamp 0.3.2 instances on this host (rig sessions t on :92, u on :93), each with
  fileverse_core/ui, loam_core 0.7.3 (built from vpavlin/loam-basecamp d5c72af), ble_mesh
  0.2.1, delivery_module 0.3.0, on the Logos test network (RLN off):
  - t: new draft, Share… ("shared · only you here", transport connected); u: Join a shared
    draft with the id: the text arrived.
  - u typed a line: it appeared in t with u's avatar ("2 here") and caret.
  - Found and fixed: u did not see t (t answered presence only to "hello", but heard u's
    cursor first; now the first message from someone new gets a presence reply), and u's
    caret was placed by a stale text (now sent again after each save).
  - After a restart of both: each sees the other; typing in both at the same moment (t on
    line 1, u on line 2): identical Markdown on both, each caret where the other typed.
  - u closed, t added a line, u reopened: the line arrived (heads → repair).

## 0.11.0: agents write drafts (skill), the app notices (2026-11-04)

- `skills/fileverse-drafts/` (SKILL.md + `fvdraft.py`, Python standard library).
  Against the GUI rig's drafts directory (`FV_DRAFTS`): `new` made "Agent deck" (file
  mode 600, written via a temporary file inside the draft's own directory, since the
  app lists every JSON file in the drafts directory); `add-image` stored the picture under
  the same name the app gives it (`08c666…9a.png`) and printed an `fvimg:` reference the
  app shows; `save --base` with a stale value was refused (exit 1), with the current one
  accepted; `info` never prints the secret; `lint` reported all eight problems in a
  deliberately broken document (unlabelled diagram, `loop`, `autonumber`, a self-message,
  a web picture, `==`, unclosed columns, no title).
- Core: `draft_save` / `draft_save_rich` take `base` (the `updatedAt` the editor loaded)
  and refuse a save over a newer version (`{ conflict, draft }`).
- Basecamp GUI: "Agent deck" appeared in Drafts and opened (picture beside text in
  columns); an agent `save` while it was open and untouched showed the new line within
  seconds; an agent `save` while typing gave the banner "changed outside the editor …
  Saving is paused" (the file kept the agent's version); "Load their version" showed it;
  repeated, "Keep mine" saved the editor's text over it.
- Includes 0.10.1 (unlabelled Mermaid fences).

## 0.10.1: unlabelled Mermaid fences (2026-11-04)

vpavlin's diagram showed as code: its fence had no `mermaid` label. A fence without a
language whose first line names a Mermaid diagram type is now given the `mermaid`
language when the draft is turned into a document (so Fileverse draws it too). cargo:
`an_unlabelled_mermaid_fence_is_a_diagram`.

## 0.10.0: diagrams (Mermaid) in documents and slides (2026-11-04)

Asked by vpavlin: draw diagrams (module dependencies, data flows) in slides.

- Syntax: a code block in the `mermaid` language, which is how dDocs stores diagrams
  (`fileverse-ddocs/package/extensions/code-block`, mermaid 11), so published drafts
  show them in Fileverse too.
- Renderer chosen by a spike (three samples: module map, sequence, storage flow),
  each drawn through Qt 6.9's SVG plugin (what Basecamp ships):
  - merman 0.10.0-alpha.1: unusable through Qt (its CSS is ignored: black shapes; its
    flowchart labels are HTML in `foreignObject`), and about 35 MB more;
  - mermaid-rs-renderer 0.3.1 (MIT): plain SVG; about 5 MB more; Qt ignored the per-line
    `tspan` positions (wrapped labels ran together) and the CSS-style font list. The core
    now gives each line its own `text` and passes one font family (the view's).
- Core `diagram(source, colors, view_dir)` (a job) writes the SVG to the view's image
  cache in the view's colours; `doc::slides` gives a lone diagram the whole stage. The
  view draws `mermaid` blocks as pictures, never splits them, and shows the source with
  the reason when one cannot be drawn.
- Found on the way: the visual editor dropped every code block's language (Qt keeps
  none), so a diagram became plain code after Visual → Markdown. Now a block whose first
  line names a Mermaid diagram type is written as `mermaid`, and other languages are
  restored from the saved draft (same text, else same place).
- cargo: `flowcharts_and_sequences_render_for_qt`, `bad_input_is_an_error_not_a_crash`,
  `wrapped_labels_become_separate_lines`, `a_diagram_alone_fills_its_slide`,
  `code_languages_survive_the_visual_editor`. Harness: modes `diagram`, `diagramonly N`.
- Basecamp GUI: a new draft with a flowchart (subgraph, cylinders, a hexagon, labelled and
  dotted edges, `<br/>` line breaks) and a sequence diagram with a note, typed in the
  Markdown view: the live preview draws both; Visual → Markdown keeps ```` ```mermaid ````;
  presenting shows "How the modules fit" with the flowchart under it and the sequence
  diagram alone on a slide after `===`.

## 0.9.2: `===` page breaks and clickable links (2026-11-03)

Asked by vpavlin: `===` should start a new slide, and links should be clickable.

- `===` already broke slides on a line of its own, but directly under a line of text
  Markdown reads it as that line's heading underline (no break, the line became a
  heading). A line of only `=` (3 or more) outside code is now always a break. cargo:
  `a_line_of_equals_signs_is_a_slide_break`.
- Bare `http(s)://` addresses become links (in documents built from drafts, in the
  preview and on slides, and in the visual editor; written back bare). cargo:
  `bare_web_addresses_become_links`, `bare_addresses_are_links_in_the_editor`.
- Links did not open: inside Basecamp `Qt.openUrlExternally` returns false (the view's
  QML sandbox). They now open through the core (`open_link`: http, https and mailto
  only, one argument to `xdg-open`/`open`). cargo: `only_web_and_mail_links_open`.
- Basecamp GUI (with a stand-in `xdg-open` that logs its argument): a pasted
  `https://logos.co/deck` showed as a link in the editor; Ctrl+click opened it, a plain
  click did not (it places the cursor; `linkAt` is off by the TextArea's padding, so the
  hovered link is used); clicking it in the Markdown preview and on a slide opened it, and
  the slide did not advance; `===` typed under that line split the deck there.

## 0.9.1: lists and headings inside columns (2026-11-03)

Reported by vpavlin on 0.9.0: in a column the list buttons said "Tables keep their own
layout", `- ` did not start a list (it was saved as `\-`), the cursor behaved oddly in
columns and jumped to the top when switching between Visual and Markdown.

- Causes, found with a Qt 6.9.2 TextArea probe (`qml` offscreen):
  - the view only counted U+2029 between blocks; Qt marks each table cell with U+FDD0
    and the table's end with U+FDD1, so a block inside a column had no number of its own;
  - `getText(0, pos)` with `pos` inside a table returns the whole table (Qt widens a
    selection that ends in a cell), so the view's idea of the cursor's block was wrong
    there. The view now cuts the full text at the cursor (positions match);
  - after a table Qt keeps an empty text block it does not write out, unless text was
    typed into it. The view now says, per table, whether that piece is empty
    (`rich::At`, "7:10").
- The core restyles blocks inside a column (headings, lists, tasks, quotes; not code);
  real tables still keep their layout. `to_html` no longer closes `</body></html>` after
  every segment.
- cargo: `blocks_inside_columns_are_found_and_restyled` and
  `text_typed_after_columns_is_the_next_block` run against HTML Qt itself wrote back
  (tests/fixtures/qt_columns*.html), `block_styles_change_one_block` updated to Qt's real
  piece count (a table's cells and the piece after it).
- Basecamp GUI: in a column, "• List" turned "Text here" into a list item and Enter
  continued it; typing `- ` at the start of a line in a column made it a list item (the
  Markdown shows `- Second item`, no `\-`); H2 in the right-hand column; text typed after
  the last columns block (Ctrl+End) became an H2. Markdown → Visual and back keep the
  cursor where it was (typed "XX" / "YY" landed after "Second item" both ways).
- Also fixed: opening the editor straight after a restart showed remote pictures as thin
  strips (Qt remembered that the cache file was missing when it first tried); the view now
  reloads them with a new query once fetched, and the core ignores the query.

## 0.9.0: images and columns in the editor, a real full-screen window (2026-11-03)

- cargo: `columns_and_images_cross_the_editor`, `qt_columns_and_images` (rich.rs),
  `unixfs_files_read_back` (ipfs.rs), `a_drafts_image_is_published_encrypted`
  (tests/images.rs): a draft's picture is encrypted with a fresh key, its tag kept apart,
  stored as a UnixFS file and its reference rewritten when the draft is published.
- Harness: modes `wysiwyg` and `slidecols`.
- Basecamp GUI, on vpavlin's image document:
  - "Edit a copy" shows both images and the two columns (list beside the picture) in the
    visual editor. Markdown view shows `::: columns align-center` … `::: column` … `:::`
    and `fvimg:` references.
  - The Columns button inserts two columns at the cursor. The Image button opens Qt's file
    dialog; the chosen PNG went into the right-hand column, was stored as
    `drafts/<id>/img/<sha>.png`, and shows in the editor, the preview and on the slide.
  - Present, then F: a separate "Presentation" window opens at screen size (inside
    Basecamp, `Window.window` is an offscreen window, so making it full screen did
    nothing in 0.8.x). Arrow keys move slides in it; Esc returns to the deck in Basecamp
    on the same slide.
  - A second visit to the shared document offers "Edit my copy" (reopens the latest
    draft copied from it) beside "New copy", instead of making another copy each time.
- Found and fixed during the walk: pictures added from disk stayed "Loading image…" in the
  preview and on slides (the view needed a CID; such pictures have only a local reference
  until published); the presentation window opened 160×160 when nothing applied full
  screen (now sized to the screen first).
- Rig limits (Xvfb, no window manager): full screen itself, and focus returning to Basecamp
  after the presentation window closes (a click was needed before Esc worked again), are
  window-manager behaviour and were not seen here. The file dialog did not take keyboard
  focus until clicked.

## 0.8.1: images in Basecamp (2026-11-03)

- On vpavlin's real dDoc with two images (one alone, one beside a list in columns), 0.8.0
  showed blank images. Two causes:
  1. Fileverse serves document images from separate gateways (`images.fileverse.io`,
     `ipfsimages.mypinata.cloud`) that answer only with whole files, not hash-checkable
     blocks, so the trustless fetch found nothing. Now images are fetched whole (your node
     first, then those gateways): safe, because AES-GCM authenticates the ciphertext with
     the key and tag from the hash-checked document. The document itself only loaded through
     `apps-ipfs.fileverse.io`, now a default gateway (it serves raw blocks).
  2. Basecamp's QML sandbox blocks `data:` URLs (and anything outside the view's install
     directory); the offscreen harness has no sandbox, so it did not show. The core now
     writes decrypted images to `plugins/fileverse_ui/imgcache` (only a directory with that
     name, under a `plugins` directory, is accepted), pruned after a day and emptied when
     the core starts. Decrypted images are therefore on disk while cached.
- Basecamp GUI, same document: both images render in the document; presenting shows "Image
  one" on its own slide and "Image Two" with the list beside the picture.

## 0.8.0: slides as in fileverse-ddocs#550 (2026-11-03)

- Slides follow vpavlin's dDocs PR #550 (direct render): the core cuts at structure only (an
  H1/H2 opens a slide and content packs beneath it, page breaks end one, no bare title
  slides); the view lays each slide out on a fixed 1080x608 stage, measures it off screen,
  and breaks between blocks (list items one by one), never inside columns, tables or images;
  a heading never ends a slide, and headings stay with a block too tall to share. The stage is
  scaled to the screen. Slide numbers on the stage; `+`/`-`/`0` change the font scale and
  re-cut the deck, staying on the same content; `F` toggles full screen.
- Columns (`columns` > `column`, dDocs' image beside text) are kept as one block with each
  column's blocks; layouts `align-right`/`align-left` make one column twice as wide. Markdown
  export lists the columns one after another. Test: `doc::tests::columns_keep_an_image_beside_text`.
- Images: encrypted dDocs images (`resizableMedia` with `encryptionKey`, `nonce`, `authTag`,
  `ipfsHash`) are fetched hash-checked and decrypted with AES-256-GCM as Penumbra writes them
  (12-byte IV, tag in the node), and shown from memory as data: URIs. Test vector made with
  WebCrypto (`tests/images.rs`). Headless runtime: the encrypted block put into the session's
  ipfs_module node came back through `image` as the original PNG, byte for byte.
- Basecamp GUI: a draft presented 10 slides; `+` x3 made 12 and stayed on the same slide.
  Harness: the measured split of the Fileverse SDK doc; an H2 with columns (text | image)
  renders side by side. Full screen: `Window.window` switches to FullScreen and back in the
  harness; the Xvfb test display has no window manager, so the visible effect is untested.
- Not tested against a real Fileverse document with images or columns (none available).
  Images in Fileverse's private lane and unencrypted external images are not loaded.

## 0.7.1 (2026-10-27)

- Your own published drafts open as drafts: a recent entry records its draft (`draftId`),
  and older entries are matched by the links the draft was published under (`links`).
  Basecamp GUI: the published "SDK notes" in Recent ("Your draft") opened in the editor;
  its published view offers "Edit" (not "Edit a copy"), which opened the same draft, and no
  new draft was created.
- Store name of the core: "Fileverse Core".

## 0.7.0: visual editor (2026-10-26)

- The editor edits formatted text (Qt RichText) with a toolbar (text, H1-H3, bold, italic,
  strikethrough, inline code, link, lists, task, quote, code block, undo/redo), Ctrl+B/I/K and
  typed shortcuts (`# `, `## `, `- `, `1. `, `[] `, `> `, ```` ``` ````). Drafts stay Markdown.
- Why not Qt's Markdown mode: its writer garbled code blocks and tables that follow lists or
  quotes (seen in the harness). The core converts instead (`rich.rs`): Markdown -> HTML for Qt,
  Qt's HTML -> Markdown, and block restyling on a block list counted as Qt counts text blocks.
  `rich::tests` use HTML captured from Qt 6.9.2; Markdown -> HTML -> Markdown renders the same.
- Found in the GUI and fixed: async conversions overwrote text typed meanwhile (now blocking
  `callModule`, milliseconds); Qt drops empty headings/lists/quotes, so a block restyled while
  empty holds a zero-width space (stripped on save); Enter after a heading continued the heading.
- Basecamp 0.3.2 GUI (OpenGL): a new document typed with shortcuts and the toolbar saved as
  `# Visual editor test / **Make this bold. And** ***this italic*** / - first item / - second item /
  ## Next section / plain after heading`; a task ticked from the toolbar saved as `- [x]`; switching
  to Markdown and back keeps the document.
- Not done: tables are shown but cannot be edited structurally; headings drop inline bold.

## 0.6.0 (2026-10-26)

- Version-4 titles are AES-GCM with a **24-byte** IV in front (`aesDecrypt` in
  @fileverse/crypto); they fell back to the first line of text. Test vector made with
  WebCrypto (`gcm_with_a_24_byte_iv`); the user's renamed document now shows its title
  ("Something testful") instead of its first line.
- `check_update(url)`: re-reads the file record on Gnosis and compares its metadata/content
  CIDs with the ones last opened. Runtime test on the user's document: unchanged → `false`;
  with the stored roots made to look older → `true`. The view checks every 60 s while a
  document is open (not while presenting) and reloads it in place, keeping the tab.
  Not tested: an actual edit made in Fileverse while Basecamp watches (needs the user).

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
