//! Fileverse portals on LEZ: core types shared by the program, the guest and clients.
//!
//! # Model
//!
//! A portal is a set of *actors* of this program, each on its own account:
//!
//! | actor | account | holds |
//! |---|---|---|
//! | portal | PDA `portal(owner, i)` | owner, collaborators, keys, file count |
//! | file | PDA `file(portal, index)` | one file entry (a deleted file stays as a tombstone) |
//! | app file | PDA `appfile(portal, app_file_id)` | the file index of an `app_file_id` |
//! | registry head | PDA `registry` | how many portals exist |
//! | all entry | PDA `all(n)` | the n-th portal ever created |
//! | owner head | PDA `owner(owner)` | how many portals an owner has been listed for |
//! | owner entry | PDA `owned(owner, i)` | the owner's i-th listed portal |
//!
//! Callers never message a portal directly. A caller sends [`Message::Act`] to *its own*
//! actor (its account signs), which forwards the operation to the portal; the portal sees
//! who forwarded it and checks the owner and collaborator lists. This is how the token
//! program lets a mint authority speak.
//!
//! Every enum here is append-only: state outlives code and Borsh encodes the variant as a
//! leading tag byte.
//!
//! # Limits
//!
//! The portal's collaborator and key lists live in one actor state and are capped (see the
//! `MAX_*` constants) to keep every message that loads the portal cheap. File entries are
//! one actor each and unbounded in number.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::{account::AccountId, program::PdaSeed};
use risc0_zkvm::sha::{Impl, Sha256 as _};
use serde::{Deserialize, Serialize};

pub mod event;

/// Most collaborators one portal lists (the owner is separate).
pub const MAX_COLLABORATORS: usize = 128;
/// Longest CID or other content address, in bytes.
pub const MAX_CID_LEN: usize = 128;
/// Longest `app_file_id`, in bytes.
pub const MAX_APP_FILE_ID_LEN: usize = 128;
/// Longest DID string (view/edit/owner), in bytes.
pub const MAX_DID_LEN: usize = 256;
/// Most key-verifier hashes a portal stores.
pub const MAX_KEY_VERIFIERS: usize = 64;

/// Visibility of a file, as in Fileverse's `FileType`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub enum FileType {
    Public,
    Private,
    Gated,
}

/// The program's state on one account. An empty actor state means "nothing here yet" and
/// is not a `PortalState`.
#[spel_attrs::state]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum PortalState {
    Portal(Portal),
    File(File),
    AppFile(AppFile),
    RegistryHead(RegistryHead),
    AllEntry(PortalRef),
    OwnerHead(OwnerHead),
    OwnerEntry(PortalRef),
}

impl PortalState {
    /// Decodes an actor state; `None` for an empty one.
    ///
    /// # Panics
    /// If the bytes are not a `PortalState`. Only this program writes its actor state.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        Some(borsh::from_slice(bytes).expect("portal actor state must decode"))
    }

    /// Like [`decode`](Self::decode), for clients: `Err` instead of a panic.
    ///
    /// # Errors
    /// If the bytes are not a `PortalState`.
    pub fn try_decode(bytes: &[u8]) -> Result<Option<Self>, borsh::io::Error> {
        if bytes.is_empty() {
            return Ok(None);
        }
        borsh::from_slice(bytes).map(Some)
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization is infallible")
    }
}

/// A portal: who owns it and who may add and edit files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Portal {
    pub owner: AccountId,
    /// Set by `TransferOwnership`, cleared by the new owner's `AcceptOwnership`.
    pub pending_owner: Option<AccountId>,
    pub owner_did: String,
    /// CID of the portal's metadata.
    pub metadata_cid: String,
    /// Accounts that may add and edit files, besides the owner.
    pub collaborators: Vec<AccountId>,
    /// Public keys collaborators (and the owner) registered to wrap file keys.
    pub collaborator_keys: Vec<CollaboratorKeys>,
    /// Hashes of the portal keys, one entry per key version (index = version).
    pub key_verifiers: Vec<[u8; 32]>,
    /// Number of files ever added; file ids are `0..file_count`, tombstones included.
    pub file_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct CollaboratorKeys {
    pub account: AccountId,
    pub view_did: String,
    pub edit_did: String,
}

