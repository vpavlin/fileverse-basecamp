//! A client for the Fileverse portal program on LEZ.
//!
//! This crate does no I/O. It builds what a transaction needs and decodes what an actor
//! holds; submitting a transaction and reading accounts goes through the LEZ wallet.
//!
//! A [`Plan`] is one transaction: the **root** actor receives the message and its account
//! must sign; the **actors** are every other actor the conversation reaches, which LEZ
//! needs in the transaction's access list (`spel send --actor …` takes the same list).
//! Everything except the root's own account is a PDA of the program, derived here.
//!
//! The plan functions take the counters they need (portal counts, a portal's file count)
//! from you: read them from chain first with [`PortalClient::decode`]. If another
//! transaction moved a counter in between, the simulation refuses and you re-read and retry.

use borsh::BorshSerialize;
use lee_core::account::AccountId;
pub use portal_core::{
    self as core, Addresses, AppFile, CollaboratorKeys, File, FileType, Internal, Message,
    OwnerHead, Portal, PortalOp, PortalRef, PortalState, RegistryHead,
};

/// One transaction: a message to `root` (which signs), with the other actors it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The actor that receives the message. Its account must sign.
    pub root: AccountId,
    pub message: Message,
    /// Every other actor of the portal program the transaction touches.
    pub actors: Vec<AccountId>,
}

impl Plan {
    /// The message as Borsh bytes: the variant tag, then the fields.
    #[must_use]
    pub fn message_bytes(&self) -> Vec<u8> {
        encode(&self.message)
    }
}

/// Borsh bytes of any message.
#[must_use]
pub fn encode(message: &impl BorshSerialize) -> Vec<u8> {
    borsh::to_vec(message).expect("borsh serialization is infallible")
}

/// A client for one deployment of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalClient {
    addresses: Addresses,
}

impl PortalClient {
    /// `program` is the account `spel deploy` printed (the program's header account).
    #[must_use]
    pub const fn new(program: AccountId) -> Self {
        Self {
            addresses: Addresses::new(program),
        }
    }

    #[must_use]
    pub const fn addresses(&self) -> Addresses {
        self.addresses
    }

    /// Decodes the bytes an actor holds: `None` for an empty actor.
    ///
    /// # Errors
    /// If the bytes are not a `PortalState` of this program.
    pub fn decode(bytes: &[u8]) -> Result<Option<PortalState>, borsh::io::Error> {
        PortalState::try_decode(bytes)
    }

    // --- Reading ----------------------------------------------------------------------

    /// The accounts of all files of a portal, tombstones included: file ids are
    /// `0..portal.file_count`.
    #[must_use]
    pub fn file_accounts(&self, portal: AccountId, file_count: u64) -> Vec<AccountId> {
        (0..file_count)
            .map(|index| self.addresses.file(portal, index))
            .collect()
    }

    /// The accounts of a window of the all-portals list: slots `from..to`
    /// (`to` at most the registry head's `portal_count`).
    #[must_use]
    pub fn all_entry_accounts(&self, from: u64, to: u64) -> Vec<AccountId> {
        (from..to).map(|n| self.addresses.all_entry(n)).collect()
    }

    /// The accounts of an owner's portal list: slots `0..owner_head.count`.
    #[must_use]
    pub fn owner_entry_accounts(&self, owner: AccountId, count: u64) -> Vec<AccountId> {
        (0..count)
            .map(|i| self.addresses.owner_entry(owner, i))
            .collect()
    }

    /// The account of the file an `app_file_id` points to, after reading the app-file actor.
    #[must_use]
    pub fn file_of(&self, app_file: &AppFile) -> AccountId {
        self.addresses.file(app_file.portal, app_file.index)
    }

    // --- Plans ------------------------------------------------------------------------

    /// Create a portal owned by `owner` (who signs).
    ///
    /// `owner_count` is the owner head's `count` (0 if its actor is empty) and
    /// `registry_count` the registry head's `portal_count` (0 if empty). The new portal
    /// lives at [`Self::new_portal_account`]`(owner, owner_count)`.
    #[must_use]
    pub fn create_portal(
        &self,
        owner: AccountId,
        owner_did: String,
        metadata_cid: String,
        owner_count: u64,
        registry_count: u64,
    ) -> Plan {
        let a = &self.addresses;
        Plan {
            root: owner,
            message: Message::CreatePortal {
                owner_did,
                metadata_cid,
            },
            actors: vec![
                a.owner_head(owner),
                a.portal(owner, owner_count),
                a.owner_entry(owner, owner_count),
                a.registry(),
                a.all_entry(registry_count),
            ],
        }
    }

