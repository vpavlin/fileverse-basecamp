//! Conversations through the in-process [`Sim`](crate::test_utils::Sim).

use lee_core::account::AccountId;
use portal_core::{
    event::{selector, PortalCreated, PortalEvent as _},
    File, FileType, PortalOp, PortalState,
};

use crate::test_utils::*;

fn add_file(app_file_id: &str, cid: &str) -> PortalOp {
    PortalOp::AddFile {
        app_file_id: app_file_id.to_owned(),
        file_type: FileType::Public,
        metadata_cid: format!("{cid}-meta"),
        content_cid: format!("{cid}-content"),
        gate_cid: String::new(),
        version: 1,
    }
}

fn portal(sim: &Sim, id: AccountId) -> portal_core::Portal {
    match sim.state(id) {
        Some(PortalState::Portal(portal)) => portal,
        other => panic!("not a portal: {other:?}"),
    }
}

fn file(sim: &Sim, portal: AccountId, index: u64) -> File {
    match sim.state(addresses().file(portal, index)) {
        Some(PortalState::File(file)) => file,
        other => panic!("not a file: {other:?}"),
    }
}

#[test]
fn creating_a_portal_lists_it_in_the_registry_and_under_its_owner() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    assert_eq!(p, addresses().portal(ALICE, 0));
    let state = portal(&sim, p);
    assert_eq!(state.owner, ALICE);
    assert_eq!(state.owner_did, "did:owner");
    assert_eq!(state.file_count, 0);

    assert_eq!(
        sim.state(addresses().registry()),
        Some(PortalState::RegistryHead(portal_core::RegistryHead {
            portal_count: 1
        }))
    );
    assert_eq!(
        sim.state(addresses().all_entry(0)),
        Some(PortalState::AllEntry(portal_core::PortalRef { portal: p }))
    );
    assert_eq!(
        sim.state(addresses().owner_head(ALICE)),
        Some(PortalState::OwnerHead(portal_core::OwnerHead {
            owner: ALICE,
            count: 1
        }))
    );
    assert_eq!(
        sim.state(addresses().owner_entry(ALICE, 0)),
        Some(PortalState::OwnerEntry(portal_core::PortalRef {
            portal: p
        }))
    );
    assert_eq!(
        sim.events,
        vec![PortalCreated {
            portal: p,
            owner: ALICE
        }
        .to_program_event()]
    );
}

#[test]
fn portals_get_consecutive_slots() {
    let mut sim = Sim::default();
    let a0 = sim.create_portal(ALICE);
    let b0 = sim.create_portal(BOB);
    let a1 = sim.create_portal(ALICE);
    assert_ne!(a0, a1);
    assert_eq!(
        sim.state(addresses().all_entry(2)),
        Some(PortalState::AllEntry(portal_core::PortalRef { portal: a1 }))
    );
    assert_eq!(
        sim.state(addresses().all_entry(1)),
        Some(PortalState::AllEntry(portal_core::PortalRef { portal: b0 }))
    );
    assert_eq!(
        sim.state(addresses().owner_entry(ALICE, 1)),
        Some(PortalState::OwnerEntry(portal_core::PortalRef {
            portal: a1
        }))
    );
    assert_eq!(
        sim.state(addresses().owner_entry(BOB, 0)),
        Some(PortalState::OwnerEntry(portal_core::PortalRef {
            portal: b0
        }))
    );
}

#[test]
#[should_panic(expected = "the owner's actor must authorize")]
fn creating_a_portal_needs_the_owners_signature() {
    let mut sim = Sim::default();
    sim.send(
        ALICE,
        false,
        &portal_core::Message::CreatePortal {
            owner_did: String::new(),
            metadata_cid: String::new(),
        },
    );
}

#[test]
fn collaborators_are_managed_by_the_owner_only() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    assert_eq!(portal(&sim, p).collaborators, vec![BOB]);

    sim.act(
        BOB,
        p,
        PortalOp::RegisterKeys {
            view_did: "view".to_owned(),
            edit_did: "edit".to_owned(),
        },
    );
    assert_eq!(portal(&sim, p).collaborator_keys.len(), 1);

    sim.act(ALICE, p, PortalOp::RemoveCollaborator { who: BOB });
    let state = portal(&sim, p);
    assert!(state.collaborators.is_empty());
    assert!(
        state.collaborator_keys.is_empty(),
        "keys go with the collaborator"
    );
}

