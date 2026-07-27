use serde::Deserialize;
use std::net::SocketAddr;

fn deserialize_socket_addr<'de, D>(deserializer: D) -> Result<SocketAddr, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    s.parse()
        .map_err(|e| serde::de::Error::custom(format!("invalid socket address '{}': {}", s, e)))
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(
        default = "default_listen",
        deserialize_with = "deserialize_socket_addr"
    )]
    pub listen_addr: SocketAddr,

    #[serde(default = "default_relay_name")]
    pub relay_name: String,

    #[serde(default = "default_relay_description")]
    pub relay_description: String,

    #[serde(default)]
    pub relay_pubkey: Option<String>,

    #[serde(default)]
    pub relay_contact: Option<String>,

    #[serde(default)]
    pub relay_icon: Option<String>,

    #[serde(default = "default_max_event_age_days")]
    pub max_event_age_days: u64,

    #[serde(default = "default_max_subscription_filters")]
    pub max_subscription_filters: usize,

    #[serde(default = "default_max_subscriptions_per_client")]
    pub max_subscriptions_per_client: usize,

    #[serde(default = "default_max_event_tags")]
    pub max_event_tags: usize,

    #[serde(default = "default_max_content_length")]
    pub max_content_length: usize,

    #[serde(default)]
    pub auth_required: bool,

    #[serde(default = "default_nip42_enabled")]
    pub nip42_enabled: bool,

    #[serde(default = "default_lmdb_map_size_gb")]
    pub lmdb_map_size_gb: usize,

    #[serde(default = "default_broadcast_channel_size")]
    pub broadcast_channel_size: usize,

    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    #[serde(default = "default_max_query_candidates")]
    pub max_query_candidates: usize,

    #[serde(default = "default_max_ws_message_size")]
    pub max_ws_message_size: usize,

    #[serde(default = "default_connection_timeout_secs")]
    pub connection_timeout_secs: u64,

    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,

    #[serde(default = "default_max_events_per_sec")]
    pub max_events_per_sec: usize,

    #[serde(default = "default_max_req_result_limit")]
    pub max_req_result_limit: usize,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 80))
}
fn default_relay_name() -> String {
    "nostrd".into()
}
fn default_relay_description() -> String {
    "A Nostr relay server".into()
}
fn default_max_event_age_days() -> u64 {
    30
}
fn default_max_subscription_filters() -> usize {
    10
}
fn default_max_subscriptions_per_client() -> usize {
    20
}
fn default_max_event_tags() -> usize {
    2000
}
fn default_max_content_length() -> usize {
    100_000
}
fn default_nip42_enabled() -> bool {
    true
}
fn default_lmdb_map_size_gb() -> usize {
    256
}
fn default_broadcast_channel_size() -> usize {
    4096
}
fn default_max_connections() -> usize {
    1000
}
fn default_max_query_candidates() -> usize {
    10_000
}
fn default_max_ws_message_size() -> usize {
    512_000
}
fn default_connection_timeout_secs() -> u64 {
    300
}
fn default_max_sessions() -> usize {
    100_000
}
fn default_max_events_per_sec() -> usize {
    100
}
fn default_max_req_result_limit() -> usize {
    5000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_listen(),
            relay_name: default_relay_name(),
            relay_description: default_relay_description(),
            relay_pubkey: None,
            relay_contact: None,
            relay_icon: None,
            max_event_age_days: default_max_event_age_days(),
            max_subscription_filters: default_max_subscription_filters(),
            max_subscriptions_per_client: default_max_subscriptions_per_client(),
            max_event_tags: default_max_event_tags(),
            max_content_length: default_max_content_length(),
            auth_required: false,
            nip42_enabled: default_nip42_enabled(),
            lmdb_map_size_gb: default_lmdb_map_size_gb(),
            broadcast_channel_size: default_broadcast_channel_size(),
            max_connections: default_max_connections(),
            max_query_candidates: default_max_query_candidates(),
            max_ws_message_size: default_max_ws_message_size(),
            connection_timeout_secs: default_connection_timeout_secs(),
            max_sessions: default_max_sessions(),
            max_events_per_sec: default_max_events_per_sec(),
            max_req_result_limit: default_max_req_result_limit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let c = Config::default();
        assert_eq!(c.relay_name, "nostrd");
        assert_eq!(c.lmdb_map_size_gb, 256);
    }

    #[test]
    fn test_config_toml_parse() {
        let toml_str = r#"
relay_name = "test"
listen_addr = "127.0.0.1:9000"
max_connections = 100
"#;
        let c: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(c.relay_name, "test");
        assert_eq!(c.listen_addr.to_string(), "127.0.0.1:9000");
        assert_eq!(c.max_connections, 100);
        assert_eq!(c.lmdb_map_size_gb, 256); // default
    }
}
