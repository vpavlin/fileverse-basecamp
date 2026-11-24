//! Events, emitted by the portal actor (the file, registry and listing actors only store
//! data). The indexer records the emitting program, not the account, so every event names
//! the portal.

use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::{account::AccountId, program::ProgramEvent};
use risc0_zkvm::sha::{Impl, Sha256 as _};

pub trait PortalEvent: BorshSerialize {
    const NAME: &'static str;

    #[must_use]
    fn to_program_event(&self) -> ProgramEvent {
        ProgramEvent {
            selector: selector(Self::NAME),
            data: borsh::to_vec(self).expect("borsh serialization is infallible"),
        }
    }
}

/// `sha256("portal::<name>")[..8]`, the LEZ selector convention.
#[must_use]
pub fn selector(name: &str) -> [u8; 8] {
    let digest = Impl::hash_bytes(format!("portal::{name}").as_bytes());
    *digest
        .as_bytes()
        .first_chunk::<8>()
        .expect("a sha256 digest has 32 bytes")
}

/// Emitted when a portal is created.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct PortalCreated {
    pub portal: AccountId,
    pub owner: AccountId,
}

impl PortalEvent for PortalCreated {
    const NAME: &'static str = "PortalCreated";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AddedCollaborator {
    pub portal: AccountId,
    pub who: AccountId,
}

impl PortalEvent for AddedCollaborator {
    const NAME: &'static str = "AddedCollaborator";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct RemovedCollaborator {
    pub portal: AccountId,
    pub who: AccountId,
}

impl PortalEvent for RemovedCollaborator {
    const NAME: &'static str = "RemovedCollaborator";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct RegisteredCollaboratorKeys {
    pub portal: AccountId,
    pub who: AccountId,
}

impl PortalEvent for RegisteredCollaboratorKeys {
    const NAME: &'static str = "RegisteredCollaboratorKeys";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct RemovedCollaboratorKeys {
    pub portal: AccountId,
    pub who: AccountId,
}

impl PortalEvent for RemovedCollaboratorKeys {
    const NAME: &'static str = "RemovedCollaboratorKeys";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct UpdatedKeyVerifiers {
    pub portal: AccountId,
}

impl PortalEvent for UpdatedKeyVerifiers {
    const NAME: &'static str = "UpdatedKeyVerifiers";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct UpdatedPortalMetadata {
    pub portal: AccountId,
    pub metadata_cid: String,
}

impl PortalEvent for UpdatedPortalMetadata {
    const NAME: &'static str = "UpdatedPortalMetadata";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct UpdatedOwnerDid {
    pub portal: AccountId,
}

impl PortalEvent for UpdatedOwnerDid {
    const NAME: &'static str = "UpdatedOwnerDid";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct OwnershipTransferStarted {
    pub portal: AccountId,
    pub new_owner: AccountId,
}

impl PortalEvent for OwnershipTransferStarted {
    const NAME: &'static str = "OwnershipTransferStarted";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct OwnershipTransferred {
    pub portal: AccountId,
    pub old_owner: AccountId,
    pub new_owner: AccountId,
}

impl PortalEvent for OwnershipTransferred {
    const NAME: &'static str = "OwnershipTransferred";
}

/// Emitted when a file is added: its id is `index`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct AddedFile {
    pub portal: AccountId,
    pub index: u64,
    pub app_file_id: String,
    pub editor: AccountId,
}

impl PortalEvent for AddedFile {
    const NAME: &'static str = "AddedFile";
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EditedFile {
    pub portal: AccountId,
    pub index: u64,
    pub editor: AccountId,
}

impl PortalEvent for EditedFile {
    const NAME: &'static str = "EditedFile";
}

/// Emitted when a file is deleted (a tombstone stays at `index`).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct DeletedFile {
    pub portal: AccountId,
    pub index: u64,
    pub editor: AccountId,
}

impl PortalEvent for DeletedFile {
    const NAME: &'static str = "DeletedFile";
}
