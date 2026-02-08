use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Notify;

use super::types::{ChannelContentState, ChannelEntry, ChannelId, SourceState, StreamInfo};

/**
    In-memory registry of all discovered channels.
*/
pub struct ChannelRegistry {
    channels: RwLock<HashMap<ChannelId, ChannelEntry>>,
    discovery_expiration: RwLock<HashMap<String, Option<DateTime<Utc>>>>,
    metadata_expiration: RwLock<HashMap<String, Option<DateTime<Utc>>>>,
    source_state: RwLock<HashMap<String, SourceState>>,
    source_notify: RwLock<HashMap<String, Arc<Notify>>>,
    channel_content_state: RwLock<HashMap<ChannelId, ChannelContentState>>,
    channel_content_notify: RwLock<HashMap<ChannelId, Arc<Notify>>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            discovery_expiration: RwLock::new(HashMap::new()),
            metadata_expiration: RwLock::new(HashMap::new()),
            source_state: RwLock::new(HashMap::new()),
            source_notify: RwLock::new(HashMap::new()),
            channel_content_state: RwLock::new(HashMap::new()),
            channel_content_notify: RwLock::new(HashMap::new()),
        }
    }

    // ── Source state ─────────────────────────────────────────────────────

    pub fn mark_source_loading(&self, source_id: &str) {
        let mut states = self.source_state.write().unwrap();
        states.insert(source_id.to_string(), SourceState::Loading);

        let mut notifies = self.source_notify.write().unwrap();
        notifies
            .entry(source_id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()));
    }

    pub fn mark_source_failed(&self, source_id: &str, error: impl ToString) {
        {
            let mut states = self.source_state.write().unwrap();
            states.insert(
                source_id.to_string(),
                SourceState::Failed(error.to_string()),
            );
        }
        let notifies = self.source_notify.read().unwrap();
        if let Some(notify) = notifies.get(source_id) {
            notify.notify_waiters();
        }
    }

    pub fn get_source_state(&self, source_id: &str) -> Option<SourceState> {
        self.source_state.read().unwrap().get(source_id).cloned()
    }

    pub async fn wait_for_source(&self, source_id: &str, timeout: Duration) -> Option<SourceState> {
        let current_state = self.get_source_state(source_id);
        match &current_state {
            Some(SourceState::Loading) => {}
            Some(state) => return Some(state.clone()),
            None => return None,
        }

        let notify = {
            let notifies = self.source_notify.read().unwrap();
            notifies.get(source_id).cloned()
        };

        let Some(notify) = notify else {
            return current_state;
        };

        let result = tokio::time::timeout(timeout, async {
            loop {
                notify.notified().await;
                let state = self.get_source_state(source_id);
                if let Some(ref s) = state {
                    if !s.is_loading() {
                        return state;
                    }
                } else {
                    return state;
                }
            }
        })
        .await;

        match result {
            Ok(state) => state,
            Err(_timeout) => self.get_source_state(source_id),
        }
    }

    // ── Channel registration ─────────────────────────────────────────────

    pub fn register_source(
        &self,
        source_name: &str,
        channels: Vec<ChannelEntry>,
        discovery_expires_at: Option<DateTime<Utc>>,
    ) {
        {
            let mut registry = self.channels.write().unwrap();
            registry.retain(|id, _| id.source != source_name);
            for entry in channels {
                let id = ChannelId::new(source_name, &entry.channel.id);
                registry.insert(id, entry);
            }
        }

        {
            let mut expirations = self.discovery_expiration.write().unwrap();
            expirations.insert(source_name.to_string(), discovery_expires_at);
        }

        {
            let mut states = self.source_state.write().unwrap();
            states.insert(source_name.to_string(), SourceState::Ready);
        }

        let notifies = self.source_notify.read().unwrap();
        if let Some(notify) = notifies.get(source_name) {
            notify.notify_waiters();
        }
    }

    pub fn get(&self, id: &ChannelId) -> Option<ChannelEntry> {
        self.channels.read().unwrap().get(id).cloned()
    }

    pub fn list_by_source(&self, source: &str) -> Vec<ChannelEntry> {
        self.channels
            .read()
            .unwrap()
            .iter()
            .filter(|(id, _)| id.source == source)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn update_stream_info(&self, id: &ChannelId, stream_info: StreamInfo) {
        let mut registry = self.channels.write().unwrap();
        if let Some(entry) = registry.get_mut(id) {
            entry.stream_info = Some(stream_info);
            entry.last_error = None;
        }
    }

    pub fn set_error(&self, id: &ChannelId, error: String) {
        let mut registry = self.channels.write().unwrap();
        if let Some(entry) = registry.get_mut(id) {
            entry.last_error = Some(error);
        }
    }

    pub fn is_stream_expired(&self, id: &ChannelId) -> bool {
        let registry = self.channels.read().unwrap();
        if let Some(entry) = registry.get(id) {
            if let Some(ref stream_info) = entry.stream_info
                && let Some(expires_at) = stream_info.expires_at
            {
                return crate::util::time::now() >= expires_at;
            }
            return entry.stream_info.is_none();
        }
        true
    }

    pub fn is_discovery_expired(&self, source: &str) -> bool {
        let expirations = self.discovery_expiration.read().unwrap();
        if let Some(Some(expires_at)) = expirations.get(source) {
            return crate::util::time::now() >= *expires_at;
        }
        false
    }

    // ── Metadata expiration ──────────────────────────────────────────────

    pub fn set_metadata_expiration(&self, source: &str, expires_at: Option<DateTime<Utc>>) {
        let mut expirations = self.metadata_expiration.write().unwrap();
        expirations.insert(source.to_string(), expires_at);
    }

    pub fn is_metadata_expired(&self, source: &str) -> bool {
        let expirations = self.metadata_expiration.read().unwrap();
        if let Some(Some(expires_at)) = expirations.get(source) {
            return crate::util::time::now() >= *expires_at;
        }
        false
    }

    /**
        Update programmes for channels in a source (from a metadata refresh).
    */
    pub fn update_programmes(
        &self,
        source: &str,
        mut programmes_by_channel: HashMap<String, Vec<super::types::Programme>>,
    ) {
        let mut registry = self.channels.write().unwrap();
        for (id, entry) in registry.iter_mut() {
            if id.source == source {
                entry.programmes = programmes_by_channel
                    .remove(&entry.channel.id)
                    .unwrap_or_default();
            }
        }
    }

    // ── Channel content state (atomic check-and-mark) ────────────────────

    /**
        Try to start resolving content for a channel.
        Returns `true` if this caller won the race and should do the resolution.
        Returns `false` if another caller is already resolving.
    */
    pub fn try_mark_resolving(&self, id: &ChannelId) -> bool {
        let mut states = self.channel_content_state.write().unwrap();
        match states.get(id) {
            Some(ChannelContentState::Resolving) => false,
            _ => {
                states.insert(id.clone(), ChannelContentState::Resolving);

                let mut notifies = self.channel_content_notify.write().unwrap();
                notifies
                    .entry(id.clone())
                    .or_insert_with(|| Arc::new(Notify::new()));

                true
            }
        }
    }

    pub fn get_channel_content_state(&self, id: &ChannelId) -> ChannelContentState {
        self.channel_content_state
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or(ChannelContentState::Pending)
    }

    pub fn mark_channel_resolved(&self, id: &ChannelId) {
        {
            let mut states = self.channel_content_state.write().unwrap();
            states.insert(id.clone(), ChannelContentState::Resolved);
        }
        let notifies = self.channel_content_notify.read().unwrap();
        if let Some(notify) = notifies.get(id) {
            notify.notify_waiters();
        }
    }

    pub fn mark_channel_failed(&self, id: &ChannelId, error: &str) {
        {
            let mut states = self.channel_content_state.write().unwrap();
            states.insert(id.clone(), ChannelContentState::Failed(error.to_string()));
        }
        let notifies = self.channel_content_notify.read().unwrap();
        if let Some(notify) = notifies.get(id) {
            notify.notify_waiters();
        }
    }

    pub fn reset_channel_content_state(&self, id: &ChannelId) {
        let mut states = self.channel_content_state.write().unwrap();
        states.insert(id.clone(), ChannelContentState::Pending);
    }

    pub async fn wait_for_channel_content(
        &self,
        id: &ChannelId,
        timeout: Duration,
    ) -> Option<ChannelContentState> {
        let current_state = self.get_channel_content_state(id);
        if !current_state.is_resolving() {
            return Some(current_state);
        }

        let notify = {
            let notifies = self.channel_content_notify.read().unwrap();
            notifies.get(id).cloned()
        };

        let Some(notify) = notify else {
            return Some(current_state);
        };

        let result = tokio::time::timeout(timeout, async {
            loop {
                notify.notified().await;
                let state = self.get_channel_content_state(id);
                if !state.is_resolving() {
                    return state;
                }
            }
        })
        .await;

        match result {
            Ok(state) => Some(state),
            Err(_timeout) => Some(self.get_channel_content_state(id)),
        }
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
