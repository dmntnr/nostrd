pub mod nip01;
pub mod nip09;
pub mod nip11;
pub mod nip18;
pub mod nip19;
pub mod nip23;
pub mod nip25;
pub mod nip28;
pub mod nip40;
pub mod nip42;
pub mod nip45;
pub mod nip77;

use std::sync::Arc;

use crate::config::Config;
use crate::db::LmdbStore;
use crate::types::{ClientMessage, Event, RelayMessage};

pub struct NipContext {
    pub config: Arc<Config>,
    pub store: Arc<LmdbStore>,
}

pub enum Action {
    Send(RelayMessage),
    CloseSubscription(String, String),
    SetAuth(String),
    BroadcastEvent(Arc<Event>),
}

impl Action {
    pub fn send_event(sub_id: &str, event: Arc<Event>) -> Self {
        Action::Send(RelayMessage::event(sub_id, event))
    }

    pub fn ok(event_id: &str, success: bool, message: &str) -> Self {
        Action::Send(RelayMessage::ok(event_id, success, message))
    }

    pub fn notice(message: &str) -> Self {
        Action::Send(RelayMessage::notice(message))
    }

    pub fn eose(sub_id: &str) -> Self {
        Action::Send(RelayMessage::eose(sub_id))
    }

    pub fn closed(sub_id: &str, message: &str) -> Self {
        Action::Send(RelayMessage::closed(sub_id, message))
    }

    pub fn count(sub_id: &str, count: usize) -> Self {
        Action::Send(RelayMessage::count(sub_id, count))
    }
}

pub fn process_message(
    ctx: &NipContext,
    msg: &ClientMessage,
    auth_pubkey: Option<&[u8; 32]>,
    neg_sessions: &nip77::NegentropySessions,
) -> Vec<Action> {
    let mut actions = Vec::new();

    match msg {
        ClientMessage::Event { event } => {
            if let Some(exp_actions) = nip40::check_expiration(event) {
                return exp_actions;
            }

            if let Some(nip25_actions) = nip25::check_event(event) {
                return nip25_actions;
            }

            if let Some(nip18_actions) = nip18::check_event(event) {
                return nip18_actions;
            }

            if let Some(nip23_actions) = nip23::check_event(event) {
                return nip23_actions;
            }

            if let Some(nip28_actions) = nip28::check_event(ctx, event) {
                return nip28_actions;
            }

            actions.extend(nip01::handle_event(ctx, event));
            actions.extend(nip09::handle_event(ctx, event));
        }
        ClientMessage::Req {
            subscription_id,
            filters,
            ..
        } => {
            actions.extend(nip01::handle_req(
                ctx,
                subscription_id,
                filters,
                auth_pubkey,
            ));
        }
        ClientMessage::Close { subscription_id } => {
            actions.extend(nip01::handle_close(subscription_id));
        }
        ClientMessage::Auth { event } => {
            actions.extend(nip42::handle_auth(ctx, event));
        }
        ClientMessage::Count {
            subscription_id,
            filters,
            ..
        } => {
            actions.extend(nip45::handle_count(
                ctx,
                subscription_id,
                filters,
                auth_pubkey,
            ));
        }
        ClientMessage::NegOpen {
            subscription_id,
            filter,
            initial_message,
        } => {
            actions.extend(nip77::handle_neg_open(
                ctx,
                subscription_id,
                filter,
                initial_message,
                neg_sessions,
            ));
        }
        ClientMessage::NegMsg {
            subscription_id,
            message,
        } => {
            actions.extend(nip77::handle_neg_msg(
                ctx,
                subscription_id,
                message,
                neg_sessions,
            ));
        }
        ClientMessage::NegClose { subscription_id } => {
            actions.extend(nip77::handle_neg_close(subscription_id, neg_sessions));
        }
    }

    actions
}
