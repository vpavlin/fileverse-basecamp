//! The registry head and the owner heads, with the entry actors they fill.
//!
//! Creating a portal is a conversation: the owner's own actor asks the owner head, which
//! picks the portal's address and an owner-list slot, the new portal registers itself with
//! the registry head, which hands it a slot in the all-portals list.

use lee_core::{
    account::{AccountId, Actor},
    program::{ReceiveInput, Response},
};
use portal_core::{
    seeds, Internal, OwnerHead, PortalRef, PortalState, RegistryHead, MAX_CID_LEN, MAX_DID_LEN,
};

use crate::{addresses, assert_receiver, assert_sender, send_to_pda, state};

/// At the caller's own actor: ask the caller's owner head for a new portal.
pub(crate) fn create_portal(
    input: &ReceiveInput,
    owner_did: String,
    metadata_cid: String,
) -> Response {
    assert!(input.is_authorized, "the owner's actor must authorize");
    assert!(
        owner_did.len() <= MAX_DID_LEN,
        "owner DID is longer than {MAX_DID_LEN} bytes"
    );
    assert!(
        metadata_cid.len() <= MAX_CID_LEN,
        "metadata CID is longer than {MAX_CID_LEN} bytes"
    );
    let owner = input.receiver.account_id;
    assert!(
        owner != AccountId::default(),
        "owner must not be the zero account id"
    );
    Response::keep_state().send(send_to_pda(
        input,
        addresses(input).owner_head(owner),
        seeds::owner(owner),
        &Internal::NewPortal {
            owner_did,
            metadata_cid,
        },
    ))
}

fn owner_head_state(input: &ReceiveInput, owner: AccountId) -> OwnerHead {
    match state(input) {
        None => OwnerHead { owner, count: 0 },
        Some(PortalState::OwnerHead(head)) => head,
        Some(_) => panic!("receiver is not an owner head"),
    }
}

/// At an owner head, from the owner's own actor.
pub(crate) fn new_portal(
    input: &ReceiveInput,
    from: Actor,
    owner_did: String,
    metadata_cid: String,
) -> Response {
    let owner = from.account_id;
    assert_receiver(input, addresses(input).owner_head(owner), "owner head");
    assert!(input.is_authorized, "owner head must be authorized");
    let head = owner_head_state(input, owner);
    let i = head.count;
    let a = addresses(input);
    let portal = a.portal(owner, i);
    Response::set_state(
        PortalState::OwnerHead(OwnerHead {
            owner,
            count: i.checked_add(1).expect("owner portal count overflow"),
        })
        .encode(),
    )
    .send(send_to_pda(
        input,
        portal,
        seeds::portal(owner, i),
        &Internal::InitPortal {
            owner,
            i,
            owner_did,
            metadata_cid,
        },
    ))
    .send(send_to_pda(
        input,
        a.owner_entry(owner, i),
        seeds::owned(owner, i),
        &Internal::SetOwnerEntry { owner, i, portal },
    ))
}

/// At an owner head, from a portal whose ownership `owner` just accepted.
pub(crate) fn owner_add(
    input: &ReceiveInput,
    from: Actor,
    owner: AccountId,
    portal: AccountId,
) -> Response {
    assert_sender(from, portal, "the portal being listed");
    assert_receiver(input, addresses(input).owner_head(owner), "owner head");
    assert!(input.is_authorized, "owner head must be authorized");
    let head = owner_head_state(input, owner);
    let i = head.count;
    Response::set_state(
        PortalState::OwnerHead(OwnerHead {
            owner,
            count: i.checked_add(1).expect("owner portal count overflow"),
        })
        .encode(),
    )
    .send(send_to_pda(
        input,
        addresses(input).owner_entry(owner, i),
        seeds::owned(owner, i),
        &Internal::SetOwnerEntry { owner, i, portal },
    ))
}

/// At an owner-entry actor, from its owner head.
pub(crate) fn set_owner_entry(
    input: &ReceiveInput,
    from: Actor,
    owner: AccountId,
    i: u64,
    portal: AccountId,
) -> Response {
    let a = addresses(input);
    assert_sender(from, a.owner_head(owner), "the owner's head");
    assert_receiver(input, a.owner_entry(owner, i), "owner entry");
    assert!(input.pre_state.is_empty(), "owner entry is already set");
    assert!(input.is_authorized, "owner entry must be authorized");
    Response::set_state(PortalState::OwnerEntry(PortalRef { portal }).encode())
}

/// At the registry head, from a new portal.
pub(crate) fn register_portal(input: &ReceiveInput, from: Actor) -> Response {
    let a = addresses(input);
    assert_receiver(input, a.registry(), "registry");
    assert!(input.is_authorized, "registry must be authorized");
    let head = match state(input) {
        None => RegistryHead { portal_count: 0 },
        Some(PortalState::RegistryHead(head)) => head,
        Some(_) => panic!("receiver is not the registry head"),
    };
    let n = head.portal_count;
    Response::set_state(
        PortalState::RegistryHead(RegistryHead {
            portal_count: n.checked_add(1).expect("portal count overflow"),
        })
        .encode(),
    )
    .send(send_to_pda(
        input,
        a.all_entry(n),
        seeds::all(n),
        &Internal::SetAllEntry {
            n,
            portal: from.account_id,
        },
    ))
}

/// At an all-entry actor, from the registry head.
pub(crate) fn set_all_entry(
    input: &ReceiveInput,
    from: Actor,
    n: u64,
    portal: AccountId,
) -> Response {
    let a = addresses(input);
    assert_sender(from, a.registry(), "the registry");
    assert_receiver(input, a.all_entry(n), "all entry");
    assert!(input.pre_state.is_empty(), "all entry is already set");
    assert!(input.is_authorized, "all entry must be authorized");
    Response::set_state(PortalState::AllEntry(PortalRef { portal }).encode())
}
