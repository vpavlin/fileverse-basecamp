//! The Fileverse portal program on LEZ's actor interface.
//!
//! A handler sees one actor: a portal, a file, a registry or listing actor, or an owner's
//! own actor. Operations that touch several are short synchronous conversations; each
//! module holds all the steps of the operations it owns:
//!
//! - [`portal`]: the portal actor (create, collaborators, keys, ownership, file operations);
//! - [`file`]: file and app-file actors;
//! - [`registry`]: the registry head, owner heads and their entry actors.
//!
//! [`Internal`] messages are decoded only when the sender is this program, so each is
//! trusted because only this program's code can have produced it. Every PDA actor also
//! checks that its own account is the one its message derives, so an actor can't be filled
//! at the wrong address.

use lee_core::{
    account::{AccountId, Actor},
    program::{ReceiveInput, Response, SyncMessage},
};
use portal_core::{seeds, Addresses, Internal, Message, PortalOp, PortalState};

mod file;
mod portal;
mod registry;
#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests;

/// The guest entry point: [`Internal`] messages from this program's actors, [`Message`]s
/// otherwise.
pub fn handle_message(input: &ReceiveInput) -> Response {
    if input.from_own_program() {
        let from = input.from.expect("an internal message always has a sender");
        let message: Internal =
            borsh::from_slice(&input.message).expect("internal portal message must decode");
        match message {
            Internal::Op { op } => portal::op(input, from, op),
            Internal::WriteFile {
                index,
                app_file_id,
                file_type,
                metadata_cid,
                content_cid,
                gate_cid,
                version,
                editor,
            } => file::write(
                input,
                from,
                file::NewFile {
                    index,
                    app_file_id,
                    file_type,
                    metadata_cid,
                    content_cid,
                    gate_cid,
                    version,
                    editor,
                },
            ),
            Internal::UpdateFile {
                file_type,
                metadata_cid,
                content_cid,
                gate_cid,
                version,
                editor,
            } => file::update(
                input,
                from,
                file::FileEdit {
                    file_type,
                    metadata_cid,
                    content_cid,
                    gate_cid,
                    version,
                    editor,
                },
            ),
            Internal::TombstoneFile { editor } => file::tombstone(input, from, editor),
            Internal::BindAppFile { app_file_id, index } => {
                file::bind_app_file(input, from, &app_file_id, index)
            }
            Internal::DeleteByAppFileId {
                app_file_id,
                editor,
            } => file::delete_by_app_file_id(input, from, &app_file_id, editor),
            Internal::NewPortal {
                owner_did,
                metadata_cid,
            } => registry::new_portal(input, from, owner_did, metadata_cid),
            Internal::InitPortal {
                owner,
                i,
                owner_did,
                metadata_cid,
            } => portal::init(input, from, owner, i, owner_did, metadata_cid),
            Internal::RegisterPortal => registry::register_portal(input, from),
            Internal::SetAllEntry { n, portal } => registry::set_all_entry(input, from, n, portal),
            Internal::OwnerAdd { owner, portal } => registry::owner_add(input, from, owner, portal),
            Internal::SetOwnerEntry { owner, i, portal } => {
                registry::set_owner_entry(input, from, owner, i, portal)
            }
        }
    } else {
        let message: Message =
            borsh::from_slice(&input.message).expect("portal message must decode");
        match message {
            Message::CreatePortal {
                owner_did,
                metadata_cid,
            } => registry::create_portal(input, owner_did, metadata_cid),
            Message::Act { portal, op } => act(input, portal, op),
        }
    }
}

/// At the caller's own actor: the account signed, so the portal may treat it as the caller.
/// The caller's actor state is left untouched.
fn act(input: &ReceiveInput, portal: AccountId, op: PortalOp) -> Response {
    assert!(input.is_authorized, "the caller's actor must authorize");
    Response::keep_state().send_sync(actor(input, portal), &Internal::Op { op })
}

/// This program's actor on `account_id`.
fn actor(input: &ReceiveInput, account_id: AccountId) -> Actor {
    Actor::new(account_id, input.receiver.program_account_id)
}

fn addresses(input: &ReceiveInput) -> Addresses {
    Addresses::new(input.receiver.program_account_id)
}

fn state(input: &ReceiveInput) -> Option<PortalState> {
    PortalState::decode(&input.pre_state)
}

/// A synchronous message to a PDA actor of this program, with the seed that authorizes it.
fn send_to_pda(
    input: &ReceiveInput,
    to: AccountId,
    seed: lee_core::program::PdaSeed,
    message: &Internal,
) -> SyncMessage {
    SyncMessage::new(actor(input, to), message).with_pda_seeds([seed])
}

/// Panics unless this actor's account is `expected`.
fn assert_receiver(input: &ReceiveInput, expected: AccountId, what: &str) {
    assert!(
        input.receiver.account_id == expected,
        "{what} actor is not at its derived address"
    );
}

/// Panics unless the sender is at `expected`.
fn assert_sender(from: Actor, expected: AccountId, what: &str) {
    assert!(from.account_id == expected, "message must come from {what}");
}

/// The seed to grant for file `index` of `portal`, re-exported for the handlers.
fn file_seed(portal: AccountId, index: u64) -> lee_core::program::PdaSeed {
    seeds::file(portal, index)
}
