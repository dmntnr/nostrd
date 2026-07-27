use heed::types::{Bytes, Unit};
use heed::{Database, Env, EnvOpenOptions, RoTxn, RwTxn};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::db::AddEventResult;
use crate::error::{RelayError, Result};
use crate::types::{Event, Filter};

const MAX_CANDIDATES: usize = 10_000;

const DB_EVENTS: &str = "events";
const DB_AUTHOR_INDEX: &str = "author_index";
const DB_KIND_INDEX: &str = "kind_index";
const DB_TAG_INDEX: &str = "tag_index";
const DB_TIME_INDEX: &str = "time_index";

const MAX_TAG_KEY_PAYLOAD: usize = 400;

pub struct LmdbStore {
    env: Env,
    events: Database<Bytes, Bytes>,
    author_index: Database<Bytes, Unit>,
    kind_index: Database<Bytes, Unit>,
    tag_index: Database<Bytes, Unit>,
    time_index: Database<Bytes, Unit>,
    max_candidates: usize,
}

fn map_lmdb_err(e: heed::Error) -> RelayError {
    let msg = e.to_string();
    if msg.contains("MDB_MAP_FULL") || msg.contains("map full") {
        RelayError::Storage(
            "LMDB map size exceeded. Increase 'lmdb_map_size_gb' in nostrd.toml and restart."
                .into(),
        )
    } else {
        RelayError::Storage("LMDB operation failed".into())
    }
}

impl LmdbStore {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        Self::open_with_map_size(path, 1024 * 1024 * 1024, MAX_CANDIDATES)
    }

    pub fn open_with_map_size(path: &Path, map_size: usize, max_candidates: usize) -> Result<Arc<Self>> {
        std::fs::create_dir_all(path)
            .map_err(|_| RelayError::Storage("failed to create data directory".into()))?;

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(map_size)
                .max_dbs(5)
                .open(path)
        }
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("MDB_MAP_FULL") || msg.contains("map full") {
                RelayError::Storage(format!(
                    "LMDB map size too small (current: {} GB). \
                     Increase 'lmdb_map_size_gb' in nostrd.toml and restart. \
                     Note: map_size uses virtual memory, not physical RAM.",
                    map_size / (1024 * 1024 * 1024)
                ))
            } else {
                RelayError::Storage("failed to open LMDB environment".into())
            }
        })?;

        let mut wtxn = env
            .write_txn()
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        let events: Database<Bytes, Bytes> = env
            .create_database(&mut wtxn, Some(DB_EVENTS))
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        let author_index: Database<Bytes, Unit> = env
            .create_database(&mut wtxn, Some(DB_AUTHOR_INDEX))
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        let kind_index: Database<Bytes, Unit> = env
            .create_database(&mut wtxn, Some(DB_KIND_INDEX))
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        let tag_index: Database<Bytes, Unit> =
            env.create_database(&mut wtxn, Some(DB_TAG_INDEX))
                .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        let time_index: Database<Bytes, Unit> = env
            .create_database(&mut wtxn, Some(DB_TIME_INDEX))
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        wtxn.commit()
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;

        Ok(Arc::new(Self {
            env,
            events,
            author_index,
            kind_index,
            tag_index,
            time_index,
            max_candidates,
        }))
    }

    fn read_txn(&self) -> Result<RoTxn<'_>> {
        self.env
            .read_txn()
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))
    }

    fn write_txn(&self) -> Result<RwTxn<'_>> {
        self.env.write_txn().map_err(map_lmdb_err)
    }
}

fn make_author_key(pubkey: &[u8; 32], created_at: u64, id: &[u8; 32]) -> [u8; 72] {
    let mut key = [0u8; 72];
    key[0..32].copy_from_slice(pubkey);
    key[32..40].copy_from_slice(&created_at.to_be_bytes());
    key[40..72].copy_from_slice(id);
    key
}

fn make_kind_key(kind: u64, created_at: u64, id: &[u8; 32]) -> [u8; 48] {
    let mut key = [0u8; 48];
    key[0..8].copy_from_slice(&kind.to_be_bytes());
    key[8..16].copy_from_slice(&created_at.to_be_bytes());
    key[16..48].copy_from_slice(id);
    key
}

fn make_time_key(created_at: u64, id: &[u8; 32]) -> [u8; 40] {
    let mut key = [0u8; 40];
    key[0..8].copy_from_slice(&created_at.to_be_bytes());
    key[8..40].copy_from_slice(id);
    key
}

