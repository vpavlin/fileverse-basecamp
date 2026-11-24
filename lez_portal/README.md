# lez_portal: Fileverse portals on LEZ

A LEZ program (LEZ **v0.4.0-rc3**, the actor model) that replaces Fileverse's Gnosis
portal contracts (`FileversePortal`, the registry, and the v3 App's file API): who owns a
portal, who may write to it, the collaborators' wrapping keys, and the list of files as
CIDs. No secret is on chain: content and keys are encrypted blobs in Logos Storage / IPFS,
as before.

Status: **the logic is tested in-process (30 tests); it has not run on a chain yet.** See
[What is not verified](#what-is-not-verified).

## Layout

| path | what |
|---|---|
| `core/` (`portal_core`) | state, messages, events, PDA seeds and address derivation. Shared by the guest and clients. |
| `program/` (`portal_program`) | the actor handlers + an in-process conversation simulator for tests. |
| `client/` (`portal_client`) | no-I/O client: builds messages and **access lists** (`Plan`), derives addresses, decodes state. `examples/addr.rs` prints any derived address. |
| `methods/guest/` | the risc0 guest (`actor_entrypoint!`), built by `scripts/build-guest.sh`. |
| `artifacts/portal-idl.json` | IDL v2 for `spel` (generated, see below). |
| `scripts/e2e.sh` | deploy + full flow with `spel`. |
| `tools/idl-gen`, `tools/risc0-packager` | adapted from `logos-blockchain/lez-programs`. |

## Model

On LEZ v0.4 a program is a set of **actors**: an actor is an `(account, program)` pair and a
handler receives one message for one actor and answers with a `Response` (new state, sync
or async messages to other actors, events). This program's actors:

| actor | account | state |
|---|---|---|
| portal | PDA `portal(owner, i)` | owner, pending owner, owner DID, metadata CID, collaborators, collaborator keys, key verifiers, `file_count` |
| file | PDA `file(portal, index)` | one file: `app_file_id`, type, metadata/content/gate CIDs, version, `deleted`, last editor |
| app file | PDA `appfile(portal, app_file_id)` | `{portal, index}`: v3's `appFileIdToFileId` |
| registry head | PDA `registry` | `portal_count` |
| all entry | PDA `all(n)` | the n-th portal ever created (= `allPortal`) |
| owner head | PDA `owner(owner)` | `count` of portals listed for the owner |
| owner entry | PDA `owned(owner, i)` | the owner's i-th portal (= `ownedPortal(owner, i)`) |

Seeds are `sha256("fvp/<kind>" ‖ parts…)`; the PDA is `AccountId::for_public_pda(program, seed)`
where `program` is the **header account `spel deploy` prints** (each deploy is a new program
address, so a new set of portals).

### Access control

A caller never messages a portal directly. It sends `Message::Act { portal, op }` to **its own
actor** (its account signs); that actor forwards `Internal::Op` to the portal, which sees who
forwarded it and checks the owner and collaborator lists. This is how the token program lets
a mint authority speak. Consequences:

* the transaction's root is the caller's account, and it is the only signer;
* every other actor the conversation reaches must be in the transaction's access list. The
  client's `Plan::actors` is that list; `spel send --actor …` takes the same.

### Operations (`PortalOp`) ↔ Solidity

| Solidity | here | who |
|---|---|---|
| mint (registry) / constructor | `Message::CreatePortal` | the owner (signs) |
| `addCollaborator` / `removeCollaborator` | `AddCollaborator` / `RemoveCollaborator` | owner |
| `registerCollaboratorKeys` / `removeCollaboratorKeys` | `RegisterKeys` / `RemoveKeys` | member for self; owner for anyone |
| `updateKeyVerifiers` | `UpdateKeyVerifiers` (replaces the list; index = version) | owner |
| `updatePortalMetadata`, `ownerDid` | `UpdateMetadata`, `SetOwnerDid` | owner |
| `transferOwnership` / `acceptOwnership` | `TransferOwnership` / `AcceptOwnership` | owner / pending owner |
| `addFile` / `editFile` | `AddFile` / `EditFile` | owner or collaborator |
| `deleteFile` / `deleteFileByAppFileId` | `DeleteFile` / `DeleteFileByAppFileId` | owner or collaborator |
| `getFileCount`, `files(i)` | `Portal.file_count`, read the file actor `i` | anyone |
| events | `portal::<Name>` events on the portal actor | |

* **Enumeration needs no events and no subgraph.** File ids are sequential: read the
  portal's `file_count`, then the file actors `0..file_count` (`PortalClient::file_accounts`).
  A deleted file stays as a **tombstone** (`deleted = true`, empty CIDs).
* **Registry**: `owner` → portals is `owner head.count` + the owner entry actors; all portals
  is `registry head.portal_count` + the all-entry actors (`PortalClient::all_entry_accounts`
  pages through it). A portal moved by `AcceptOwnership` is added to the new owner's list;
  the old owner's entry stays, so **filter a list by `portal.owner == owner`**.
* Registry and owner lists are written by the portal's own creation conversation, so a
  portal cannot be listed under someone who did not sign its creation.

### Differences from the Solidity contracts (decide whether they matter)

* A portal is created *by its owner*: the account `portal(owner, i)` is derived, not chosen.
* Owner and collaborators are LEZ **accounts** (public keys), not EVM addresses; a collaborator's
  `view_did`/`edit_did` strings are stored as in v1.
* Limits (caps keep every message that loads the portal cheap): 128 collaborators,
  64 key verifiers, DIDs ≤ 256 bytes, CIDs ≤ 128 bytes, `app_file_id` ≤ 128 bytes.
* `key_verifiers` is a list of 32-byte hashes (index = version). If Fileverse's contract
  stores something else, change `Portal::key_verifiers` and `UpdateKeyVerifiers`.
* `app_file_id` is unique per portal and immutable. `EditFile` replaces type, CIDs and version.
* No gasless forwarder: the caller (or `--fee-payer`) pays LEZ fees from a wallet account.
* Comments (`CommentModule`) are not implemented.
* Concurrency: `AddFile`/`CreatePortal` derive new addresses from counters read beforehand.
  If another transaction moved the counter, the wallet's simulation refuses the transaction
  (nothing is sent, no fee). Re-read and retry.

## Build and test

```bash
cargo test --workspace            # 30 tests, no chain needed (RISC0_DEV_MODE not required)
```

`Cargo.lock` is checked in on purpose: `risc0-zkvm =3.0.5` is **yanked** on crates.io, so the
workspace only resolves against a lockfile that already has it (copied from lez-programs).
The guest has its own lockfile (`methods/guest/Cargo.lock`) for the same reason.

### Guest binary (no Docker)

```bash
curl -L https://risc0.com/install | bash && rzup install cargo-risczero 3.0.5 && rzup install rust
scripts/build-guest.sh            # -> target/guest/portal.bin
```

This uses the host `+risc0` toolchain; lez-programs' Docker build gives a reproducible
ImageID instead. The ImageID is not the program's address, so a different ImageID is
harmless for a local chain.

### IDL

```bash
cargo run -q --manifest-path tools/idl-gen/Cargo.toml -- core --events event > artifacts/portal-idl.json
# or, with spel from logos-co/spel@refactor/lez-v0.4.x:
spel generate-idl core --events event > artifacts/portal-idl.json
```

## Run on a local chain

1. A LEZ v0.4.0-rc3 sequencer (standalone) and wallet, `LEE_WALLET_HOME_DIR` set.
2. `spel` from logos-co/spel branch `refactor/lez-v0.4.x` (`cargo install --path spel-cli`).
3. Public accounts labelled `alice`, `bob`, `payer` (payer funded); export `ALICE_ID`, `BOB_ID`.
4. `scripts/build-guest.sh && scripts/e2e.sh`.

## Using the client from Rust

```rust
use portal_client::{PortalClient, Cids, FileType};

let client = PortalClient::new(program_address);              // from `spel deploy`
// read counters from chain with the wallet, decode with PortalClient::decode(bytes)
let plan = client.add_file(caller, portal, file_count, "doc-1".into(),
                           FileType::Private, Cids { metadata, content, gate }, 1);
// plan.root signs; plan.message_bytes() is the Borsh message; plan.actors is the access list
```

Submitting the transaction (and reading actor state) is the wallet's job: follow
`logos-co/logos-rln-lez-module`, which owns a wallet through LEZ `core_ffi`.

## What is not verified

* **Nothing has run on a chain.** Handlers are tested through an in-process simulator that
  applies the runtime's rules as I read them from `lee_core` (seed grants authorize exactly
  the derived PDA; sync messages run depth-first). If the runtime differs (e.g. a limit on
  conversation depth: create-portal is five actors deep, owner actor → owner head → portal →
  registry → all entry), the first e2e run will show it.
* `scripts/e2e.sh` has never been run; CLI details (JSON enum arguments, `--actor` handling)
  follow `docs/reference/cli.md` of the spel branch.
* `scripts/build-guest.sh` is derived from lez-programs' Docker build and untested here.
* The v4 state layout is **append-only from here**: variants of `PortalState`, `PortalOp`,
  `Message` and `Internal` must only be added at the end.
