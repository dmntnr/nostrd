use crate::nips::{Action, NipContext};
use crate::types::Event;

pub fn check_event(ctx: &NipContext, event: &Event) -> Option<Vec<Action>> {
    let mut actions = Vec::new();

    match event.kind {
        40 => check_channel_creation(event, &mut actions),
        42 => check_channel_message(event, &mut actions),
        43 => check_hide_message(ctx, event, &mut actions),
        44 => check_mute_user(ctx, event, &mut actions),
        _ => return None,
    }

    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

fn check_channel_creation(event: &Event, actions: &mut Vec<Action>) {
    let has_name = event
        .tags
        .iter()
        .any(|t| t.first().map(|s| s.as_str()) == Some("name"));
    if !has_name {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: channel creation requires a name tag",
        ));
    }
}

fn check_channel_message(event: &Event, actions: &mut Vec<Action>) {
    if event.event_refs().is_empty() {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: channel message requires an 'e' tag referencing the channel",
        ));
    }
}

fn check_hide_message(ctx: &NipContext, event: &Event, actions: &mut Vec<Action>) {
    let e_tags = event.event_refs();
    if e_tags.is_empty() {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: hide message requires an 'e' tag referencing the message to hide",
        ));
        return;
    }

    if !is_hide_authorized(ctx, event, &e_tags) {
        actions.push(Action::ok(
            &event.id,
            false,
            "restricted: must be the message author or channel creator to hide",
        ));
    }
}

fn is_hide_authorized(ctx: &NipContext, event: &Event, e_refs: &[&str]) -> bool {
    for e_ref in e_refs {
        if let Some(target) = lookup_event(ctx, e_ref) {
            if target.pubkey == event.pubkey {
                return true;
            }
            if is_channel_creator(ctx, event, &target) {
                return true;
            }
        }
    }
    false
}

fn is_channel_creator(ctx: &NipContext, event: &Event, target: &Event) -> bool {
    for ch_ref in target.event_refs() {
        if let Some(channel) = lookup_event(ctx, ch_ref) {
            if channel.pubkey == event.pubkey {
                return true;
            }
        }
    }
    false
}

fn check_mute_user(ctx: &NipContext, event: &Event, actions: &mut Vec<Action>) {
    if event.pubkey_refs().is_empty() {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: mute user requires a 'p' tag",
        ));
        return;
    }
    if event.event_refs().is_empty() {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: mute user requires an 'e' tag referencing the channel",
        ));
        return;
    }

    if !is_mute_authorized(ctx, event) {
        actions.push(Action::ok(
            &event.id,
            false,
            "restricted: must be the channel creator to mute",
        ));
    }
}

fn is_mute_authorized(ctx: &NipContext, event: &Event) -> bool {
    for e_ref in event.event_refs() {
        if let Some(channel) = lookup_event(ctx, e_ref) {
            if channel.pubkey == event.pubkey {
                return true;
            }
        }
    }
    false
}

fn lookup_event(ctx: &NipContext, hex_id: &str) -> Option<Event> {
    let bytes = hex::decode(hex_id).ok()?;
    let id: [u8; 32] = bytes.as_slice().try_into().ok()?;
    ctx.store.get_event(&id).ok().flatten()
}
