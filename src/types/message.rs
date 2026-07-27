use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::{Event, Filter};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "UPPERCASE")]
pub enum RelayMessage {
    #[serde(rename = "EVENT")]
    Event {
        subscription_id: String,
        #[serde(skip)]
        event: Arc<Event>,
    },

    #[serde(rename = "OK")]
    Ok {
        event_id: String,
        success: bool,
        message: String,
    },

    #[serde(rename = "EOSE")]
    Eose { subscription_id: String },

    #[serde(rename = "NOTICE")]
    Notice { message: String },

    #[serde(rename = "AUTH")]
    Auth { challenge: String },

    #[serde(rename = "COUNT")]
    Count {
        subscription_id: String,
        count: usize,
    },

    #[serde(rename = "NEG-MSG")]
    NegMsg {
        subscription_id: String,
        message: String,
    },

    #[serde(rename = "NEG-ERR")]
    NegErr {
        subscription_id: String,
        code: String,
    },

    #[serde(rename = "CLOSED")]
    Closed {
        subscription_id: String,
        message: String,
    },
}

impl RelayMessage {
    pub fn event(sub_id: impl Into<String>, event: Arc<Event>) -> Self {
        Self::Event {
            subscription_id: sub_id.into(),
            event,
        }
    }

    pub fn ok(event_id: impl Into<String>, success: bool, message: impl Into<String>) -> Self {
        Self::Ok {
            event_id: event_id.into(),
            success,
            message: message.into(),
        }
    }

    pub fn eose(sub_id: impl Into<String>) -> Self {
        Self::Eose {
            subscription_id: sub_id.into(),
        }
    }

    pub fn notice(message: impl Into<String>) -> Self {
        Self::Notice {
            message: message.into(),
        }
    }

    pub fn auth(challenge: impl Into<String>) -> Self {
        Self::Auth {
            challenge: challenge.into(),
        }
    }

    pub fn count(sub_id: impl Into<String>, count: usize) -> Self {
        Self::Count {
            subscription_id: sub_id.into(),
            count,
        }
    }

    pub fn closed(sub_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Closed {
            subscription_id: sub_id.into(),
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> String {
        // Relay messages are sent as JSON arrays: ["EVENT", sub_id, event]
        match self {
            Self::Event {
                subscription_id,
                event,
            } => {
                serde_json::to_string(&json!(["EVENT", subscription_id, event.as_ref()])).unwrap_or_default()
            }
            Self::Ok {
                event_id,
                success,
                message,
            } => serde_json::to_string(&json!(["OK", event_id, success, message]))
                .unwrap_or_default(),
            Self::Eose { subscription_id } => {
                serde_json::to_string(&json!(["EOSE", subscription_id])).unwrap_or_default()
            }
            Self::Notice { message } => {
                serde_json::to_string(&json!(["NOTICE", message])).unwrap_or_default()
            }
            Self::Auth { challenge } => {
                serde_json::to_string(&json!(["AUTH", challenge])).unwrap_or_default()
            }
            Self::Count {
                subscription_id,
                count,
            } => serde_json::to_string(&json!(["COUNT", subscription_id, { "count": count }]))
                .unwrap_or_default(),
            Self::NegMsg {
                subscription_id,
                message,
            } => serde_json::to_string(&json!(["NEG-MSG", subscription_id, message]))
                .unwrap_or_default(),
            Self::NegErr {
                subscription_id,
                code,
            } => serde_json::to_string(&json!(["NEG-ERR", subscription_id, code]))
                .unwrap_or_default(),
            Self::Closed {
                subscription_id,
                message,
            } => serde_json::to_string(&json!(["CLOSED", subscription_id, message]))
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ClientMessage {
    Event {
        #[serde(rename = "EVENT")]
        event: Event,
    },
    Req {
        #[serde(rename = "REQ")]
        subscription_id: String,
        #[serde(skip)]
        filters: Vec<Filter>,
        #[serde(skip)]
        _parse_error: bool,
    },
    Close {
        #[serde(rename = "CLOSE")]
        subscription_id: String,
    },
    Auth {
        #[serde(rename = "AUTH")]
        event: Event,
    },
    Count {
        #[serde(rename = "COUNT")]
        subscription_id: String,
        #[serde(skip)]
        filters: Vec<Filter>,
        #[serde(skip)]
        _parse_error: bool,
    },
    NegOpen {
        #[serde(rename = "NEG-OPEN")]
        subscription_id: String,
        #[serde(skip)]
        filter: Filter,
        #[serde(skip)]
        initial_message: String,
    },
    NegMsg {
        #[serde(rename = "NEG-MSG")]
        subscription_id: String,
        #[serde(skip)]
        message: String,
    },
    NegClose {
        #[serde(rename = "NEG-CLOSE")]
        subscription_id: String,
    },
}

pub fn parse_client_message(s: &str) -> Option<ClientMessage> {
    let value: serde_json::Value = serde_json::from_str(s).ok()?;
    let arr = value.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let msg_type = arr[0].as_str()?;

    match msg_type {
        "EVENT" => {
            let event: Event = serde_json::from_value(arr.get(1)?.clone()).ok()?;
            Some(ClientMessage::Event { event })
        }
        "REQ" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            let mut filters = Vec::new();
            let mut had_error = false;
            for v in arr.iter().skip(2) {
                match serde_json::from_value(v.clone()) {
                    Ok(f) => filters.push(f),
                    Err(_) => had_error = true,
                }
            }
            if filters.is_empty() && !had_error {
                filters.push(Filter::default());
            }
            Some(ClientMessage::Req {
                subscription_id,
                filters,
                _parse_error: had_error,
            })
        }
        "CLOSE" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            Some(ClientMessage::Close { subscription_id })
        }
        "AUTH" => {
            let event: Event = serde_json::from_value(arr.get(1)?.clone()).ok()?;
            Some(ClientMessage::Auth { event })
        }
        "COUNT" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            let mut filters = Vec::new();
            let mut had_error = false;
            for v in arr.iter().skip(2) {
                match serde_json::from_value(v.clone()) {
                    Ok(f) => filters.push(f),
                    Err(_) => had_error = true,
                }
            }
            if filters.is_empty() && !had_error {
                filters.push(Filter::default());
            }
            Some(ClientMessage::Count {
                subscription_id,
                filters,
                _parse_error: had_error,
            })
        }
        "NEG-OPEN" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            let filter: Filter = serde_json::from_value(arr.get(2)?.clone()).ok()?;
            let initial_message = arr
                .get(3)
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default();
            Some(ClientMessage::NegOpen {
                subscription_id,
                filter,
                initial_message,
            })
        }
        "NEG-MSG" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            let message = arr
                .get(2)
                .map(|v| v.as_str().unwrap_or("").to_string())
                .unwrap_or_default();
            Some(ClientMessage::NegMsg {
                subscription_id,
                message,
            })
        }
        "NEG-CLOSE" => {
            let subscription_id = arr.get(1)?.as_str()?.to_string();
            Some(ClientMessage::NegClose { subscription_id })
        }
        _ => None,
    }
}