    /// The account a portal created now by `owner` gets.
    #[must_use]
    pub fn new_portal_account(&self, owner: AccountId, owner_count: u64) -> AccountId {
        self.addresses.portal(owner, owner_count)
    }

    /// Any operation that touches only the portal itself: collaborators, keys, key
    /// verifiers, metadata, owner DID and `TransferOwnership`.
    ///
    /// For the operations that reach more actors use the dedicated plans below; this
    /// declares just the portal (and panics on an op that needs more).
    ///
    /// # Panics
    /// On `AddFile`, `EditFile`, `DeleteFile`, `DeleteFileByAppFileId` and `AcceptOwnership`.
    #[must_use]
    pub fn portal_op(&self, caller: AccountId, portal: AccountId, op: PortalOp) -> Plan {
        assert!(
            !matches!(
                op,
                PortalOp::AddFile { .. }
                    | PortalOp::EditFile { .. }
                    | PortalOp::DeleteFile { .. }
                    | PortalOp::DeleteFileByAppFileId { .. }
                    | PortalOp::AcceptOwnership
            ),
            "this operation reaches more actors: use its own plan"
        );
        act(caller, portal, op, vec![portal])
    }

    /// Add a file. `file_count` is the portal's current `file_count`.
    #[must_use]
    pub fn add_file(
        &self,
        caller: AccountId,
        portal: AccountId,
        file_count: u64,
        app_file_id: String,
        file_type: FileType,
        cids: Cids,
        version: u32,
    ) -> Plan {
        let a = &self.addresses;
        let actors = vec![
            portal,
            a.file(portal, file_count),
            a.app_file(portal, &app_file_id),
        ];
        act(
            caller,
            portal,
            PortalOp::AddFile {
                app_file_id,
                file_type,
                metadata_cid: cids.metadata,
                content_cid: cids.content,
                gate_cid: cids.gate,
                version,
            },
            actors,
        )
    }

    /// Replace file `index`'s type, CIDs and version.
    #[must_use]
    pub fn edit_file(
        &self,
        caller: AccountId,
        portal: AccountId,
        index: u64,
        file_type: FileType,
        cids: Cids,
        version: u32,
    ) -> Plan {
        act(
            caller,
            portal,
            PortalOp::EditFile {
                index,
                file_type,
                metadata_cid: cids.metadata,
                content_cid: cids.content,
                gate_cid: cids.gate,
                version,
            },
            vec![portal, self.addresses.file(portal, index)],
        )
    }

    /// Delete file `index` (a tombstone stays).
    #[must_use]
    pub fn delete_file(&self, caller: AccountId, portal: AccountId, index: u64) -> Plan {
        act(
            caller,
            portal,
            PortalOp::DeleteFile { index },
            vec![portal, self.addresses.file(portal, index)],
        )
    }

    /// Delete the file an `app_file_id` points to. `index` is the file index in the app-file
    /// actor (read it first: [`AppFile::index`]).
    #[must_use]
    pub fn delete_file_by_app_file_id(
        &self,
        caller: AccountId,
        portal: AccountId,
        app_file_id: String,
        index: u64,
    ) -> Plan {
        let actors = vec![
            portal,
            self.addresses.app_file(portal, &app_file_id),
            self.addresses.file(portal, index),
        ];
        act(
            caller,
            portal,
            PortalOp::DeleteFileByAppFileId { app_file_id },
            actors,
        )
    }

    /// The pending owner accepts. `owner_count` is `caller`'s owner head `count` (0 if empty).
    #[must_use]
    pub fn accept_ownership(&self, caller: AccountId, portal: AccountId, owner_count: u64) -> Plan {
        let a = &self.addresses;
        act(
            caller,
            portal,
            PortalOp::AcceptOwnership,
            vec![
                portal,
                a.owner_head(caller),
                a.owner_entry(caller, owner_count),
            ],
        )
    }
}

