use uuid::Uuid;

use crate::nips::{Action, NipContext};
use crate::types::Event;
use crate::utils::crypto;

pub fn handle_auth(_ctx: &NipContext, event: &Event) -> Vec<Action> {
    if event.kind != 22242 {
        return vec![Action::ok(
            &event.id,
            false,
            "invalid: AUTH requires kind 22242 event",
        )];
    }

    let now = chrono::Utc::now().timestamp() as u64;
    if event.created_at > now + 600 || now > event.created_at + 600 {
        return vec![Action::ok(
            &event.id,
            false,
            "invalid: AUTH event created_at is too far from the current time",
        )];
    }

    if !crypto::verify_event_signature(event).unwrap_or(false) {
        return vec![Action::ok(
            &event.id,
            false,
            "invalid: AUTH event signature invalid",
        )];
    }

    let pubkey = event.pubkey.clone();

    vec![
        Action::ok(&event.id, true, ""),
        Action::SetAuth(pubkey),
    ]
}

pub fn generate_challenge() -> String {
    Uuid::new_v4().to_string()
}
