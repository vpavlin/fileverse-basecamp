# What was tested, and how to repeat it

## 0.16.2 (view): "catching up…" also while changes wait for a missing one (2026-11-11)

reflection_core 0.6.3 adds `delivery.loroPending`: received changes held back because they
depend on one this device never got (the "title only" state). The chip shows "catching up…"
for it as for `catchingUp`. Works with 0.6.2 too (the field is then absent). Harness:
sharedcatchup now sets only loroPending.

## 0.16.1 (view): the title keeps its room on shared documents (2026-11-11)

On a shared document the title row held the title, the avatars of who is here, the sharing
chip, "catching up…" and four buttons, and the title was cut off ("edback: Chat in a fresh
Basecamp"). The avatars and the chips moved to the row under the title, next to Copy id and
Download Markdown; the title row keeps the title and the buttons.

- Rig, both Basecamps on reflection_core 0.6.2 and Fileverse 0.16.0: the 10.5 KB feedback
  document shared from t, joined from u. u had the whole text after about 150 s,
  byte-identical, showing "catching up…" until then; the chip went away by itself.

## 0.16.0: Download Markdown; "catching up…" on shared documents (2026-11-11)

"Download Markdown" saves the document as a .md file in the Downloads folder (XDG_DOWNLOAD_DIR,
else ~/Downloads, made if missing), named after its title, like a browser download: no dialog,
never overwriting ("Title (2).md"), and a message says where it went. It is under the title in
the editor (the editor's text as it is now, starting with the title as a heading) and next to
"Copy Markdown" for opened documents. The core's new `download_file(title, ext, content)` does
the writing; the title is made a safe file name (no separators, not hidden, 100 characters).

Qt's own save dialog was tried first and dropped: inside Basecamp it listed the process's
working folder whatever folder it was given, and did not fill in the proposed name.

- Basecamp t, "Feedback on 0.3.3" (10.5 KB): downloaded twice, "Feedback on 0.3.3.md" and
  "… (2).md", byte-identical to the user's original Markdown.
- Core: safe names, the "(2)" rule and XDG_DOWNLOAD_DIR have tests.

Shared documents need reflection_core 0.6.2, which keeps asking for missing changes until a
note is complete (before, a joiner asked three times in its first 30 s and the sharer answered
one of them: on a slow network the note could stay at its title for good). Its
`state().current.delivery` drives a new chip: "catching up…" while others' changes are missing,
"sending…" while ours are not out yet. "Copy id" moved to the row under the title, with
"Download Markdown", so a shared document's title is not cut off. Harness: sharedcatchup.

## 0.15.1 (view): Share no longer hides a failed send (2026-11-11)

vpavlin shared an existing document; the other side saw "2 here" but no text. Joining the
same note from Basecamp t showed only its first line, the title: Share creates the note with
the title, then sets the whole text, and ignored whether that worked. A refused text (most
likely reflection_core's 100 KB limit, "The note is too long") still linked the draft and said
"Shared". Now Share checks the size first (at most 100 KB of text, with a clear message) and
the result of setting the text; on a refusal it says why and shares nothing.

- Basecamp t joined vpavlin's note 8f28acd3…: one operation arrived (the title), nothing more
  in 90 s.

## 0.15.0: published documents findable in Logos Storage; Light / Dark paper (2026-11-11)

**Storage links.** vpavlin: a published link carried only the manifest's IPFS CID, so only
the publishing device (which kept "IPFS CID -> Storage CID" in its own table) could use the
Logos Storage copy. Now publishing copies the files to Storage first, writes the manifest with
each file's Storage CID (`"storage": { ipfs CID: [storage CID, size] }`), copies that too, and
the link carries the manifest's Storage CID: `fvbc://ipfs/<manifest>?ls=<storage cid>#key=…`.
A reader uses the link's `ls` and the manifest's map for any block IPFS does not deliver;
every block is still checked against its IPFS CID. Without Storage, publishing falls back to
the IPFS-only link. Old links open as before. Storage is now always offered as a source.
Tests: `a_link_carries_the_manifests_storage_cid`, `the_manifest_names_where_each_file_is_in_storage`.

Found on the way: storage_module gives `manifests()` (the list of every stored item) one
second (`DEFAULT_TIMEOUT_MS 1000`), so on a node with ~660 items it failed and publishing lost
its Storage CIDs. Uploads now learn their CID from storage_module's `storageUploadDone` event
(session id -> cid), with `manifests()` only as the fallback. A failed Storage fetch now says
why (no Storage CID known, the download call's error, or not finished in 90 s).

- Basecamp t: published "Storage link test": "Published to IPFS (ipfs_module) and Logos
  Storage", link with `?ls=zDvZRw…`.
- Basecamp u (gateways pointed at a dead address, no IPFS node): opening the link asked Logos
  Storage for the manifest by its `ls` CID; the transfer did not complete in 90 s. Both Storage
  nodes are behind NAT here ("TCP port mapping failed"); they connected and the block-exchange
  stream closed. Device-to-device Storage transfer between NATed nodes is a known limit (a
  reachable Storage node is needed); not verified end to end.
- Seen: u without an IPFS node waits 60 s for ipfs_module before trying gateways and Storage.

**Paper.** vpavlin: the Basecamp grey-on-beige is hard to read. Settings > Paper: Basecamp,
Light (white, black text) or Dark (very dark grey). It colours the visual editor, the Markdown
source and preview, documents, tables, code, diagrams and slides; the rest of the app keeps
Basecamp's colours. Kept by the core (`set_pref`, the snapshot's `prefs`). Harness: doc-light,
doc-dark, editmd-light, edit-light, settings. Basecamp t: switched Light and Dark on an open
document; `settings.json` keeps `"prefs": {"paper": "dark"}`.

## 0.14.7: inline code stands out (2026-11-11)

vpavlin: inline code (a path in backticks) must be highlighted. It was not: the reading view
(documents, preview, slides, tables) used Qt's styled text, which ignores <code>, and the
editor showed only a monospace font. Now inline code is monospace in the accent colour on a
tinted background everywhere (reading view and tables as rich text, links tinted as before;
in the editor the core's <code> and the typed / toolbar code carry the style). After a typed
`code` or the toolbar's code the editor puts a zero-width space outside the span so typing
goes on plain; the core drops it (`inline_code_keeps_no_marker_after_it`).

- Harness editmd (preview with inline code and a link). Basecamp v: typed `~/.local/share/x`
  and toolbar code on a selected word, text typed after both plain (cases5.py).

## 0.14.6 (view): lists set in from the text (2026-11-11)

vpavlin: documents read condensed, and top-level list items sat at the left edge with the
text, so lists, paragraphs and headings were hard to tell apart. The reading view (documents,
the Markdown preview, slides) drew a list item as "•  text" with no indent. Now list items are
indented 28 px (24 more per level) with the bullet, number or checkbox in a hanging margin, so
wrapped lines align with the text; nested bullets are hollow. A little more space above H1/H2
and after paragraphs; list items stay close. The visual editor already indented lists (Qt).

- Harness: doc mode (the SDK documentation's list), before and after.

## 0.14.5 (view): the cursor after undo (2026-11-11)

vpavlin: after undo the cursor jumped to the end of the document or the next line. Undo put
it back at the step's start, counted before the editor reloaded. Now it goes where the text
changed (the end of what came back, or where the removed text was), found by comparing the
text before and after; after undoing a formatting change it stays where it was. Steps now
close just before a space or Enter, so undo removes " word" with its space, as Google Docs does.

Tests (tools/gui-tests/cases4.py, 15): undo, then type on: the letter lands where the undone
word was, in the visual and the Markdown editor, also after redo. Reproduced first on 0.14.4
(the letter went to the start of the next line).

## 0.14.4: behave like Google Docs, LibreOffice and ddocs.new (2026-11-11)

vpavlin: the editor should do what people are used to. Compared with ddocs.new's editor
(read in fileverse-ddocs: Tiptap with CommonMark emphasis rules, tab-indent.ts, Yjs undo)
and the documented defaults of Google Docs and LibreOffice Writer; where they differ,
ddocs.new's way.

- Typed emphasis: `**b**`, `__b__` bold, `*i*`, `_i_` italic, `~~s~~` strike, `` `c` `` code
  as the closing marker is typed, with ddocs.new's CommonMark rules (`5 * 3 * 2` and
  snake_case stay text); the text typed after it is plain again.
- Shift+Enter: a line break inside the paragraph, written as Markdown's backslash break; one
  at the start or end of a block (Qt writes empty lines as `<br />`) is dropped
  (`shift_enter_is_a_line_break`). Shift+Enter at the end of a heading keeps the heading.
- Tab in text and headings: an indent (four no-break spaces; Qt collapses other inserted
  spaces), Shift+Tab takes it back; the core keeps leading no-break spaces (`a_tab_indent_survives`).
- Undo: Fileverse's own history instead of Qt's (which merged all typing since the last
  formatting step, and was wiped by every remote edit or reload). Steps end at a word (space
  or Enter), a 0.7 s pause, or a formatting change; a step keeps only what it changed with
  some text around it, so undo finds that place in the current text and restores it, leaving
  other people's edits alone. Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y and the toolbar, in both editors.

Tests (tools/gui-tests: cases3.py 14, cases4.py 11, undo_shared.py): all pass (Enter then two
undos removes the line with its Enter, which is how the steps are cut). Shared document, t
and u on Logos: t typed a word, u typed a word, t pressed Ctrl+Z: on both, t's word gone and
u's kept. u's word took a minute to reach t over Logos (network, as before).

## 0.14.3: editor UX pass, three people on one document (2026-11-10)

vpavlin asked for a thorough editor test (formatting, Tab leaving the editor, double Enter
not ending a list) and a test with three people. Scripted with xdotool in Basecamps t, u, v
(tools/gui-tests: edtest.py drives the editor and reads the saved Markdown back after each
case; cases1.py, cases2.py: 66 + 9 cases; collab3.py: three-person rounds, frames of every
screen). Found and fixed:

- Typed shortcuts (`# `, `- `, `1. `, `[] `, `> `) failed on a line the editor had restyled
  while empty, e.g. after Enter at the end of a heading: it holds a zero-width space.
- Double Enter did not end a list, task list or quote (new empty items kept coming); now an
  empty item ends the list (a nested one goes up one level), an empty quote line ends the
  quote, an empty last line of a code block leaves it. An empty line in a quote is a quote
  line for the core (`an_empty_line_in_a_quote_is_the_quotes`).
- Tab left the editor (and later keys went nowhere): Basecamp hosts the view in a
  QQuickWidget, which hands Tab to QML only when an item after the focused one can take
  focus. An invisible tab stop after the editor keeps it in; Tab nests a list item and
  Shift+Tab lifts it (core: `indent` / `outdent`, `tab_nests_list_items_and_shift_tab_lifts_them`),
  Tab in code indents (no-break spaces, written as spaces: `code_keeps_its_indentation_in_the_editor`),
  elsewhere it does nothing; in the Markdown editor Tab inserts four spaces.
- Ctrl+K did nothing: Basecamp binds it (search); the editor takes it (shortcut override).
- Tab or Enter in the title went to the buttons; now into the text.
- Ctrl+B / Ctrl+I with nothing selected did nothing; now they switch the style of what is
  typed next, on and off.
- Enter followed at once by typing could put the first letter before the line break: the
  Enter handler asked the core for the block kind synchronously, which let the next key in
  first. It uses the kind the editor keeps up to date now.

Still as before (noted, not changed): typed `**bold**` stays text; Ctrl+Z undoes everything
typed since the last formatting step; Shift+Enter is a space in Markdown.

Three people (t, u, v on Logos; recording in reports/, not in the repo): join, text on both
joiners after 9 s; each in turn; all three at once in different lines (identical after 12 s,
every sentence intact); all three at once at the same place (identical after 17 s, each
person's letters together). One lag: v got t's last line of round 1 only with the next edits
(over 2 min), on the transport side. On "both", one Basecamp's p2panda_core.open timed out
under load and reflection_core then could not publish that note on Logos either (reported to
jimmy-crib/vpavlin). Early runs were garbled by X key auto-repeat on the loaded test machine
(`xset r off` fixes the rig).

## 0.14.2: Reflection 0.6 networks (2026-11-10)

reflection_core 0.6.0 (jimmy-crib/vpavlin, same day) publishes on Logos, p2panda or both by its
`networks` setting, with no per-note bridge. fileverse_core requires reflection_core ~0.6.0 (LIDL
refreshed from its package; only doc comments changed); the chip shows the networks the running
node uses (`state().networks`) and its tooltip each network's status (`networkStatus`).

- Two Basecamps with reflection_core 0.6.0, p2panda_core 0.2.0 and the Reflection view 0.6.0
  (sha256 checked against the public catalogue): Settings showed Logos / p2panda / Both; Both
  in each (reflection_core's settings.json: `"networks": "both"`); a new document shared in t
  ("shared · Logos + p2panda"), joined in u (text, name, "2 here"), typed both ways, both carets;
  tooltip "Logos: connected, 29 peers · p2panda: connected, 1 peers in the last 5 min".
  This is the "two Basecamps both on both" case jimmy-crib had not run.

## 0.14.1 (view): "Local" and one My files list (2026-11-10)

vpavlin: "Drafts" was confusing. The section is "Local" now (documents kept on this device;
"draft" is gone from the view's wording), and My files lists the portal's Fileverse files and
the local documents together, newest first, each with a chip (Fileverse, Local, plus Shared
or Published, and the backup chips), with All / Fileverse / Local / Shared and the title
filter. Without an imported backup key it shows the local documents and "Import your
Fileverse files…". A local row opens in the editor, a Fileverse row as before.

- Harness: portal (merged list with both kinds), localfiles (no portal), edit (Local sidebar).
- Basecamp t: My files with 19 local documents, Shared chips on the Reflection-linked ones.

## 0.14.0: shared drafts are Reflection notes only (2026-11-10)

vpavlin: one format, so anyone with Fileverse can join anything shared from Fileverse. HashSeq
removed (engine, loam_core link, Name and Sync speed settings); reflection_core is a required
dependency; "Share…" makes the draft a Reflection note at once; one Join field; the header chip
shows the networks ("shared · Logos · 2 here"); Settings has "Shared drafts: networks"
(Logos / p2panda / Both) once reflection_core has a `networks` setting, and says which network
it runs on until then. Drafts shared with HashSeq are private drafts now. The fvdraft skill
treats Reflection-linked drafts as shared. flake.lock: 17 MB to 0.5 MB without loam_core's input.

- cargo: 63 engine tests pass without the collab module.
- Harness: shared (chip, carets, no choice panel), settings (the Networks choice with a
  reflection_core that reports `networks`).
- Two Basecamps: a new draft in t, Share (one click: "shared · Logos · only you here"); in u,
  Join with its id: the text and name arrived, "2 here"; typing both ways and both carets;
  Settings in t: "now on Logos", no choice (reflection_core 0.5.0 has no `networks`).
- Also checked the day before: Fileverse loads when reflection_core is missing (0.13.0).
  Not checked: that Basecamp's package manager installs reflection_core with Fileverse.

## 0.13.0: drafts as Reflection notes, through reflection_core (2026-11-09)

"Share…" offers, when reflection_core is installed, "As a Reflection note": the draft's text
goes into a new note of reflection_core and the draft stays linked to it; the join panel
joins a note by id or opens one from the REFLECTION NOTES list. Fileverse calls
reflection_core only through fileverse_core (`reflection_call`, a fixed list of methods),
over Basecamp's module calls; nothing of Reflection (GPL) is linked in. Shared with
Fileverse (HashSeq) stays as it was.

- Basecamp loads a module only for a caller that declared it, and the builder wants a LIDL
  contract for every declared dependency: fileverse_core lists reflection_core as an optional
  dependency with its contract vendored (`contracts/reflection_core.lidl`,
  `dependency_overrides`).
- Empty lines: Fileverse's `&nbsp;` paragraphs go onto the note as a line holding one space
  and come back as empty lines (any line of only blanks, outside code blocks), with cursor
  offsets mapped across. First tried with a no-break space: Reflection's editor reads one
  back as a plain space, its text never matches the note's again, and it stops showing
  remote edits (found in the two Basecamps: u's Reflection kept "Fil" of a line t's
  Fileverse had finished, while u's reflection_core had the whole line).
- A reply to a poll sent before a save could replace newer text in the editor: only a newer
  version than the editor's is applied now.
- A draft made for a note takes the note's name (Reflection names notes by their first line).
- "Share my cursor" for Reflection notes: reflection_core sends a cursor only when this
  device's setting is on (the same setting as Reflection's own switch, off by default).
- Two Basecamps (t: Fileverse, u: the Reflection app, then Fileverse too): a note shared
  from t's Fileverse, joined in u; typing in t's Fileverse shows in u's Reflection and the
  other way round, a word replaced mid-line, a line added at the end, both typing at the
  same time (converged, nothing lost), the empty line kept on both; u's Reflection caret
  shown in t's Fileverse at the right place; with "Share my cursor" on in t, t's caret in
  u's Reflection at the right place. A note made in u's Reflection joined by id in t's
  Fileverse (text and name arrive); "Unlink note" leaves a private draft. The join panel
  lists the Reflection notes also while one is open (filled from the open note's poll).
  Shared with Fileverse (HashSeq) again on a new share, joined in u, typed both ways.

## 0.12.2: live sync speed, cursors, empty lines, sending while typing (2026-11-09)

vpavlin on 0.12.1: text arrives slowly, cursors jump (moving one seemed to move the other's),
empty lines are missing on the other side, and nothing is sent while you keep typing.

- Speed: "Sync speed" in Settings, Live (default: save 0.2 s after a keystroke and every
  0.2 s while typing, send within 0.3 s, poll 0.3 s, cursors 4/s) or Economy (2 s, RLN
  budget).
- Cursors: the view mapped visual-editor positions to Markdown by the text before the caret
  with brackets stripped, so escaped text (`\[t typing\]`) never matched and carets fell
  back to a proportional guess. Now through a visible-text map (markers, emphasis, escapes,
  link targets removed), against the core's current text.
- Empty lines: Markdown has none, so the editor's empty paragraphs were dropped (and a caret
  on one landed in the next line). Written as `&nbsp;` paragraphs now, shown as empty
  paragraphs again (no hidden character), skipped by slides. cargo:
  `empty_lines_survive_the_round_trip`.
- Typing on during a save: the core returns the edit version (exactly the text sent); the
  next text builds on it. Found with `typing_on_while_others_change_the_text_loses_nothing`:
  a rebased insert anchored "after the left neighbour" landed after others' new line;
  rebased inserts now follow Fugue's rule as HashSeq's own inserts do.
- Two Basecamps: two empty lines and a line typed without pause in t: u showed the empty
  lines, the line growing while t typed (t's caret at its end), and all of it after 11 s
  (the relay dropped some of the small messages; the heads after the burst repaired them).

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
