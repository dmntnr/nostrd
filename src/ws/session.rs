use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::types::Filter;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub auth_pubkey: Option<String>,
    pub auth_challenge: Option<String>,
    pub subscriptions: HashMap<String, Vec<Filter>>,
    pub event_timestamps: Vec<u64>,
}

impl Session {
    pub fn new(id: String) -> Self {
        Self {
            id,
            auth_pubkey: None,
            auth_challenge: None,
            subscriptions: HashMap::new(),
            event_timestamps: Vec::with_capacity(100),
        }
    }

    pub fn add_subscription(&mut self, sub_id: String, filters: Vec<Filter>) {
        self.subscriptions.insert(sub_id, filters);
    }

    pub fn remove_subscription(&mut self, sub_id: &str) -> Option<Vec<Filter>> {
        self.subscriptions.remove(sub_id)
    }

    pub fn check_rate_limit(&mut self, max_per_sec: usize, now: u64) -> bool {
        let cutoff = now.saturating_sub(1);
        self.event_timestamps.retain(|&t| t >= cutoff);
        if self.event_timestamps.len() >= max_per_sec {
            return false;
        }
        self.event_timestamps.push(now);
        true
    }
}

pub struct SessionManager {
    sessions: RwLock<HashMap<String, Session>>,
}

impl SessionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
        })
    }

    pub async fn get_or_create(&self, id: &str) -> Session {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(id.to_string())
            .or_insert_with(|| Session::new(id.to_string()))
            .clone()
    }

    pub async fn try_create(&self, id: &str, max_sessions: usize) -> Option<Session> {
        let mut sessions = self.sessions.write().await;
        if sessions.len() >= max_sessions && !sessions.contains_key(id) {
            return None;
        }
        Some(
            sessions
                .entry(id.to_string())
                .or_insert_with(|| Session::new(id.to_string()))
                .clone(),
        )
    }

    pub async fn len(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.sessions.read().await.is_empty()
    }

    pub async fn update<F>(&self, id: &str, f: F)
    where
        F: FnOnce(&mut Session),
    {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(id) {
            f(session);
        }
    }

    pub async fn remove(&self, id: &str) -> Option<Session> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(id)
    }

    pub async fn get(&self, id: &str) -> Option<Session> {
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned()
    }
}
