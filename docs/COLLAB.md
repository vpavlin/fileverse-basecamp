# Collaborative editing over Logos Delivery: analysis and plan

Status: implemented in 0.12.0 with HashSeq (vpavlin chose HashSeq first, Loro as the fallback; private drafts, shared only on request; cursors from the start; text only). Proposal written 2026-11-09. Sources read: vpavlin/reflection `feat/logos-delivery`
@ e25cf05 (docs/logos/spec.md, ADR 0001-0008, logos-p2panda), vpavlin/reflection-basecamp
@ 1f86daa (reflection_core), fileverse/collaboration-server (Fileverse's RTC), and
davidrusu/hashseq (README).

## What exists

**Reflection over Logos** (vpavlin's fork of p2panda Reflection, spec version 1):
- Text is a **Loro** document. Every edit is a p2panda operation: ed25519-signed,
  per-author, hash-chained (seq, backlink), with a Loro update as its body, or a full Loro
  snapshot with a prune flag.
- Transport: plain relay through **loam_core** (the host's shared Delivery node) on two
  content topics per document (`-ops`, `-eph`), derived from a network id N and the
  document id T. **T is the shared secret** (it travels as a 64-hex id); every frame is
  XChaCha20-Poly1305 with a key derived from N and T, and a deterministic nonce.
- Catch-up without store nodes: peers that open a document broadcast `heads`; whoever has
  newer operations repairs after a random delay, suppressed if someone else answered.
- RLN-aware: only the open document publishes (heads on opening, presence pings every
  120 s); edits are merged into at most one operation every few seconds, then a snapshot.
- The basecamp core (reflection_core) owns the CRDT and the sync, and the view sends the
  whole text plus the version it edited from; the core rebases and merges it.
- Reusable: the spec and the `logos-p2panda` crate (opaque bodies; needs p2panda-core and an
  sqlx SQLite pool). reflection_core itself is Reflection-specific.

**Fileverse's own real-time collaboration** (ddocs.new): Yjs updates and awareness,
encrypted on the client with a room key, relayed and stored (MongoDB) by Fileverse's
collaboration server, with edit access gated by a ZK proof and UCANs. Its unit is the
ProseMirror XML structure of a dDoc, and it needs Fileverse's server.

**HashSeq** (David Rusu): a Byzantine-fault-tolerant CRDT family (sequences with marks,
maps, moves, a hash-DAG object store) for open networks: every operation is content
addressed (BLAKE3), causality is the set of heads an author saw, no actor ids or clocks to
forge, Fugue ordering (no interleaving), fast (4-5.7M edits/s), snapshots 2-11x the text.
Pre-release 0.1, little adoption, ships a block-editor demo synced through a relay.

## What Fileverse drafts are

A draft is **Markdown text** (plus pictures kept with it). The visual editor converts to
and from Markdown; slides, diagrams and publishing all start from that text. So the thing
to share is the Markdown text, exactly the unit Reflection shares.

## Options for the CRDT

| | Loro (as Reflection) | Yjs (yrs) Y.Text | HashSeq |
|---|---|---|---|
| Reuse | spec + logos-p2panda + reflection_core's merge logic as-is | spec + logos-p2panda; own rebase (yrs has no fork-at-version) | spec's transport idea only; its own DAG sync |
| Interop | **a shared draft is a Reflection note**: editable in Reflection GTK and Reflection for Basecamp (same N) | none (ddocs.new collaborates on the dDoc XML, over its own server) | none |
| Maturity | 1.x, used by Reflection | mature, already in our core | 0.1, pre-release |
| Malicious member | can corrupt (forged ids), but every op is signed, so the author is known | same | designed for it |
| Size | +loro (several MB) | nothing new | small |

Fileverse compatibility is not affected by the choice: publishing converts the Markdown
into a Fileverse (Yjs) document, as today, whichever CRDT holds the text while it is being
written.

## Recommendation

**Version 1: Reflection protocol v1 unchanged, with Loro text, and Reflection's network
id**, so a shared Fileverse draft and a Reflection note are the same thing. Use
`logos-p2panda` as a dependency (same operations, sealing, heads/repair, RLN behaviour)
and loam_core as the transport, following reflection_core's split (an IPC thread for
loam_core, never a blocking cross-module call on the module thread).

Keep the sync layer CRDT-agnostic (opaque bodies), so that **HashSeq** can be tried later
for open-membership documents (anyone may write, no shared secret), where its BFT design
matters; for invited collaborators (shared secret) Loro with signed per-author logs is
enough.

## Plan

1. **Core: shared drafts.** A draft gets `collab: { id }` (T, 32 bytes, the secret).
   Methods: `collab_share(draft_id)` (start sharing; returns the id to send),
   `collab_join(id, title)` (a new draft from a shared one), `collab_state(draft_id)` (text,
   version, peers, transport status), `collab_set_text(draft_id, base_version, markdown)`
   (merge, as reflection_core's `set_text`), `collab_set_active(draft_id | "")`,
   `collab_leave(draft_id)` (stop syncing; the draft stays). The Loro document and the
   operation log live in the module's data directory. The draft's Markdown on disk follows
   the shared text, so slides, the preview, the agent skill and publishing keep working.
2. **Transport:** depend on loam_core (module dependency). Needs loam_core 0.7.3 or later
   (relay by default) in the repos: lan03 and public have up to 0.7.2.
3. **View:** "Share for editing" on a draft (copy the id; a QR later), "Join a shared
   draft" (paste an id). The editor sends the whole Markdown with the version it edited from
   (debounced), polls `collab_state` every second while open, and applies remote changes
   keeping the cursor (the 0.11 reload path). A peers indicator (online = has it open).
4. **Pictures in shared drafts:** pictures are files on the author's device. Before
   sharing, upload them encrypted (as publishing does) and rewrite their references to
   `cid + key`, so peers fetch them. Version 1 may start text-only with a warning.
5. **Tests:** cargo tests with an in-memory bus (two peers converge, late joiner catches
   up, rebase of a stale edit), two headless logosctl sessions over loam_core, then the
   GUI rig; and a draft opened in Reflection for Basecamp, if we keep the network id.

Not in version 1: cursors of others (Reflection has them; later), read-only members,
revocation, documents over 140 KiB of snapshot, co-editing with ddocs.new.

## What was built (0.12.0)

- `fileverse_core/rust-lib/src/collab.rs`: the engine (HashSeq in a `HashWeb`, one sequence
  object per shared draft, origin derived from `T`). Frames: `ops` (HashSeq delta `0xDE`),
  `heads` (8-byte nonce + HashSeq frontier `0xC1`), `eph` (JSON presence/cursor). Sealed with
  AES-256-GCM, key `sha256("fileverse-collab-v1" ‖ "key" ‖ T)`, nonce `sha256(key ‖ pt)[..12]`;
  topics `/fileverse/1/<hex(sha256(.."topic"..T))[..16]>-ops/bin` and `-eph/bin`.
- Repairs answer heads after 50-500 ms with `deltas_for(their clock)`, skipping operations
  seen on the wire meanwhile; a draft with orphans (missing dependencies) asks with heads.
  Big deltas are split under 120 KiB; HashSeq buffers what arrives before its dependencies.
- Only the open draft publishes: heads 0.5/5/30 s after opening, presence on open/close and
  every 120 s (and once in reply to someone new), local edits at most every 2 s, cursors at
  most every 1 s.
- Stale edits from the view are rebased by element id (inserts anchor to the element they
  followed in the edited version, removals name elements). Cursors are element ids.
- Glue: `collab_*` methods, a runtime started on first use or when a shared draft exists,
  loam_core (optional dependency, ~0.7.3, relay) on dedicated threads, the HashSeq
  snapshot in `collab/<id>.hs`, the text mirrored into the draft file.

HashSeq notes: no API for "my operations since X", so the engine keeps the clock of its last
send and sends `deltas_for` it. Everything else mapped directly (frontier = heads,
`deltas_for` = repair, orphan buffering = gap handling). Nothing fishy found so far.
