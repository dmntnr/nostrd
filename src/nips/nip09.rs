use crate::nips::{Action, NipContext};
use crate::types::Event;

pub fn handle_event(ctx: &NipContext, event: &Event) -> Vec<Action> {
    if event.kind != 5 {
        return vec![];
    }

    let mut actions = Vec::new();
    let author = match event.pubkey_bytes() {
        Some(a) => a,
        None => return vec![],
    };

    let e_refs = event.event_refs();
    let a_refs: Vec<&str> = event.get_tag_values("a");
    if e_refs.is_empty() && a_refs.is_empty() {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: deletion event requires at least one 'e' or 'a' tag",
        ));
        return actions;
    }

    for e_ref in e_refs {
        if let Ok(event_id) = hex::decode(e_ref) {
            if let Ok(event_id) = event_id.try_into() {
                match ctx.store.delete_event(&event_id, &author) {
                    Ok(()) => {
                        tracing::info!("Event {} deleted by {} via kind 5", e_ref, event.pubkey);
                    }
                    Err(_) => {
                        actions.push(Action::notice("error: deletion failed"));
                    }
                }
            }
        }
    }

    for a_ref in a_refs {
        delete_by_a_tag(ctx, event, a_ref, &author);
    }

    actions
}

fn delete_by_a_tag(
    ctx: &NipContext,
    event: &Event,
    a_ref: &str,
    author: &[u8; 32],
) {
    let first_colon = match a_ref.find(':') {
        Some(p) => p,
        None => return,
    };
    let kind: u64 = match a_ref[..first_colon].parse() {
        Ok(k) => k,
        Err(_) => return,
    };

    let after_kind = &a_ref[first_colon + 1..];
    if after_kind.len() < 64 {
        return;
    }
    let pubkey_hex = &after_kind[..64];
    let d_tag = after_kind[64..].trim_start_matches(':');
    if !pubkey_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return;
    }

    let filter = crate::types::Filter {
        kinds: Some(vec![kind]),
        authors: Some(vec![pubkey_hex.to_string()]),
        until: Some(event.created_at),
        ..Default::default()
    };

    if let Ok(events) = ctx.store.query(&filter) {
        for matched in events {
            if d_tag.is_empty() || matched.d_tag() == Some(d_tag) {
                if let Some(id) = matched.id_bytes() {
                    match ctx.store.delete_event(&id, author) {
                        Ok(()) => {
                            tracing::info!(
                                "Event {} deleted by {} via kind 5 (a-tag)",
                                matched.id,
                                event.pubkey
                            );
                        }
                        Err(_) => {
                            // may fail if author doesn't match — skip silently
                        }
                    }
                }
            }
        }
    }
}
