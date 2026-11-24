//! A tiny in-process stand-in for the sequencer: it delivers a message to an actor, applies
//! the response's state change and then delivers its synchronous messages depth-first, with
//! the same authorization rule the runtime applies (a seed granted by this program's actor
//! authorizes exactly the PDA it derives).

use std::collections::HashMap;

use borsh::BorshSerialize;
use lee_core::{
    account::{AccountId, Actor, ActorState},
    program::{ProgramEvent, ReceiveInput, Response},
};
use portal_core::{Addresses, Message, PortalOp, PortalState};

use crate::handle_message;

pub(crate) const PROGRAM: AccountId = AccountId::new([0xAA; 32]);
pub(crate) const OTHER_PROGRAM: AccountId = AccountId::new([0xBB; 32]);

pub(crate) const ALICE: AccountId = AccountId::new([1; 32]);
pub(crate) const BOB: AccountId = AccountId::new([2; 32]);
pub(crate) const CAROL: AccountId = AccountId::new([3; 32]);

pub(crate) fn addresses() -> Addresses {
    Addresses::new(PROGRAM)
}

pub(crate) fn me(account_id: AccountId) -> Actor {
    Actor::new(account_id, PROGRAM)
}

#[derive(Default)]
pub(crate) struct Sim {
    pub states: HashMap<AccountId, Vec<u8>>,
    pub events: Vec<ProgramEvent>,
    /// Every actor that processed a message, in order.
    pub visited: Vec<AccountId>,
}

impl Sim {
    pub fn state(&self, account: AccountId) -> Option<PortalState> {
        PortalState::decode(self.states.get(&account).map_or(&[][..], Vec::as_slice))
    }

    /// A transaction: `message` to `root`, which is authorized when its account signed.
    pub fn send(&mut self, root: AccountId, signed: bool, message: &impl BorshSerialize) {
        self.deliver(
            me(root),
            None,
            signed,
            borsh::to_vec(message).expect("encodes"),
        );
    }

    /// `caller` signs and acts on `portal`.
    pub fn act(&mut self, caller: AccountId, portal: AccountId, op: PortalOp) {
        self.send(caller, true, &Message::Act { portal, op });
    }

    pub fn create_portal(&mut self, owner: AccountId) -> AccountId {
        let i = match self.state(addresses().owner_head(owner)) {
            Some(PortalState::OwnerHead(head)) => head.count,
            _ => 0,
        };
        self.send(
            owner,
            true,
            &Message::CreatePortal {
                owner_did: "did:owner".to_owned(),
                metadata_cid: "bafymeta".to_owned(),
            },
        );
        addresses().portal(owner, i)
    }

    fn deliver(&mut self, to: Actor, from: Option<Actor>, is_authorized: bool, message: Vec<u8>) {
        let pre = self.states.get(&to.account_id).cloned().unwrap_or_default();
        let input = ReceiveInput {
            receiver: to,
            from,
            is_authorized,
            pre_state: ActorState::from(pre),
            message,
        };
        let response: Response = handle_message(&input);
        self.visited.push(to.account_id);
        if let Some(post) = response.post_state {
            self.states.insert(to.account_id, post.into_inner());
        }
        self.events.extend(response.events);
        assert!(
            response.async_messages.is_empty(),
            "the portal program sends no async messages"
        );
        for sync in response.sync_messages {
            let granted = sync.pda_seeds.iter().any(|seed| {
                // Only an actor of the program that owns the PDA may grant its seed.
                to.program_account_id == sync.to.program_account_id
                    && AccountId::for_public_pda(&to.program_account_id, seed) == sync.to.account_id
            });
            self.deliver(sync.to, Some(to), granted, sync.message);
        }
    }
}