fn make_tag_key(tag_name: &[u8], tag_value: &[u8], created_at: u64, id: &[u8; 32]) -> Vec<u8> {
    let max_name = tag_name
        .len()
        .min(u8::MAX as usize)
        .min(MAX_TAG_KEY_PAYLOAD / 2);
    let max_value = tag_value
        .len()
        .min(u8::MAX as usize)
        .min(MAX_TAG_KEY_PAYLOAD - max_name);
    let mut key = Vec::with_capacity(2 + max_name + max_value + 8 + 32);
    key.push(max_name as u8);
    key.extend_from_slice(&tag_name[..max_name]);
    key.push(max_value as u8);
    key.extend_from_slice(&tag_value[..max_value]);
    key.extend_from_slice(&created_at.to_be_bytes());
    key.extend_from_slice(id);
    key
}

fn make_tag_prefix(tag_name: &str, tag_value: &str) -> Vec<u8> {
    let tn = tag_name.as_bytes();
    let tv = tag_value.as_bytes();
    let max_name = tn.len().min(u8::MAX as usize).min(MAX_TAG_KEY_PAYLOAD / 2);
    let max_value = tv
        .len()
        .min(u8::MAX as usize)
        .min(MAX_TAG_KEY_PAYLOAD - max_name);
    let mut prefix = Vec::with_capacity(2 + max_name + max_value);
    prefix.push(max_name as u8);
    prefix.extend_from_slice(&tn[..max_name]);
    prefix.push(max_value as u8);
    prefix.extend_from_slice(&tv[..max_value]);
    prefix
}

impl LmdbStore {
    pub fn add_event(&self, event: &Event) -> Result<AddEventResult> {
        let id = event
            .id_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid id hex".into()))?;
        let pubkey = event
            .pubkey_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid pubkey hex".into()))?;

        let serialized = serde_json::to_vec(event)?;

        let mut wtxn = self.write_txn()?;

        if self
            .events
            .get(&wtxn, &id)
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            .is_some()
        {
            return Ok(AddEventResult::Duplicate);
        }

        if (event.is_replaceable() || event.is_parameterized_replaceable())
            && self.has_newer_replaceable(&wtxn, event)?
        {
            return Ok(AddEventResult::Duplicate);
        }

        self.events
            .put(&mut wtxn, &id, &serialized)
            .map_err(map_lmdb_err)?;

        let author_key = make_author_key(&pubkey, event.created_at, &id);
        self.author_index
            .put(&mut wtxn, &author_key, &())
            .map_err(map_lmdb_err)?;

        let kind_key = make_kind_key(event.kind, event.created_at, &id);
        self.kind_index
            .put(&mut wtxn, &kind_key, &())
            .map_err(map_lmdb_err)?;

        let time_key = make_time_key(event.created_at, &id);
        self.time_index
            .put(&mut wtxn, &time_key, &())
            .map_err(map_lmdb_err)?;

        for tag in &event.tags {
            if tag.len() >= 2 {
                let tag_key =
                    make_tag_key(tag[0].as_bytes(), tag[1].as_bytes(), event.created_at, &id);
                self.tag_index
                    .put(&mut wtxn, &tag_key, &())
                    .map_err(map_lmdb_err)?;
            }
        }

        let mut replaced_count = 0;
        if event.is_replaceable() || event.is_parameterized_replaceable() {
            replaced_count = self.remove_previous_replaceable(&mut wtxn, event)?;
        }

        wtxn.commit().map_err(map_lmdb_err)?;

        Ok(if replaced_count > 0 {
            AddEventResult::Replaced(replaced_count)
        } else {
            AddEventResult::New
        })
    }

