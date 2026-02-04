use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::Notify;

use crate::manifest::{ChannelEntry, StreamInfo};

/**
    State of a source's discovery process.
*/
#[derive(Debug, Clone)]
pub enum SourceState {
    /// Discovery is in progress
    Loading,
    /// Discovery completed successfully
    Ready,
    /// Discovery failed with an error
    Failed(String),
}

impl SourceState {
    pub fn is_loading(&self) -> bool {
        matches!(self, SourceState::Loading)
    }
}

/**
    State of a channel's content resolution process.
*/
#[derive(Debug, Clone)]
pub enum ChannelContentState {
    /// Content phase not yet run
    Pending,
    /// Content phase in progress
    Resolving,
    /// Content phase completed successfully
    Resolved,
    /// Content phase failed
    Failed(String),
}

impl ChannelContentState {
    pub fn is_resolving(&self) -> bool {
        matches!(self, ChannelContentState::Resolving)
    }
}

/**
    Full channel ID combining source and channel ID.
*/
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId {
    pub source: String,
    pub id: String,
}

impl ChannelId {
    pub fn new(source: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            id: id.into(),
        }
    }

    /**
        Parse from "source:id" format
    */
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        let (source, id) = s.split_once(':')?;
        Some(Self::new(source, id))
    }

    /**
        Format as "source:id"
    */
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        format!("{}:{}", self.source, self.id)
    }
}