/// One file of a portal. A deleted file keeps its slot so enumeration still works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct File {
    pub portal: AccountId,
    pub index: u64,
    pub app_file_id: String,
    pub file_type: FileType,
    pub metadata_cid: String,
    pub content_cid: String,
    pub gate_cid: String,
    pub version: u32,
    /// Set on delete; the CIDs are then empty.
    pub deleted: bool,
    /// The account that last added, edited or deleted the file.
    pub last_editor: AccountId,
}

/// Where a file's `app_file_id` points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AppFile {
    pub portal: AccountId,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct RegistryHead {
    pub portal_count: u64,
}

/// A registry entry: the portal at this slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct PortalRef {
    pub portal: AccountId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct OwnerHead {
    pub owner: AccountId,
    pub count: u64,
}

/// Operations on a portal, forwarded by the caller's own actor via [`Message::Act`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum PortalOp {
    /// Owner only.
    AddCollaborator { who: AccountId },
    /// Owner only. Also drops the collaborator's registered keys.
    RemoveCollaborator { who: AccountId },
    /// The owner or a collaborator registers their own keys (replacing earlier ones).
    RegisterKeys { view_did: String, edit_did: String },
    /// The owner, or `who` themself.
    RemoveKeys { who: AccountId },
    /// Owner only: replace the key-verifier hashes (index = key version).
    UpdateKeyVerifiers { verifiers: Vec<[u8; 32]> },
    /// Owner only.
    UpdateMetadata { metadata_cid: String },
    /// Owner only.
    SetOwnerDid { owner_did: String },
    /// Owner only: name the next owner, who must `AcceptOwnership`.
    TransferOwnership { new_owner: AccountId },
    /// The pending owner only.
    AcceptOwnership,
    /// Owner or collaborator. Takes file id `file_count`; `app_file_id` must be new.
    AddFile {
        app_file_id: String,
        file_type: FileType,
        metadata_cid: String,
        content_cid: String,
        gate_cid: String,
        version: u32,
    },
    /// Owner or collaborator. Replaces the file's type, CIDs and version.
    EditFile {
        index: u64,
        file_type: FileType,
        metadata_cid: String,
        content_cid: String,
        gate_cid: String,
        version: u32,
    },
    /// Owner or collaborator. Leaves a tombstone.
    DeleteFile { index: u64 },
    /// Owner or collaborator. Same as `DeleteFile`, found through the `app_file_id`.
    DeleteFileByAppFileId { app_file_id: String },
}

/// Messages anyone may send: a transaction author or another program.
#[spel_attrs::message]
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Message {
    /// Create a portal owned by the receiving actor's account.
    ///
    /// Sent to the caller's own actor, which must be authorized. The portal lands at
    /// `Addresses::portal(owner, i)` where `i` is the owner head's current count, and is
    /// listed in the registry and under its owner.
    CreatePortal {
        owner_did: String,
        metadata_cid: String,
    },

    /// Do `op` on `portal` as the receiving actor's account.
    ///
    /// Sent to the caller's own actor (the account of the owner or collaborator), which
    /// must be authorized. The portal checks the caller against its owner and collaborators.
    Act { portal: AccountId, op: PortalOp },
}

/// Messages only portal-program actors send each other. The guest decodes these only when
/// the sender is this program, so each is trusted because only this program's code can
/// have produced it.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum Internal {
    /// Caller's actor -> portal.
    Op { op: PortalOp },
    /// Portal -> file actor (new file).
    WriteFile {
        index: u64,
        app_file_id: String,
        file_type: FileType,
        metadata_cid: String,
        content_cid: String,
        gate_cid: String,
        version: u32,
        editor: AccountId,
    },
    /// Portal -> file actor.
    UpdateFile {
        file_type: FileType,
        metadata_cid: String,
        content_cid: String,
        gate_cid: String,
        version: u32,
        editor: AccountId,
    },
    /// Portal or app-file actor -> file actor.
    TombstoneFile { editor: AccountId },
    /// Portal -> app-file actor (new file).
    BindAppFile { app_file_id: String, index: u64 },
    /// Portal -> app-file actor: delete the file this id points to.
    DeleteByAppFileId {
        app_file_id: String,
        editor: AccountId,
    },
    /// Owner's actor -> owner head.
    NewPortal {
        owner_did: String,
        metadata_cid: String,
    },
    /// Owner head -> the new portal.
    InitPortal {
        owner: AccountId,
        i: u64,
        owner_did: String,
        metadata_cid: String,
    },
    /// New portal -> registry head.
    RegisterPortal,
    /// Registry head -> all-entry actor.
    SetAllEntry { n: u64, portal: AccountId },
    /// Portal -> the new owner's head, when the owner accepts a transfer.
    OwnerAdd { owner: AccountId, portal: AccountId },
    /// Owner head -> owner-entry actor.
    SetOwnerEntry {
        owner: AccountId,
        i: u64,
        portal: AccountId,
    },
}