    fn has_newer_replaceable(&self, wtxn: &RwTxn, event: &Event) -> Result<bool> {
        let pubkey = event
            .pubkey_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid pubkey hex".into()))?;

        let author_prefix = &pubkey;

        for result in self
            .author_index
            .prefix_iter(wtxn, author_prefix)
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
        {
            let (key, _) = result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
            if key.len() < 72 {
                continue;
            }
            let ev_id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);

            if let Some(existing_bytes) = self
                .events
                .get(wtxn, &ev_id)
                .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            {
                if let Ok(existing) = serde_json::from_slice::<Event>(existing_bytes) {
                    let same_replaceable = event.is_replaceable()
                        && existing.is_replaceable()
                        && existing.kind == event.kind;
                    let same_param_replaceable = event.is_parameterized_replaceable()
                        && existing.is_parameterized_replaceable()
                        && existing.kind == event.kind
                        && existing.d_tag() == event.d_tag();

                    if same_replaceable || same_param_replaceable {
                        if existing.created_at > event.created_at {
                            return Ok(true);
                        }
                        if existing.created_at == event.created_at
                            && existing.id < event.id
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    fn remove_previous_replaceable(&self, wtxn: &mut RwTxn, event: &Event) -> Result<usize> {
        let pubkey = event
            .pubkey_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid pubkey hex".into()))?;

        let author_prefix = &pubkey;
        let mut to_remove: Vec<[u8; 32]> = Vec::new();

        for result in self
            .author_index
            .prefix_iter(wtxn, author_prefix)
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
        {
            let (key, _) = result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
            if key.len() < 72 {
                continue;
            }
            let ev_id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);

            if let Some(existing_bytes) = self
                .events
                .get(wtxn, &ev_id)
                .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            {
                if let Ok(existing) = serde_json::from_slice::<Event>(existing_bytes) {
                    if (event.is_replaceable()
                        && existing.is_replaceable()
                        && existing.kind == event.kind
                        && existing.created_at < event.created_at)
                        || (event.is_parameterized_replaceable()
                            && existing.is_parameterized_replaceable()
                            && existing.kind == event.kind
                            && existing.d_tag() == event.d_tag()
                            && existing.created_at < event.created_at)
                        || (event.is_replaceable()
                            && existing.is_replaceable()
                            && existing.kind == event.kind
                            && existing.created_at == event.created_at
                            && existing.id > event.id)
                        || (event.is_parameterized_replaceable()
                            && existing.is_parameterized_replaceable()
                            && existing.kind == event.kind
                            && existing.d_tag() == event.d_tag()
                            && existing.created_at == event.created_at
                            && existing.id > event.id)
                    {
                        to_remove.push(ev_id);
                    }
                }
            }
        }

        let new_id = event.id_bytes().unwrap_or([0u8; 32]);
        to_remove.retain(|id| id != &new_id);

        for id in &to_remove {
            if let Some(existing_bytes) = self
                .events
                .get(wtxn, id)
                .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            {
                if let Ok(existing) = serde_json::from_slice::<Event>(existing_bytes) {
                    self.remove_event_internal(wtxn, &existing)?;
                }
            }
        }

        Ok(to_remove.len())
    }

    fn remove_event_internal(&self, wtxn: &mut RwTxn, event: &Event) -> Result<()> {
        let id = event
            .id_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid id".into()))?;
        let pubkey = event
            .pubkey_bytes()
            .ok_or_else(|| RelayError::InvalidEvent("invalid pubkey".into()))?;

        self.events.delete(wtxn, &id).map_err(map_lmdb_err)?;
        let author_key = make_author_key(&pubkey, event.created_at, &id);
        self.author_index
            .delete(wtxn, &author_key)
            .map_err(map_lmdb_err)?;
        let kind_key = make_kind_key(event.kind, event.created_at, &id);
        self.kind_index
            .delete(wtxn, &kind_key)
            .map_err(map_lmdb_err)?;
        let time_key = make_time_key(event.created_at, &id);
        self.time_index
            .delete(wtxn, &time_key)
            .map_err(map_lmdb_err)?;
        for tag in &event.tags {
            if tag.len() >= 2 {
                let tag_key =
                    make_tag_key(tag[0].as_bytes(), tag[1].as_bytes(), event.created_at, &id);
                self.tag_index
                    .delete(wtxn, &tag_key)
                    .map_err(map_lmdb_err)?;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_event(&self, id: &[u8; 32]) -> Result<Option<Event>> {
        let rtxn = self.read_txn()?;
        self.events
            .get(&rtxn, id)
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))
            .map(|opt| opt.and_then(|bytes| serde_json::from_slice(bytes).ok()))
    }

    pub fn delete_event(&self, event_id: &[u8; 32], deleter: &[u8; 32]) -> Result<()> {
        let mut wtxn = self.write_txn()?;

        let event = self
            .events
            .get(&wtxn, event_id)
            .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            .and_then(|bytes| serde_json::from_slice::<Event>(bytes).ok());

        if let Some(event) = event {
            let event_pubkey = event.pubkey_bytes().unwrap_or_default();
            if event_pubkey != *deleter {
                return Err(RelayError::PermissionDenied(
                    "only the author can delete their event".into(),
                ));
            }
            self.remove_event_internal(&mut wtxn, &event)?;
        }

        wtxn.commit().map_err(map_lmdb_err)?;
        Ok(())
    }

    pub fn query(&self, filter: &Filter) -> Result<Vec<Event>> {
        let rtxn = self.read_txn()?;
        let mut candidate_ids: HashSet<[u8; 32]> =
            HashSet::with_capacity(self.max_candidates.min(1024));
        let since = filter.since;
        let until = filter.until;

        if let Some(ref ids) = filter.ids {
            for id_str in ids {
                if let Ok(id) = hex::decode(id_str) {
                    if let Ok(id) = id.try_into() {
                        candidate_ids.insert(id);
                    }
                }
            }
        } else if let Some(ref authors) = filter.authors {
            for author in authors {
                if let Ok(pubkey) = hex::decode(author) {
                    if let Ok(pubkey) = pubkey.as_slice().try_into() {
                        let pubkey: &[u8; 32] = pubkey;
                        let iter = self.author_index.prefix_iter(&rtxn, pubkey).map_err(|_| {
                            RelayError::Storage("failed to open LMDB environment".into())
                        })?;
                        for result in iter {
                            let (key, _) = result
                                .map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
                            if key.len() >= 72 {
                                let ts =
                                    u64::from_be_bytes(key[32..40].try_into().unwrap_or([0u8; 8]));
                                if let Some(s) = since {
                                    if ts < s {
                                        continue;
                                    }
                                }
                                if let Some(u) = until {
                                    if ts > u {
                                        break;
                                    }
                                }
                                let id: [u8; 32] =
                                    key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);
                                candidate_ids.insert(id);
                            }
                        }
                    }
                }
            }
        } else if let Some(ref e_tags) = filter.e_tags {
            for e_tag in e_tags {
                let prefix = make_tag_prefix("e", e_tag);
                let iter = self
                    .tag_index
                    .prefix_iter(&rtxn, &prefix)
                    .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;
                for result in iter {
                    let (key, _) =
                        result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
                    if key.len() >= prefix.len() + 8 + 32 {
                        let ts_off = key.len() - 40;
                        let ts = u64::from_be_bytes(
                            key[ts_off..ts_off + 8].try_into().unwrap_or([0u8; 8]),
                        );
                        if let Some(s) = since {
                            if ts < s {
                                continue;
                            }
                        }
                        if let Some(u) = until {
                            if ts > u {
                                break;
                            }
                        }
                        let id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);
                        candidate_ids.insert(id);
                    }
                }
            }
        } else if let Some(ref p_tags) = filter.p_tags {
            for p_tag in p_tags {
                let prefix = make_tag_prefix("p", p_tag);
                let iter = self
                    .tag_index
                    .prefix_iter(&rtxn, &prefix)
                    .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;
                for result in iter {
                    let (key, _) =
                        result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
                    if key.len() >= prefix.len() + 8 + 32 {
                        let ts_off = key.len() - 40;
                        let ts = u64::from_be_bytes(
                            key[ts_off..ts_off + 8].try_into().unwrap_or([0u8; 8]),
                        );
                        if let Some(s) = since {
                            if ts < s {
                                continue;
                            }
                        }
                        if let Some(u) = until {
                            if ts > u {
                                break;
                            }
                        }
                        let id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);
                        candidate_ids.insert(id);
                    }
                }
            }
        } else if let Some(ref kinds) = filter.kinds {
            for kind in kinds {
                let kind_prefix = kind.to_be_bytes();
                let iter = self
                    .kind_index
                    .prefix_iter(&rtxn, &kind_prefix)
                    .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;
                for result in iter {
                    let (key, _) =
                        result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
                    if key.len() >= 48 {
                        let ts = u64::from_be_bytes(key[8..16].try_into().unwrap_or([0u8; 8]));
                        if let Some(s) = since {
                            if ts < s {
                                continue;
                            }
                        }
                        if let Some(u) = until {
                            if ts > u {
                                break;
                            }
                        }
                        let id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);
                        candidate_ids.insert(id);
                    }
                }
            }
        } else {
            let iter = self
                .time_index
                .iter(&rtxn)
                .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;
            for result in iter {
                let (key, _) =
                    result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
                if key.len() >= 40 {
                    let ts = u64::from_be_bytes(key[..8].try_into().unwrap_or([0u8; 8]));
                    if let Some(s) = since {
                        if ts < s {
                            continue;
                        }
                    }
                    if let Some(u) = until {
                        if ts > u {
                            break;
                        }
                    }
                    let id: [u8; 32] = key[key.len() - 32..].try_into().unwrap_or([0u8; 32]);
                    candidate_ids.insert(id);
                }
            }
        }

        let mut events: Vec<Event> = Vec::with_capacity(candidate_ids.len().min(self.max_candidates));
        for id in &candidate_ids {
            if events.len() >= self.max_candidates.saturating_sub(1) {
                break;
            }
            if let Some(bytes) = self
                .events
                .get(&rtxn, id)
                .map_err(|_e| RelayError::Storage("LMDB read error".into()))?
            {
                if let Ok(event) = serde_json::from_slice::<Event>(bytes) {
                    if let Some(exp) = event.expiration() {
                        let now = chrono::Utc::now().timestamp() as u64;
                        if now >= exp {
                            continue;
                        }
                    }
                    if filter.matches_event(&event) {
                        events.push(event);
                    }
                }
            }
        }

        events.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });

        if let Some(limit) = filter.limit {
            events.truncate(limit);
        }

        Ok(events)
    }

    #[allow(dead_code)]
    pub fn count(&self, filter: &Filter) -> Result<usize> {
        let events = self.query(filter)?;
        Ok(events.len())
    }

    #[allow(dead_code)]
    pub fn get_all_event_ids(&self) -> Result<Vec<[u8; 32]>> {
        let rtxn = self.read_txn()?;
        let mut ids = Vec::new();
        let iter = self
            .events
            .iter(&rtxn)
            .map_err(|_| RelayError::Storage("failed to open LMDB environment".into()))?;
        for result in iter {
            let (key, _) = result.map_err(|_e| RelayError::Storage("LMDB read error".into()))?;
            if key.len() == 32 {
                ids.push(key.try_into().unwrap_or([0u8; 32]));
            }
        }
        Ok(ids)
    }
}