/**
    In-memory registry of all discovered channels.
*/
pub struct ChannelRegistry {
    channels: RwLock<HashMap<ChannelId, ChannelEntry>>,
    /// When each source's discovery results expire
    discovery_expiration: RwLock<HashMap<String, Option<u64>>>,
    /// Current state of each source (Loading, Ready, Failed)
    source_state: RwLock<HashMap<String, SourceState>>,
    /// Notification handles for waiters on each source
    source_notify: RwLock<HashMap<String, Arc<Notify>>>,
    /// Per-channel content resolution state
    channel_content_state: RwLock<HashMap<ChannelId, ChannelContentState>>,
    /// Notification handles for waiters on channel content resolution
    channel_content_notify: RwLock<HashMap<ChannelId, Arc<Notify>>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            discovery_expiration: RwLock::new(HashMap::new()),
            source_state: RwLock::new(HashMap::new()),
            source_notify: RwLock::new(HashMap::new()),
            channel_content_state: RwLock::new(HashMap::new()),
            channel_content_notify: RwLock::new(HashMap::new()),
        }
    }

    /**
        Mark a source as loading (discovery in progress).
    */
    pub fn mark_source_loading(&self, source_id: &str) {
        let mut states = self.source_state.write().unwrap();
        states.insert(source_id.to_string(), SourceState::Loading);

        // Create notify handle if it doesn't exist
        let mut notifies = self.source_notify.write().unwrap();
        notifies
            .entry(source_id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()));
    }

    /**
        Mark a source as failed.
    */
    pub fn mark_source_failed(&self, source_id: &str, error: impl ToString) {
        {
            let mut states = self.source_state.write().unwrap();
            states.insert(
                source_id.to_string(),
                SourceState::Failed(error.to_string()),
            );
        }

        // Notify any waiters
        let notifies = self.source_notify.read().unwrap();
        if let Some(notify) = notifies.get(source_id) {
            notify.notify_waiters();
        }
    }

    /**
        Get the current state of a source.
    */
    pub fn get_source_state(&self, source_id: &str) -> Option<SourceState> {
        self.source_state.read().unwrap().get(source_id).cloned()
    }

    /**
        Wait for a source to finish loading (with timeout).
        Returns the final state (Ready or Failed), or None if timeout.
    */
    pub async fn wait_for_source(&self, source_id: &str, timeout: Duration) -> Option<SourceState> {
        // Check current state first
        let current_state = self.get_source_state(source_id);
        match &current_state {
            Some(SourceState::Loading) => {
                // Need to wait
            }
            Some(state) => return Some(state.clone()),
            None => return None, // Unknown source
        }

        // Get the notify handle
        let notify = {
            let notifies = self.source_notify.read().unwrap();
            notifies.get(source_id).cloned()
        };

        let Some(notify) = notify else {
            return current_state;
        };

        // Wait with timeout
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
            Err(_timeout) => {
                // Return current state on timeout (might still be Loading)
                self.get_source_state(source_id)
            }
        }
    }

    /**
        Register channels from a source discovery result.
        Also marks the source as Ready and notifies waiters.
    */
    pub fn register_source(
        &self,
        source_name: &str,
        channels: Vec<ChannelEntry>,
        discovery_expires_at: Option<u64>,
    ) {
        {
            let mut registry = self.channels.write().unwrap();

            // Remove old channels from this source
            registry.retain(|id, _| id.source != source_name);

            // Add new channels
            for entry in channels {
                let id = ChannelId::new(source_name, &entry.channel.id);
                registry.insert(id, entry);
            }
        }

        // Update discovery expiration
        {
            let mut expirations = self.discovery_expiration.write().unwrap();
            expirations.insert(source_name.to_string(), discovery_expires_at);
        }

        // Mark source as ready
        {
            let mut states = self.source_state.write().unwrap();
            states.insert(source_name.to_string(), SourceState::Ready);
        }

        // Notify any waiters
        let notifies = self.source_notify.read().unwrap();
        if let Some(notify) = notifies.get(source_name) {
            notify.notify_waiters();
        }
    }

    /**
        Get a channel by its full ID.
    */
    pub fn get(&self, id: &ChannelId) -> Option<ChannelEntry> {
        self.channels.read().unwrap().get(id).cloned()
    }

    /**
        Get a channel by source and channel ID strings.
    */
    #[allow(dead_code)]
    pub fn get_by_parts(&self, source: &str, channel_id: &str) -> Option<ChannelEntry> {
        let id = ChannelId::new(source, channel_id);
        self.get(&id)
    }

    /**
        List all channels.
    */
    #[allow(dead_code)]
    pub fn list_all(&self) -> Vec<(ChannelId, ChannelEntry)> {
        self.channels
            .read()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /**
        List channels from a specific source.
    */
    #[allow(dead_code)]
    pub fn list_by_source(&self, source: &str) -> Vec<ChannelEntry> {
        self.channels
            .read()
            .unwrap()
            .iter()
            .filter(|(id, _)| id.source == source)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /**
        Update stream info for a channel.
    */
    pub fn update_stream_info(&self, id: &ChannelId, stream_info: StreamInfo) {
        let mut registry = self.channels.write().unwrap();
        if let Some(entry) = registry.get_mut(id) {
            entry.stream_info = Some(stream_info);
            entry.last_error = None;
        }
    }

    /**
        Mark a channel as having an error.
    */
    pub fn set_error(&self, id: &ChannelId, error: String) {
        let mut registry = self.channels.write().unwrap();
        if let Some(entry) = registry.get_mut(id) {
            entry.last_error = Some(error);
        }
    }

    /**
        Check if a channel's stream info has expired.
    */
    pub fn is_stream_expired(&self, id: &ChannelId) -> bool {
        let registry = self.channels.read().unwrap();
        if let Some(entry) = registry.get(id) {
            if let Some(ref stream_info) = entry.stream_info
                && let Some(expires_at) = stream_info.expires_at
            {
                return crate::time::now() >= expires_at;
            }
            // No stream info or no expiration = treat as expired
            return entry.stream_info.is_none();
        }
        true // Channel not found = expired
    }

    /**
        Check if a source's discovery has expired.
    */
    pub fn is_discovery_expired(&self, source: &str) -> bool {
        let expirations = self.discovery_expiration.read().unwrap();
        if let Some(Some(expires_at)) = expirations.get(source) {
            return crate::time::now() >= *expires_at;
        }
        // No expiration set = not expired (discovery runs once at startup)
        false
    }

    /**
        Get total channel count.
    */
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.channels.read().unwrap().len()
    }

    /**
        Check if registry is empty.
    */
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.channels.read().unwrap().is_empty()
    }

    // ===== Channel Content Resolution State =====

    /**
        Get the content resolution state for a channel.
        Returns Pending if no state has been set.
    */
    pub fn get_channel_content_state(&self, id: &ChannelId) -> ChannelContentState {
        self.channel_content_state
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or(ChannelContentState::Pending)
    }

    /**
        Mark a channel as having content resolution in progress.
        Creates a notify handle for waiters.
    */
    pub fn mark_channel_resolving(&self, id: &ChannelId) {
        {
            let mut states = self.channel_content_state.write().unwrap();
            states.insert(id.clone(), ChannelContentState::Resolving);
        }

        // Create notify handle if it doesn't exist
        let mut notifies = self.channel_content_notify.write().unwrap();
        notifies
            .entry(id.clone())
            .or_insert_with(|| Arc::new(Notify::new()));
    }

    /**
        Mark a channel's content as successfully resolved.
        Notifies any waiters.
    */
    pub fn mark_channel_resolved(&self, id: &ChannelId) {
        {
            let mut states = self.channel_content_state.write().unwrap();
            states.insert(id.clone(), ChannelContentState::Resolved);
        }

        // Notify any waiters
        let notifies = self.channel_content_notify.read().unwrap();
        if let Some(notify) = notifies.get(id) {
            notify.notify_waiters();
        }
    }

    /**
        Mark a channel's content resolution as failed.
        Notifies any waiters.
    */
    pub fn mark_channel_failed(&self, id: &ChannelId, error: &str) {
        {
            let mut states = self.channel_content_state.write().unwrap();
            states.insert(id.clone(), ChannelContentState::Failed(error.to_string()));
        }

        // Notify any waiters
        let notifies = self.channel_content_notify.read().unwrap();
        if let Some(notify) = notifies.get(id) {
            notify.notify_waiters();
        }
    }

    /**
        Reset a channel's content state back to Pending.
        Used when stream_info expires and needs to be re-resolved.
    */
    pub fn reset_channel_content_state(&self, id: &ChannelId) {
        let mut states = self.channel_content_state.write().unwrap();
        states.insert(id.clone(), ChannelContentState::Pending);
    }

    /**
        Wait for a channel's content to be resolved (with timeout).
        Returns the final state (Resolved or Failed), or None if timeout.
    */
    pub async fn wait_for_channel_content(
        &self,
        id: &ChannelId,
        timeout: Duration,
    ) -> Option<ChannelContentState> {
        // Check current state first
        let current_state = self.get_channel_content_state(id);
        if !current_state.is_resolving() {
            return Some(current_state);
        }

        // Get the notify handle
        let notify = {
            let notifies = self.channel_content_notify.read().unwrap();
            notifies.get(id).cloned()
        };

        let Some(notify) = notify else {
            return Some(current_state);
        };

        // Wait with timeout
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
            Err(_timeout) => {
                // Return current state on timeout (might still be Resolving)
                Some(self.get_channel_content_state(id))
            }
        }
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
