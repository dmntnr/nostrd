use std::collections::HashSet;
use std::sync::Arc;

use crate::nips::{Action, NipContext};
use crate::types::{Event, Filter};
use crate::utils::crypto::validate_event;

const MAX_EVENT_SIZE: usize = 512_000;

pub fn handle_event(ctx: &NipContext, event: &Event) -> Vec<Action> {
    let mut actions = Vec::new();

    match validate_event(event) {
        Ok(()) => {}
        Err(e) => {
            actions.push(Action::ok(
                &event.id,
                false,
                &format!("invalid: {}", e),
            ));
            return actions;
        }
    }

    if event.kind > 65535 {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: kind must be between 0 and 65535",
        ));
        return actions;
    }

    let total_size: usize = event.content.len()
        + event
            .tags
            .iter()
            .flat_map(|t| t.iter())
            .map(|s| s.len())
            .sum::<usize>();
    if total_size > MAX_EVENT_SIZE {
        actions.push(Action::ok(
            &event.id,
            false,
            &format!("invalid: event too large, max {} bytes", MAX_EVENT_SIZE),
        ));
        return actions;
    }

    if event.tags.len() > ctx.config.max_event_tags {
        actions.push(Action::ok(
            &event.id,
            false,
            &format!("invalid: too many tags, max {}", ctx.config.max_event_tags),
        ));
        return actions;
    }

    if event.content.len() > ctx.config.max_content_length {
        actions.push(Action::ok(
            &event.id,
            false,
            &format!(
                "invalid: content too long, max {} bytes",
                ctx.config.max_content_length
            ),
        ));
        return actions;
    }

    let now = chrono::Utc::now().timestamp() as u64;
    let max_age = ctx.config.max_event_age_days.saturating_mul(86400);
    if event.created_at > now + 900 {
        actions.push(Action::ok(
            &event.id,
            false,
            "invalid: created_at is too far in the future",
        ));
        return actions;
    }
    if now.saturating_sub(event.created_at) > max_age {
        actions.push(Action::ok(&event.id, false, "invalid: event is too old"));
        return actions;
    }

    if event.is_ephemeral() {
        let actions = if event.kind == 22242 {
            vec![Action::ok(&event.id, true, "")]
        } else {
            vec![
                Action::ok(&event.id, true, "ephemeral: relayed but not stored"),
                Action::BroadcastEvent(Arc::new(event.clone())),
            ]
        };
        return actions;
    }

    match ctx.store.add_event(event) {
        Ok(crate::db::AddEventResult::New) => {
            actions.push(Action::ok(&event.id, true, ""));
            actions.push(Action::BroadcastEvent(Arc::new(event.clone())));
        }
        Ok(crate::db::AddEventResult::Duplicate) => {
            actions.push(Action::ok(
                &event.id,
                true,
                "duplicate: event already exists",
            ));
        }
        Ok(crate::db::AddEventResult::Replaced(n)) => {
            actions.push(Action::ok(
                &event.id,
                true,
                &format!("replaced: {} old event(s) replaced", n),
            ));
            actions.push(Action::BroadcastEvent(Arc::new(event.clone())));
        }
        Err(_) => {
            actions.push(Action::ok(&event.id, false, "error: storage error"));
        }
    }

    actions
}

pub fn handle_req(
    ctx: &NipContext,
    subscription_id: &str,
    filters: &[Filter],
    auth_pubkey: Option<&[u8; 32]>,
) -> Vec<Action> {
    let mut actions = Vec::new();

    if filters.is_empty() {
        actions.push(Action::notice("error: REQ requires at least one filter"));
        actions.push(Action::closed(subscription_id, "invalid: no filters provided"));
        return actions;
    }

    if filters.len() > ctx.config.max_subscription_filters {
        actions.push(Action::closed(
            subscription_id,
            &format!(
                "invalid: too many filters, max {}",
                ctx.config.max_subscription_filters
            ),
        ));
        return actions;
    }

    let mut seen_ids: HashSet<String> = HashSet::new();

    for filter in filters {
        let mut effective_filter = filter.clone();
        let max_limit = ctx.config.max_req_result_limit;
        if effective_filter.is_empty() && effective_filter.limit.is_none() {
            effective_filter.limit = Some(max_limit);
        }
        if let Some(ref mut l) = effective_filter.limit {
            *l = (*l).min(max_limit);
        }

        match ctx.store.query(&effective_filter) {
            Ok(events) => {
                for event in events {
                    let event = Arc::new(event);
                    if event.is_protected() && auth_pubkey.is_none() {
                        continue;
                    }
                    if seen_ids.insert(event.id.clone()) {
                        actions.push(Action::send_event(subscription_id, event));
                    }
                }
            }
            Err(_) => {
                actions.push(Action::notice("error: query failed"));
            }
        }
    }

    actions.push(Action::eose(subscription_id));
    actions
}

pub fn handle_close(subscription_id: &str) -> Vec<Action> {
    vec![Action::CloseSubscription(
        subscription_id.to_string(),
        "closed by client".to_string(),
    )]
}
