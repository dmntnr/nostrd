use crate::nips::Action;
use crate::types::Event;

pub fn check_event(_event: &Event) -> Option<Vec<Action>> {
    if _event.kind != 30023 {
        return None;
    }

    if _event.d_tag().is_none() {
        Some(vec![Action::ok(
            &_event.id,
            false,
            "invalid: long-form content requires a 'd' tag",
        )])
    } else {
        None
    }
}