pub struct StoreStats {
    pub event_count: usize,
}

impl LmdbStore {
    pub fn stats(&self) -> Result<StoreStats> {
        let rtxn = self.read_txn()?;
        let count =
            self.events
                .len(&rtxn)
                .map_err(|_e| RelayError::Storage("LMDB read error".into()))? as usize;
        Ok(StoreStats { event_count: count })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Event;

    fn hex32(s: &str) -> String {
        format!("{:0>64}", s)
    }

    fn make_event(
        id: &str,
        pubkey: &str,
        created_at: u64,
        kind: u64,
        tags: Vec<Vec<String>>,
    ) -> Event {
        Event {
            id: hex32(id),
            pubkey: hex32(pubkey),
            created_at,
            kind,
            tags,
            content: String::new(),
            sig: hex32("sig"),
        }
    }

    #[test]
    fn test_follow_set_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = LmdbStore::open(dir.path()).unwrap();

        // Publish kind 3 (follow set) for author A, following B
        let event1 = make_event("e1", "a1", 100, 3, vec![vec!["p".into(), hex32("b1")]]);
        let result = store.add_event(&event1).unwrap();
        assert_eq!(result, AddEventResult::New);

        // Verify it's queryable
        let filter = Filter {
            kinds: Some(vec![3]),
            authors: Some(vec![hex32("a1")]),
            ..Default::default()
        };
        let results = store.query(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, hex32("e1"));

        // Verify #p query works (who follows "b")
        let filter_p = Filter {
            kinds: Some(vec![3]),
            p_tags: Some(vec![hex32("b1")]),
            ..Default::default()
        };
        let results = store.query(&filter_p).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, hex32("e1"));

        // Publish replacement kind 3 (author A follows C instead)
        let event2 = make_event("e2", "a1", 200, 3, vec![vec!["p".into(), hex32("c1")]]);
        let result = store.add_event(&event2).unwrap();
        assert_eq!(result, AddEventResult::Replaced(1));

        // Old event should be gone
        let results = store.query(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, hex32("e2"));

        // Query who follows "b" should now return empty (A no longer follows B)
        let results = store.query(&filter_p).unwrap();
        assert_eq!(results.len(), 0);

        // Query who follows "c" should return A
        let filter_c = Filter {
            kinds: Some(vec![3]),
            p_tags: Some(vec![hex32("c1")]),
            ..Default::default()
        };
        let results = store.query(&filter_c).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, hex32("e2"));

        // Publish duplicate should return Duplicate
        let result = store.add_event(&event2).unwrap();
        assert_eq!(result, AddEventResult::Duplicate);
    }
}
