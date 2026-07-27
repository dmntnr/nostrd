use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<u64>>,

    #[serde(rename = "#e", skip_serializing_if = "Option::is_none")]
    pub e_tags: Option<Vec<String>>,

    #[serde(rename = "#p", skip_serializing_if = "Option::is_none")]
    pub p_tags: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Filter {
    pub fn generic_tags(&self) -> Vec<(&str, &str)> {
        let mut tags = Vec::new();
        for (key, value) in &self.extra {
            if key.len() >= 2 && key.starts_with('#') {
                let tag_name = &key[1..];
                if let Some(values) = value.as_array() {
                    for v in values {
                        if let Some(s) = v.as_str() {
                            tags.push((tag_name, s));
                        }
                    }
                }
            }
        }
        tags
    }

    fn is_empty_vec<T>(opt: &Option<Vec<T>>) -> bool {
        opt.as_ref().is_none_or(|v| v.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        Self::is_empty_vec(&self.ids)
            && Self::is_empty_vec(&self.authors)
            && Self::is_empty_vec(&self.kinds)
            && Self::is_empty_vec(&self.e_tags)
            && Self::is_empty_vec(&self.p_tags)
            && self.since.is_none()
            && self.until.is_none()
            && self.extra.is_empty()
    }

    pub fn matches_event(&self, event: &super::Event) -> bool {
        if let Some(ref ids) = self.ids {
            if !ids.is_empty() && !ids.contains(&event.id) {
                return false;
            }
        }
        if let Some(ref authors) = self.authors {
            if !authors.is_empty() && !authors.contains(&event.pubkey) {
                return false;
            }
        }
        if let Some(ref kinds) = self.kinds {
            if !kinds.is_empty() && !kinds.contains(&event.kind) {
                return false;
            }
        }
        if let Some(ref e_tags) = self.e_tags {
            if !e_tags.is_empty() {
                let event_e_tags = event.event_refs();
                if !e_tags.iter().any(|t| event_e_tags.contains(&t.as_str())) {
                    return false;
                }
            }
        }
        if let Some(ref p_tags) = self.p_tags {
            if !p_tags.is_empty() {
                let event_p_tags = event.pubkey_refs();
                if !p_tags.iter().any(|t| event_p_tags.contains(&t.as_str())) {
                    return false;
                }
            }
        }

        let generic = self.generic_tags();
        if !generic.is_empty() {
            let mut grouped: std::collections::HashMap<&str, Vec<&str>> =
                std::collections::HashMap::new();
            for (name, value) in &generic {
                grouped.entry(name).or_default().push(value);
            }
            for (tag_name, filter_values) in &grouped {
                if !filter_values.is_empty() {
                    let event_values = event.get_tag_values(tag_name);
                    let any_match = filter_values.iter().any(|fv| event_values.contains(fv));
                    if !any_match {
                        return false;
                    }
                }
            }
        }

        if let Some(since) = self.since {
            if event.created_at < since {
                return false;
            }
        }
        if let Some(until) = self.until {
            if event.created_at > until {
                return false;
            }
        }
        true
    }
}