// --- Addresses ------------------------------------------------------------------------

fn seed_of(parts: &[&[u8]]) -> PdaSeed {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(part);
    }
    let digest = Impl::hash_bytes(&bytes);
    PdaSeed::new(
        digest
            .as_bytes()
            .try_into()
            .expect("a sha256 digest has 32 bytes"),
    )
}

/// PDA seeds of the program's derived actors. A seed plus the program's account id gives the
/// actor's account (see [`pda`]); a message that creates one is sent with its seed granted.
pub mod seeds {
    use super::{seed_of, AccountId, PdaSeed};

    #[must_use]
    pub fn registry() -> PdaSeed {
        seed_of(&[b"fvp/registry"])
    }

    #[must_use]
    pub fn portal(owner: AccountId, i: u64) -> PdaSeed {
        seed_of(&[b"fvp/portal", owner.as_ref(), &i.to_le_bytes()])
    }

    #[must_use]
    pub fn all(n: u64) -> PdaSeed {
        seed_of(&[b"fvp/all", &n.to_le_bytes()])
    }

    #[must_use]
    pub fn owner(owner: AccountId) -> PdaSeed {
        seed_of(&[b"fvp/owner", owner.as_ref()])
    }

    #[must_use]
    pub fn owned(owner: AccountId, i: u64) -> PdaSeed {
        seed_of(&[b"fvp/owned", owner.as_ref(), &i.to_le_bytes()])
    }

    #[must_use]
    pub fn file(portal: AccountId, index: u64) -> PdaSeed {
        seed_of(&[b"fvp/file", portal.as_ref(), &index.to_le_bytes()])
    }

    #[must_use]
    pub fn app_file(portal: AccountId, app_file_id: &str) -> PdaSeed {
        // Length-prefixed so (portal, id) pairs cannot collide.
        let len = u32::try_from(app_file_id.len()).unwrap_or(u32::MAX);
        seed_of(&[
            b"fvp/appfile",
            portal.as_ref(),
            &len.to_le_bytes(),
            app_file_id.as_bytes(),
        ])
    }
}

/// The account of the program's PDA actor for `seed`. `program` is the program's account id
/// (the header account it was deployed to).
#[must_use]
pub fn pda(program: AccountId, seed: &PdaSeed) -> AccountId {
    AccountId::for_public_pda(&program, seed)
}

/// Every address the program derives, for one deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Addresses {
    pub program: AccountId,
}

impl Addresses {
    #[must_use]
    pub const fn new(program: AccountId) -> Self {
        Self { program }
    }

    #[must_use]
    pub fn registry(&self) -> AccountId {
        pda(self.program, &seeds::registry())
    }

    #[must_use]
    pub fn portal(&self, owner: AccountId, i: u64) -> AccountId {
        pda(self.program, &seeds::portal(owner, i))
    }

    #[must_use]
    pub fn all_entry(&self, n: u64) -> AccountId {
        pda(self.program, &seeds::all(n))
    }

    #[must_use]
    pub fn owner_head(&self, owner: AccountId) -> AccountId {
        pda(self.program, &seeds::owner(owner))
    }

    #[must_use]
    pub fn owner_entry(&self, owner: AccountId, i: u64) -> AccountId {
        pda(self.program, &seeds::owned(owner, i))
    }

    #[must_use]
    pub fn file(&self, portal: AccountId, index: u64) -> AccountId {
        pda(self.program, &seeds::file(portal, index))
    }

    #[must_use]
    pub fn app_file(&self, portal: AccountId, app_file_id: &str) -> AccountId {
        pda(self.program, &seeds::app_file(portal, app_file_id))
    }
}
