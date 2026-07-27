use crate::nips::Action;
use crate::types::Event;

pub fn check_event(event: &Event) -> Option<Vec<Action>> {
    match event.kind {
        6 => check_repost(event),
        16 => None,
        _ => None,
    }
}

fn check_repost(event: &Event) -> Option<Vec<Action>> {
    if event.event_refs().is_empty() {
        Some(vec![Action::ok(
            &event.id,
            false,
            "invalid: repost requires at least one 'e' tag referencing the event being reposted",
        )])
    } else {
        None
    }
}
