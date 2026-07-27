use crate::nips::Action;
use crate::types::Event;

pub fn check_expiration(event: &Event) -> Option<Vec<Action>> {
    if let Some(expiration) = event.expiration() {
        let now = chrono::Utc::now().timestamp() as u64;
        if now >= expiration {
            return Some(vec![Action::ok(&event.id, false, "invalid: event has expired")]);
        }
    }
    None
}