#[test]
#[should_panic(expected = "only the owner may add a collaborator")]
fn a_collaborator_cannot_add_collaborators() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    sim.act(BOB, p, PortalOp::AddCollaborator { who: CAROL });
}

#[test]
#[should_panic(expected = "already a collaborator")]
fn a_collaborator_is_added_once() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
}

#[test]
#[should_panic(expected = "only the owner or a collaborator may register keys")]
fn strangers_cannot_register_keys() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(
        CAROL,
        p,
        PortalOp::RegisterKeys {
            view_did: String::new(),
            edit_did: String::new(),
        },
    );
}

#[test]
fn keys_are_replaced_and_removed() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    for edit in ["e1", "e2"] {
        sim.act(
            ALICE,
            p,
            PortalOp::RegisterKeys {
                view_did: "v".to_owned(),
                edit_did: edit.to_owned(),
            },
        );
    }
    let keys = portal(&sim, p).collaborator_keys;
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].edit_did, "e2");
    sim.act(ALICE, p, PortalOp::RemoveKeys { who: ALICE });
    assert!(portal(&sim, p).collaborator_keys.is_empty());
}

#[test]
fn owner_updates_verifiers_metadata_and_did() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(
        ALICE,
        p,
        PortalOp::UpdateKeyVerifiers {
            verifiers: vec![[7; 32], [8; 32]],
        },
    );
    sim.act(
        ALICE,
        p,
        PortalOp::UpdateMetadata {
            metadata_cid: "bafynew".to_owned(),
        },
    );
    sim.act(
        ALICE,
        p,
        PortalOp::SetOwnerDid {
            owner_did: "did:new".to_owned(),
        },
    );
    let state = portal(&sim, p);
    assert_eq!(state.key_verifiers, vec![[7; 32], [8; 32]]);
    assert_eq!(state.metadata_cid, "bafynew");
    assert_eq!(state.owner_did, "did:new");
}

#[test]
fn files_get_sequential_ids_and_an_app_file_lookup() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    sim.act(ALICE, p, add_file("doc-a", "a"));
    sim.act(BOB, p, add_file("doc-b", "b"));

    assert_eq!(portal(&sim, p).file_count, 2);
    let f1 = file(&sim, p, 1);
    assert_eq!(f1.app_file_id, "doc-b");
    assert_eq!(f1.content_cid, "b-content");
    assert_eq!(f1.last_editor, BOB);
    assert_eq!(
        sim.state(addresses().app_file(p, "doc-a")),
        Some(PortalState::AppFile(portal_core::AppFile {
            portal: p,
            index: 0
        }))
    );
}

#[test]
#[should_panic(expected = "app file id is already in use")]
fn app_file_ids_are_unique() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, add_file("doc", "a"));
    sim.act(ALICE, p, add_file("doc", "b"));
}

#[test]
#[should_panic(expected = "only the owner or a collaborator may add files")]
fn strangers_cannot_add_files() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(CAROL, p, add_file("doc", "a"));
}

#[test]
fn a_removed_collaborator_can_no_longer_add_files() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    sim.act(BOB, p, add_file("doc-b", "b"));
    sim.act(ALICE, p, PortalOp::RemoveCollaborator { who: BOB });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sim.act(BOB, p, add_file("doc-c", "c"));
    }));
    assert!(result.is_err());
}

#[test]
fn editing_replaces_the_cids_and_the_editor() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::AddCollaborator { who: BOB });
    sim.act(ALICE, p, add_file("doc", "a"));
    sim.act(
        BOB,
        p,
        PortalOp::EditFile {
            index: 0,
            file_type: FileType::Gated,
            metadata_cid: "m2".to_owned(),
            content_cid: "c2".to_owned(),
            gate_cid: "g2".to_owned(),
            version: 2,
        },
    );
    let f = file(&sim, p, 0);
    assert_eq!(
        (
            f.file_type,
            f.content_cid.as_str(),
            f.gate_cid.as_str(),
            f.version
        ),
        (FileType::Gated, "c2", "g2", 2)
    );
    assert_eq!(f.last_editor, BOB);
    assert_eq!(f.app_file_id, "doc");
}

