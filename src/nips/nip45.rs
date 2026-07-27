use crate::nips::{Action, NipContext};
use crate::types::Filter;

pub fn handle_count(
    ctx: &NipContext,
    subscription_id: &str,
    filters: &[Filter],
    auth_pubkey: Option<&[u8; 32]>,
) -> Vec<Action> {
    let mut total_count: usize = 0;

    for filter in filters {
        let mut effective_filter = filter.clone();
        // For COUNT, remove any limit to count all matching events accurately
        effective_filter.limit = None;

        match ctx.store.query(&effective_filter) {
            Ok(events) => {
                for event in events {
                    if event.is_protected() && auth_pubkey.is_none() {
                        continue;
                    }
                    total_count = total_count.saturating_add(1);
                }
            }
            Err(_) => {
                return vec![Action::closed(
                    subscription_id,
                    "error: count query failed",
                )];
            }
        }
    }

    vec![Action::count(subscription_id, total_count)]
}
