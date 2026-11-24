//! The portal actor: creation, collaborators, keys, ownership and the file operations,
//! which it checks and then hands to file actors.

use lee_core::{
    account::{AccountId, Actor},
    program::{ReceiveInput, Response},
};
use portal_core::{
    event::{
        AddedCollaborator, AddedFile, DeletedFile, EditedFile, OwnershipTransferStarted,
        OwnershipTransferred, PortalCreated, PortalEvent as _, RegisteredCollaboratorKeys,
        RemovedCollaborator, RemovedCollaboratorKeys, UpdatedKeyVerifiers, UpdatedOwnerDid,
        UpdatedPortalMetadata,
    },
    seeds, CollaboratorKeys, Internal, Portal, PortalOp, PortalState, MAX_APP_FILE_ID_LEN,
    MAX_CID_LEN, MAX_COLLABORATORS, MAX_DID_LEN, MAX_KEY_VERIFIERS,
};

use crate::{addresses, assert_receiver, assert_sender, file_seed, send_to_pda, state};

fn portal_state(input: &ReceiveInput) -> Portal {
    match state(input) {
        Some(PortalState::Portal(portal)) => portal,
        _ => panic!("receiver is not a portal"),
    }
}

fn check_len(value: &str, max: usize, what: &str) {
    assert!(value.len() <= max, "{what} is longer than {max} bytes");
}

fn check_did(value: &str, what: &str) {
    check_len(value, MAX_DID_LEN, what);
}

fn check_cids(metadata_cid: &str, content_cid: &str, gate_cid: &str) {
    check_len(metadata_cid, MAX_CID_LEN, "metadata CID");
    check_len(content_cid, MAX_CID_LEN, "content CID");
    check_len(gate_cid, MAX_CID_LEN, "gate CID");
}

/// At the new portal, from its owner's head.
pub(crate) fn init(
    input: &ReceiveInput,
    from: Actor,
    owner: AccountId,
    i: u64,
    owner_did: String,
    metadata_cid: String,
) -> Response {
    let addresses = addresses(input);
    assert_sender(from, addresses.owner_head(owner), "the owner's head");
    assert_receiver(input, addresses.portal(owner, i), "portal");
    assert!(input.pre_state.is_empty(), "portal account must be empty");
    assert!(input.is_authorized, "portal creation must be authorized");
    check_did(&owner_did, "owner DID");
    check_len(&metadata_cid, MAX_CID_LEN, "metadata CID");

    let portal = input.receiver.account_id;
    let state = PortalState::Portal(Portal {
        owner,
        pending_owner: None,
        owner_did,
        metadata_cid,
        collaborators: Vec::new(),
        collaborator_keys: Vec::new(),
        key_verifiers: Vec::new(),
        file_count: 0,
    });
    Response::set_state(state.encode())
        .send(send_to_pda(
            input,
            addresses.registry(),
            seeds::registry(),
            &Internal::RegisterPortal,
        ))
        .event(PortalCreated { portal, owner }.to_program_event())
}