/// The three CIDs of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cids {
    pub metadata: String,
    pub content: String,
    pub gate: String,
}

fn act(caller: AccountId, portal: AccountId, op: PortalOp, actors: Vec<AccountId>) -> Plan {
    Plan {
        root: caller,
        message: Message::Act { portal, op },
        actors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: AccountId = AccountId::new([0xAA; 32]);
    const ALICE: AccountId = AccountId::new([1; 32]);
    const BOB: AccountId = AccountId::new([2; 32]);

    fn client() -> PortalClient {
        PortalClient::new(PROGRAM)
    }

    #[test]
    fn addresses_are_deterministic_and_distinct() {
        let c = client();
        let a = c.addresses();
        let ids = [
            a.registry(),
            a.all_entry(0),
            a.all_entry(1),
            a.owner_head(ALICE),
            a.owner_head(BOB),
            a.owner_entry(ALICE, 0),
            a.portal(ALICE, 0),
            a.portal(ALICE, 1),
            a.file(a.portal(ALICE, 0), 0),
            a.file(a.portal(ALICE, 0), 1),
            a.app_file(a.portal(ALICE, 0), "x"),
            a.app_file(a.portal(ALICE, 0), "y"),
        ];
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(ids.iter().filter(|other| *other == id).count(), 1, "{i}");
        }
        assert_eq!(
            PortalClient::new(PROGRAM).addresses().registry(),
            a.registry()
        );
        assert_ne!(PortalClient::new(BOB).addresses().registry(), a.registry());
    }

    #[test]
    fn create_portal_declares_every_actor_it_reaches() {
        let c = client();
        let plan = c.create_portal(ALICE, "did".into(), "cid".into(), 2, 5);
        let a = c.addresses();
        assert_eq!(plan.root, ALICE);
        assert_eq!(
            plan.actors,
            vec![
                a.owner_head(ALICE),
                a.portal(ALICE, 2),
                a.owner_entry(ALICE, 2),
                a.registry(),
                a.all_entry(5),
            ]
        );
        assert_eq!(c.new_portal_account(ALICE, 2), a.portal(ALICE, 2));
    }

    #[test]
    fn add_file_declares_the_new_file_and_app_file_actors() {
        let c = client();
        let a = c.addresses();
        let portal = a.portal(ALICE, 0);
        let plan = c.add_file(
            BOB,
            portal,
            3,
            "doc".into(),
            FileType::Public,
            Cids::default(),
            1,
        );
        assert_eq!(plan.root, BOB);
        assert_eq!(
            plan.actors,
            vec![portal, a.file(portal, 3), a.app_file(portal, "doc")]
        );
    }

    #[test]
    fn messages_round_trip_through_borsh() {
        let plan = client().delete_file(ALICE, AccountId::new([9; 32]), 4);
        let decoded: Message = borsh::from_slice(&plan.message_bytes()).expect("decodes");
        assert_eq!(decoded, plan.message);
    }

    #[test]
    fn state_decodes_and_an_empty_actor_is_none() {
        assert_eq!(PortalClient::decode(&[]).expect("ok"), None);
        let state = PortalState::RegistryHead(RegistryHead { portal_count: 7 });
        assert_eq!(
            PortalClient::decode(&state.encode()).expect("ok"),
            Some(state)
        );
        assert!(PortalClient::decode(&[0xFF, 1, 2]).is_err());
    }

    #[test]
    #[should_panic(expected = "use its own plan")]
    fn portal_op_refuses_operations_that_reach_more_actors() {
        let _ = client().portal_op(ALICE, ALICE, PortalOp::DeleteFile { index: 0 });
    }

    #[test]
    fn listing_helpers_cover_the_ranges() {
        let c = client();
        let portal = AccountId::new([7; 32]);
        assert_eq!(c.file_accounts(portal, 3).len(), 3);
        assert_eq!(c.all_entry_accounts(2, 5).len(), 3);
        assert_eq!(c.owner_entry_accounts(ALICE, 2).len(), 2);
        assert_eq!(c.file_accounts(portal, 0), Vec::<AccountId>::new());
    }
}
