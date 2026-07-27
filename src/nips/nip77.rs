use std::collections::HashMap;
use std::sync::Mutex;

use crate::nips::{Action, NipContext};
use crate::types::{Filter, RelayMessage};

// NIP-77: Negentropy Syncing

const MAX_SESSION_IDS: usize = 10_000;

pub struct NegentropySessions {
    pub(crate) sessions: Mutex<HashMap<String, NegentropySession>>,
}

impl Default for NegentropySessions {
    fn default() -> Self {
        Self::new()
    }
}

impl NegentropySessions {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

pub(crate) struct NegentropySession {
    server_ids: Vec<[u8; 32]>,
    server_boundary: usize,
}

pub fn handle_neg_open(
    ctx: &NipContext,
    subscription_id: &str,
    filter: &Filter,
    _initial_message: &str,
    neg_sessions: &NegentropySessions,
) -> Vec<Action> {
    let mut actions = Vec::new();

    match ctx.store.query(filter) {
        Ok(events) => {
            let ids: Vec<[u8; 32]> = events
                .iter()
                .take(MAX_SESSION_IDS)
                .map(|e| e.id_bytes().unwrap_or_default())
                .collect();

            if ids.is_empty() {
                actions.push(Action::Send(RelayMessage::NegErr {
                    subscription_id: subscription_id.to_string(),
                    code: "error: empty set, no events match the filter".to_string(),
                }));
                return actions;
            }

            let first_id = hex::encode(ids[0]);
            let last_id = hex::encode(ids[ids.len() - 1]);
            let initial_msg = format!("{}:{}/{}", ids.len(), first_id, last_id);

            let session = NegentropySession {
                server_ids: ids,
                server_boundary: 0,
            };

            if let Ok(mut s) = neg_sessions.sessions.lock() {
                s.insert(subscription_id.to_string(), session);
            }

            actions.push(Action::Send(RelayMessage::NegMsg {
                subscription_id: subscription_id.to_string(),
                message: initial_msg,
            }));
        }
        Err(_) => {
            actions.push(Action::Send(RelayMessage::NegErr {
                subscription_id: subscription_id.to_string(),
                code: "error: query failed".to_string(),
            }));
        }
    }

    actions
}

pub fn handle_neg_msg(
    _ctx: &NipContext,
    subscription_id: &str,
    message: &str,
    neg_sessions: &NegentropySessions,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut sessions = match neg_sessions.sessions.lock() {
        Ok(s) => s,
        Err(_) => {
            return vec![Action::Send(RelayMessage::NegErr {
                subscription_id: subscription_id.to_string(),
                code: "error: internal error".to_string(),
            })];
        }
    };

    if let Some(session) = sessions.get_mut(subscription_id) {
        if message == "c" || message == "continue" {
            let remaining = session.server_ids.len() - session.server_boundary;
            let batch_size = remaining.min(20);
            let batch: Vec<String> = session.server_ids
                [session.server_boundary..session.server_boundary + batch_size]
                .iter()
                .map(hex::encode)
                .collect();

            session.server_boundary += batch_size;
            let msg = format!("e:{}", batch.join(","));

            if session.server_boundary >= session.server_ids.len() {
                sessions.remove(subscription_id);
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: format!("{}/done", msg),
                }));
                actions.push(Action::Send(RelayMessage::Closed {
                    subscription_id: subscription_id.to_string(),
                    message: "neg-sync-complete".to_string(),
                }));
            } else {
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: msg,
                }));
            }
        } else if let Some(have_list) = message.strip_prefix("have:") {
            let client_have: Vec<String> =
                have_list.split(',').map(|s| s.trim().to_string()).collect();

            let have_set: std::collections::HashSet<String> = client_have.into_iter().collect();

            let missing: Vec<String> = session
                .server_ids
                .iter()
                .map(hex::encode)
                .filter(|id| !have_set.contains(id))
                .collect();

            if missing.is_empty() {
                sessions.remove(subscription_id);
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: "complete/all-synced".to_string(),
                }));
                actions.push(Action::Send(RelayMessage::Closed {
                    subscription_id: subscription_id.to_string(),
                    message: "neg-sync-complete".to_string(),
                }));
            } else {
                let msg = format!("e:{}", missing.join(","));
                sessions.remove(subscription_id);
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: format!("{}/done", msg),
                }));
                actions.push(Action::Send(RelayMessage::Closed {
                    subscription_id: subscription_id.to_string(),
                    message: "neg-sync-complete".to_string(),
                }));
            }
        } else if let Some(fp) = message.strip_prefix("fingerprint:") {
            let our_fp = compute_fingerprint(&session.server_ids);
            if our_fp != fp {
                let all_ids: Vec<String> = session.server_ids.iter().map(hex::encode).collect();
                sessions.remove(subscription_id);
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: format!("e:{}/done", all_ids.join(",")),
                }));
                actions.push(Action::Send(RelayMessage::Closed {
                    subscription_id: subscription_id.to_string(),
                    message: "neg-sync-complete".to_string(),
                }));
            } else {
                sessions.remove(subscription_id);
                actions.push(Action::Send(RelayMessage::NegMsg {
                    subscription_id: subscription_id.to_string(),
                    message: "complete/fingerprint-match".to_string(),
                }));
                actions.push(Action::Send(RelayMessage::Closed {
                    subscription_id: subscription_id.to_string(),
                    message: "neg-sync-complete".to_string(),
                }));
            }
        } else {
            sessions.remove(subscription_id);
            actions.push(Action::Send(RelayMessage::NegErr {
                subscription_id: subscription_id.to_string(),
                code: "error: unknown sync message".to_string(),
            }));
        }
    } else {
        actions.push(Action::Send(RelayMessage::NegErr {
            subscription_id: subscription_id.to_string(),
                code: "error: no active negentropy session".to_string(),
        }));
    }

    actions
}

pub fn handle_neg_close(subscription_id: &str, neg_sessions: &NegentropySessions) -> Vec<Action> {
    neg_sessions
        .sessions
        .lock()
        .ok()
        .map(|mut s| s.remove(subscription_id));
    vec![Action::CloseSubscription(
        subscription_id.to_string(),
        "negentropy closed".to_string(),
    )]
}

fn compute_fingerprint(ids: &[[u8; 32]]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let limit = ids.len().min(16);
    for id in ids.iter().take(limit) {
        hasher.update(id);
    }
    hex::encode(&hasher.finalize()[..4])
}
