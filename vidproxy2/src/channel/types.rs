use chrono::{DateTime, Utc};

/// Full channel ID combining source and channel ID.
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

    pub fn _parse(s: &str) -> Option<Self> {
        let (source, id) = s.split_once(':')?;
        Some(Self::new(source, id))
    }

    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        format!("{}:{}", self.source, self.id)
    }
}

/// A discovered channel.
#[derive(Debug, Clone)]
pub struct Channel {
    #[allow(dead_code)]
    pub source_id: String,
    pub id: String,
    pub name: Option<String>,
    pub image: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
}

/// Stream info from the content phase.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub manifest_url: String,
    pub license_url: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub headers: Vec<(String, String)>,
}

/// A single EPG programme entry.
#[derive(Debug, Clone)]
pub struct Programme {
    pub title: String,
    pub description: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub episode: Option<String>,
    pub season: Option<String>,
    pub genres: Vec<String>,
    pub image: Option<String>,
    pub is_live: Option<bool>,
}

/// Full channel entry combining discovery, metadata, and content info.
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub channel: Channel,
    pub stream_info: Option<StreamInfo>,
    pub programmes: Vec<Programme>,
    pub last_error: Option<String>,
}

impl ChannelEntry {
    /// Check if the channel is currently live based on EPG data.
    ///
    /// Finds the currently-airing programme by matching timestamps and checks
    /// its `is_live` field. Returns `true` (assume live) when there's no
    /// programme data or no `is_live` field.
    pub fn is_live_now(&self) -> bool {
        if self.programmes.is_empty() {
            return true;
        }

        let now = crate::util::time::now();

        let current = self
            .programmes
            .iter()
            .find(|p| p.start_time <= now && now < p.end_time);

        match current {
            Some(prog) => prog.is_live.unwrap_or(true),
            None => {
                // No programme covers "now". If any programme in the schedule has
                // `is_live` data, this source provides availability info — assume
                // offline rather than blindly assuming live. This handles the case
                // where the schedule is stale / expired (e.g. yesterday's schedule).
                let has_live_info = self.programmes.iter().any(|p| p.is_live.is_some());
                !has_live_info
            }
        }
    }
}

/// State of a source's discovery process.
#[derive(Debug, Clone)]
pub enum SourceState {
    Loading,
    Ready,
    Failed(String),
}

impl SourceState {
    pub fn is_loading(&self) -> bool {
        matches!(self, SourceState::Loading)
    }
}

/// State of a channel's content resolution process.
#[derive(Debug, Clone)]
pub enum ChannelContentState {
    Pending,
    Resolving,
    Resolved,
    Failed(String),
}

impl ChannelContentState {
    pub fn is_resolving(&self) -> bool {
        matches!(self, ChannelContentState::Resolving)
    }
}