/// At a portal, from the caller's actor: `from.account_id` is the account that signed.
pub(crate) fn op(input: &ReceiveInput, from: Actor, op: PortalOp) -> Response {
    let mut portal = portal_state(input);
    let caller = from.account_id;
    let id = input.receiver.account_id;
    let is_owner = caller == portal.owner;
    let is_member = is_owner || portal.collaborators.contains(&caller);

    match op {
        PortalOp::AddCollaborator { who } => {
            assert!(is_owner, "only the owner may add a collaborator");
            assert!(
                who != AccountId::default(),
                "collaborator must not be the zero account id"
            );
            assert!(who != portal.owner, "the owner is already a member");
            assert!(
                !portal.collaborators.contains(&who),
                "already a collaborator"
            );
            assert!(
                portal.collaborators.len() < MAX_COLLABORATORS,
                "the portal has {MAX_COLLABORATORS} collaborators already"
            );
            portal.collaborators.push(who);
            write(&portal).event(AddedCollaborator { portal: id, who }.to_program_event())
        }
        PortalOp::RemoveCollaborator { who } => {
            assert!(is_owner, "only the owner may remove a collaborator");
            let before = portal.collaborators.len();
            portal.collaborators.retain(|c| *c != who);
            assert!(portal.collaborators.len() < before, "not a collaborator");
            portal.collaborator_keys.retain(|k| k.account != who);
            write(&portal).event(RemovedCollaborator { portal: id, who }.to_program_event())
        }
        PortalOp::RegisterKeys { view_did, edit_did } => {
            assert!(
                is_member,
                "only the owner or a collaborator may register keys"
            );
            check_did(&view_did, "view DID");
            check_did(&edit_did, "edit DID");
            portal.collaborator_keys.retain(|k| k.account != caller);
            portal.collaborator_keys.push(CollaboratorKeys {
                account: caller,
                view_did,
                edit_did,
            });
            write(&portal).event(
                RegisteredCollaboratorKeys {
                    portal: id,
                    who: caller,
                }
                .to_program_event(),
            )
        }
        PortalOp::RemoveKeys { who } => {
            assert!(
                is_owner || who == caller,
                "only the owner may remove someone else's keys"
            );
            let before = portal.collaborator_keys.len();
            portal.collaborator_keys.retain(|k| k.account != who);
            assert!(
                portal.collaborator_keys.len() < before,
                "no keys registered"
            );
            write(&portal).event(RemovedCollaboratorKeys { portal: id, who }.to_program_event())
        }
        PortalOp::UpdateKeyVerifiers { verifiers } => {
            assert!(is_owner, "only the owner may update the key verifiers");
            assert!(
                verifiers.len() <= MAX_KEY_VERIFIERS,
                "more than {MAX_KEY_VERIFIERS} key verifiers"
            );
            portal.key_verifiers = verifiers;
            write(&portal).event(UpdatedKeyVerifiers { portal: id }.to_program_event())
        }
        PortalOp::UpdateMetadata { metadata_cid } => {
            assert!(is_owner, "only the owner may update the metadata");
            check_len(&metadata_cid, MAX_CID_LEN, "metadata CID");
            portal.metadata_cid = metadata_cid.clone();
            write(&portal).event(
                UpdatedPortalMetadata {
                    portal: id,
                    metadata_cid,
                }
                .to_program_event(),
            )
        }
        PortalOp::SetOwnerDid { owner_did } => {
            assert!(is_owner, "only the owner may set the owner DID");
            check_did(&owner_did, "owner DID");
            portal.owner_did = owner_did;
            write(&portal).event(UpdatedOwnerDid { portal: id }.to_program_event())
        }
        PortalOp::TransferOwnership { new_owner } => {
            assert!(is_owner, "only the owner may transfer ownership");
            assert!(
                new_owner != AccountId::default(),
                "new owner must not be the zero account id"
            );
            portal.pending_owner = Some(new_owner);
            write(&portal).event(
                OwnershipTransferStarted {
                    portal: id,
                    new_owner,
                }
                .to_program_event(),
            )
        }
        PortalOp::AcceptOwnership => {
            assert!(
                portal.pending_owner == Some(caller),
                "only the pending owner may accept ownership"
            );
            let old_owner = portal.owner;
            portal.owner = caller;
            portal.pending_owner = None;
            // The new owner is no longer a plain collaborator, and the old owner's keys stay.
            portal.collaborators.retain(|c| *c != caller);
            let addresses = addresses(input);
            write(&portal)
                .send(send_to_pda(
                    input,
                    addresses.owner_head(caller),
                    seeds::owner(caller),
                    &Internal::OwnerAdd {
                        owner: caller,
                        portal: id,
                    },
                ))
                .event(
                    OwnershipTransferred {
                        portal: id,
                        old_owner,
                        new_owner: caller,
                    }
                    .to_program_event(),
                )
        }
        PortalOp::AddFile {
            app_file_id,
            file_type,
            metadata_cid,
            content_cid,
            gate_cid,
            version,
        } => {
            assert!(is_member, "only the owner or a collaborator may add files");
            assert!(!app_file_id.is_empty(), "app file id must not be empty");
            check_len(&app_file_id, MAX_APP_FILE_ID_LEN, "app file id");
            check_cids(&metadata_cid, &content_cid, &gate_cid);
            let index = portal.file_count;
            portal.file_count = index.checked_add(1).expect("file count overflow");
            let addresses = addresses(input);
            write(&portal)
                .send(send_to_pda(
                    input,
                    addresses.file(id, index),
                    file_seed(id, index),
                    &Internal::WriteFile {
                        index,
                        app_file_id: app_file_id.clone(),
                        file_type,
                        metadata_cid,
                        content_cid,
                        gate_cid,
                        version,
                        editor: caller,
                    },
                ))
                .send(send_to_pda(
                    input,
                    addresses.app_file(id, &app_file_id),
                    seeds::app_file(id, &app_file_id),
                    &Internal::BindAppFile {
                        app_file_id: app_file_id.clone(),
                        index,
                    },
                ))
                .event(
                    AddedFile {
                        portal: id,
                        index,
                        app_file_id,
                        editor: caller,
                    }
                    .to_program_event(),
                )
        }
        PortalOp::EditFile {
            index,
            file_type,
            metadata_cid,
            content_cid,
            gate_cid,
            version,
        } => {
            assert!(is_member, "only the owner or a collaborator may edit files");
            assert!(index < portal.file_count, "no such file");
            check_cids(&metadata_cid, &content_cid, &gate_cid);
            Response::keep_state()
                .send(send_to_pda(
                    input,
                    addresses(input).file(id, index),
                    file_seed(id, index),
                    &Internal::UpdateFile {
                        file_type,
                        metadata_cid,
                        content_cid,
                        gate_cid,
                        version,
                        editor: caller,
                    },
                ))
                .event(
                    EditedFile {
                        portal: id,
                        index,
                        editor: caller,
                    }
                    .to_program_event(),
                )
        }
        PortalOp::DeleteFile { index } => {
            assert!(
                is_member,
                "only the owner or a collaborator may delete files"
            );
            assert!(index < portal.file_count, "no such file");
            Response::keep_state()
                .send(send_to_pda(
                    input,
                    addresses(input).file(id, index),
                    file_seed(id, index),
                    &Internal::TombstoneFile { editor: caller },
                ))
                .event(
                    DeletedFile {
                        portal: id,
                        index,
                        editor: caller,
                    }
                    .to_program_event(),
                )
        }
        PortalOp::DeleteFileByAppFileId { app_file_id } => {
            assert!(
                is_member,
                "only the owner or a collaborator may delete files"
            );
            check_len(&app_file_id, MAX_APP_FILE_ID_LEN, "app file id");
            // The app-file actor knows the index; it forwards the tombstone. The event is
            // emitted there, once the file is known to exist.
            Response::keep_state().send(send_to_pda(
                input,
                addresses(input).app_file(id, &app_file_id),
                seeds::app_file(id, &app_file_id),
                &Internal::DeleteByAppFileId {
                    app_file_id,
                    editor: caller,
                },
            ))
        }
    }
}

fn write(portal: &Portal) -> Response {
    Response::set_state(PortalState::Portal(portal.clone()).encode())
}
