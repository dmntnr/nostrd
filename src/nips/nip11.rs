use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct RelayInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub supported_nips: Vec<u32>,
    pub software: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitation: Option<Limitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payments_url: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Limitation {
    pub max_message_length: usize,
    pub max_subscriptions: usize,
    pub max_filters: usize,
    pub max_limit: usize,
    pub max_subid_length: usize,
    pub max_event_tags: usize,
    pub max_content_length: usize,
    pub min_pow_difficulty: u32,
    pub auth_required: bool,
    pub payment_required: bool,
    pub restricted_writes: bool,
    pub created_at_lower_limit: u64,
    pub created_at_upper_limit: u64,
}

impl RelayInfo {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            name: config.relay_name.clone(),
            description: config.relay_description.clone(),
            pubkey: config.relay_pubkey.clone(),
            contact: config.relay_contact.clone(),
            icon: config.relay_icon.clone(),
            supported_nips: vec![1, 9, 11, 12, 18, 19, 23, 25, 28, 40, 42, 45, 77],
            software: "https://github.com/dmntnr/nostrd".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            limitation: Some(Limitation {
                max_message_length: 524288,
                max_subscriptions: config.max_subscriptions_per_client,
                max_filters: config.max_subscription_filters,
                max_limit: 5000,
                max_subid_length: 256,
                max_event_tags: config.max_event_tags,
                max_content_length: config.max_content_length,
                min_pow_difficulty: 0,
                auth_required: config.auth_required,
                payment_required: false,
                restricted_writes: false,
                created_at_lower_limit: 0,
                created_at_upper_limit: 9999999999,
            }),
            payments_url: None,
        }
    }
}
