use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

impl Event {
    pub fn id_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.id).ok()?.try_into().ok()
    }

    pub fn pubkey_bytes(&self) -> Option<[u8; 32]> {
        hex::decode(&self.pubkey).ok()?.try_into().ok()
    }

    pub fn get_tag_values(&self, tag_name: &str) -> Vec<&str> {
        self.tags
            .iter()
            .filter(|t| t.len() >= 2 && t[0] == tag_name)
            .map(|t| t[1].as_str())
            .collect()
    }

    pub fn is_replaceable(&self) -> bool {
        (10_000..20_000).contains(&self.kind) || self.kind == 0 || self.kind == 3 || self.kind == 41
    }

    pub fn is_ephemeral(&self) -> bool {
        (20_000..30_000).contains(&self.kind)
    }

    pub fn is_parameterized_replaceable(&self) -> bool {
        (30_000..40_000).contains(&self.kind)
    }

    pub fn is_protected(&self) -> bool {
        self.tags
            .iter()
            .any(|t| t.first().map(|s| s.as_str()) == Some("-"))
    }

    pub fn expiration(&self) -> Option<u64> {
        self.tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("expiration"))
            .and_then(|t| t.get(1))
            .and_then(|v| v.parse::<u64>().ok())
    }

    pub fn event_refs(&self) -> Vec<&str> {
        self.get_tag_values("e")
    }

    pub fn pubkey_refs(&self) -> Vec<&str> {
        self.get_tag_values("p")
    }

    pub fn d_tag(&self) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("d"))
            .and_then(|t| t.get(1))
            .map(|s| s.as_str())
    }

    #[allow(dead_code)]
    pub fn delegation_tag(&self) -> Option<Vec<&str>> {
        self.tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("delegation"))
            .map(|t| t.iter().map(|s| s.as_str()).collect())
    }

    #[allow(dead_code)]
    pub fn is_regular(&self) -> bool {
        self.kind < 1000 || (self.kind >= 1000 && self.kind < 10000)
    }

    pub fn serialized_for_id(&self) -> String {
        serde_json::to_string(&serde_json::json!([
            0,
            self.pubkey,
            self.created_at,
            self.kind,
            self.tags,
            self.content,
        ]))
        .unwrap_or_default()
    }
}