#[test]
#[should_panic(expected = "no such file")]
fn editing_a_missing_file_fails() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(
        ALICE,
        p,
        PortalOp::EditFile {
            index: 0,
            file_type: FileType::Public,
            metadata_cid: String::new(),
            content_cid: String::new(),
            gate_cid: String::new(),
            version: 1,
        },
    );
}

#[test]
fn deleting_leaves_a_tombstone_so_enumeration_still_works() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, add_file("doc-a", "a"));
    sim.act(ALICE, p, add_file("doc-b", "b"));
    sim.act(ALICE, p, PortalOp::DeleteFile { index: 0 });

    assert_eq!(portal(&sim, p).file_count, 2);
    let f0 = file(&sim, p, 0);
    assert!(f0.deleted);
    assert!(f0.content_cid.is_empty() && f0.metadata_cid.is_empty());
    assert!(!file(&sim, p, 1).deleted);
}

#[test]
fn deleting_by_app_file_id_finds_the_file() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, add_file("doc-a", "a"));
    sim.act(ALICE, p, add_file("doc-b", "b"));
    sim.act(
        ALICE,
        p,
        PortalOp::DeleteFileByAppFileId {
            app_file_id: "doc-b".to_owned(),
        },
    );
    assert!(file(&sim, p, 1).deleted);
    assert!(!file(&sim, p, 0).deleted);
}

#[test]
#[should_panic(expected = "file is deleted")]
fn a_deleted_file_cannot_be_edited() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, add_file("doc", "a"));
    sim.act(ALICE, p, PortalOp::DeleteFile { index: 0 });
    sim.act(
        ALICE,
        p,
        PortalOp::EditFile {
            index: 0,
            file_type: FileType::Public,
            metadata_cid: String::new(),
            content_cid: String::new(),
            gate_cid: String::new(),
            version: 1,
        },
    );
}

#[test]
fn ownership_moves_in_two_steps_and_the_new_owner_is_listed() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::TransferOwnership { new_owner: BOB });
    assert_eq!(portal(&sim, p).owner, ALICE, "not yet");
    sim.act(BOB, p, PortalOp::AcceptOwnership);

    let state = portal(&sim, p);
    assert_eq!((state.owner, state.pending_owner), (BOB, None));
    assert_eq!(
        sim.state(addresses().owner_entry(BOB, 0)),
        Some(PortalState::OwnerEntry(portal_core::PortalRef {
            portal: p
        }))
    );
    // The previous owner has lost the owner rights.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sim.act(ALICE, p, PortalOp::AddCollaborator { who: CAROL });
    }));
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "only the pending owner may accept ownership")]
fn only_the_named_account_accepts_ownership() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, PortalOp::TransferOwnership { new_owner: BOB });
    sim.act(CAROL, p, PortalOp::AcceptOwnership);
}

#[test]
#[should_panic(expected = "the caller's actor must authorize")]
fn an_unsigned_caller_is_refused() {
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.send(
        ALICE,
        false,
        &portal_core::Message::Act {
            portal: p,
            op: PortalOp::AddCollaborator { who: BOB },
        },
    );
}

#[test]
fn events_use_the_portal_selector_convention() {
    use portal_core::event::AddedFile;
    let mut sim = Sim::default();
    let p = sim.create_portal(ALICE);
    sim.act(ALICE, p, add_file("doc", "a"));
    let added = sim.events.last().expect("an event");
    assert_eq!(added.selector, selector("AddedFile"));
    assert_eq!(
        *added,
        AddedFile {
            portal: p,
            index: 0,
            app_file_id: "doc".to_owned(),
            editor: ALICE,
        }
        .to_program_event()
    );
}

/// An actor of another program can't impersonate a portal: `Internal` messages are only
/// decoded for senders of this program, so the same bytes are a (failing) `Message` here.
#[test]
#[should_panic(expected = "portal message must decode")]
fn another_program_cannot_send_internal_messages() {
    use crate::handle_message;
    use lee_core::{
        account::{Actor, ActorState},
        program::ReceiveInput,
    };
    let internal = portal_core::Internal::TombstoneFile { editor: BOB };
    let input = ReceiveInput {
        receiver: me(addresses().file(ALICE, 0)),
        from: Some(Actor::new(ALICE, OTHER_PROGRAM)),
        is_authorized: true,
        pre_state: ActorState::empty(),
        message: borsh::to_vec(&internal).expect("encodes"),
    };
    let _ = handle_message(&input);
}
