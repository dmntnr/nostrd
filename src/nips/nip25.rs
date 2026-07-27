use crate::nips::Action;
use crate::types::Event;

pub fn check_event(event: &Event) -> Option<Vec<Action>> {
    match event.kind {
        7 => check_reaction(event),
        17 => check_external_reaction(event),
        _ => None,
    }
}

fn check_reaction(event: &Event) -> Option<Vec<Action>> {
    if event.event_refs().is_empty() {
        Some(vec![Action::ok(
            &event.id,
            false,
            "invalid: reaction requires at least one 'e' tag referencing the event being reacted to",
        )])
    } else {
        None
    }
}

fn check_external_reaction(event: &Event) -> Option<Vec<Action>> {
    let has_k = event
        .tags
        .iter()
        .any(|t| t.first().map(|s| s.as_str()) == Some("k"));
    let has_i = event
        .tags
        .iter()
        .any(|t| t.first().map(|s| s.as_str()) == Some("i"));

    if !has_k || !has_i {
        Some(vec![Action::ok(
            &event.id,
            false,
            "invalid: external content reaction requires at least one 'k' tag and one 'i' tag (NIP-73)",
        )])
    } else {
        None
    }
}
