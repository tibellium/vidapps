use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/**
    A source manifest defining how to discover channels and extract stream info.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub source: Source,
    pub discovery: DiscoveryPhase,
    /// Optional processing phase to filter and transform channels
    #[serde(default)]
    pub process: Option<ProcessPhase>,
    /// Optional metadata phase to extract EPG data per channel
    #[serde(default)]
    pub metadata: Option<MetadataPhase>,
    pub content: ContentPhase,
}

/**
    Processing phase - filter and transform discovered channels.
*/
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProcessPhase {
    /// Filter to select which channels to include
    #[serde(default)]
    pub filter: Option<ChannelFilter>,
    /// Transforms to apply to matching channels
    #[serde(default)]
    pub transforms: Vec<Transform>,
}

/**
    Filter to apply to discovered channels.
    Only channels matching the filter criteria will have content phase run.
*/
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChannelFilter {
    /// Filter by channel name (exact match, case-sensitive)
    #[serde(default)]
    pub name: Vec<String>,
    /// Filter by channel ID (exact match)
    #[serde(default)]
    pub id: Vec<String>,
}

/**
    A transform to apply to channels.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum Transform {
    /// Add a category/genre to channels matching by name or id
    AddCategory {
        /// Channel name to match (optional)
        #[serde(default)]
        name: Option<String>,
        /// Channel ID to match (optional)
        #[serde(default)]
        id: Option<String>,
        /// Category value to add
        category: String,
    },
    /// Add a description to channels matching by name or id
    AddDescription {
        /// Channel name to match (optional)
        #[serde(default)]
        name: Option<String>,
        /// Channel ID to match (optional)
        #[serde(default)]
        id: Option<String>,
        /// Description value to add
        description: String,
    },
    /// Rename channels matching by name or id
    Rename {
        /// Channel name to match (optional)
        #[serde(default)]
        name: Option<String>,
        /// Channel ID to match (optional)
        #[serde(default)]
        id: Option<String>,
        /// New name to set
        to: String,
    },
}

/**
    Source metadata.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    /// Unique identifier for the source (used in URLs, registry keys, etc.)
    pub id: String,
    /// Display name for the source
    pub name: String,
    /// Optional SOCKS5 proxy URL (e.g., "socks5://127.0.0.1:1080")
    #[serde(default)]
    pub proxy: Option<String>,
    /// ISO 3166-1 alpha-2 country code (e.g., "CO" for Colombia, "US" for United States)
    #[serde(default)]
    pub country: Option<String>,
    /// ISO 639-1 language code (e.g., "es" for Spanish, "en" for English)
    #[serde(default)]
    pub language: Option<String>,
    /// Run browser in headless mode for this source
    #[serde(default)]
    pub headless: bool,
}

/**
    Discovery phase - finds all channels from a source.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryPhase {
    pub steps: Vec<Step>,
    pub outputs: DiscoveryOutputs,
}

/**
    Outputs from the discovery phase (per-channel).
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryOutputs {
    /// Channel ID (required, supports interpolation)
    pub id: String,
    /// Channel name (optional, supports interpolation)
    #[serde(default)]
    pub name: Option<String>,
    /// Channel thumbnail/logo URL (optional, supports interpolation)
    #[serde(default)]
    pub image: Option<String>,
    /// Expiration timestamp for discovery results (optional, supports interpolation)
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Static expiration duration in seconds (alternative to expires_at)
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/**
    Metadata phase - optional, extracts EPG data per channel.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetadataPhase {
    pub steps: Vec<Step>,
    pub outputs: MetadataOutputs,
}

/**
    Outputs from the metadata phase (per-channel EPG data).
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetadataOutputs {
    /// Reference to the extracted programmes array (supports interpolation)
    pub programmes: String,
    /// Static expiration duration in seconds for metadata
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/**
    Content phase - fetches stream info for a single channel.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentPhase {
    pub steps: Vec<Step>,
    pub outputs: ContentOutputs,
}

/**
    Outputs from the content phase.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentOutputs {
    /// The DASH manifest URL (required, supports interpolation)
    pub manifest_url: String,
    /// License URL for DRM content (optional, supports interpolation)
    #[serde(default)]
    pub license_url: Option<String>,
    /// Expiration timestamp for stream info (optional, supports interpolation)
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Static expiration duration in seconds (alternative to expires_at)
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/**
    A step in a phase.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Step {
    /// Unique name for this step (used in interpolation)
    pub name: String,
    /// The kind of step
    pub kind: StepKind,
    /// URL for Navigate steps (supports interpolation)
    #[serde(default)]
    pub url: Option<String>,
    /// Wait condition for Navigate steps
    #[serde(default)]
    pub wait_for: Option<WaitCondition>,
    /// Request matching for Sniff steps
    #[serde(default)]
    pub request: Option<RequestMatch>,
    /// Extractors to run on the response
    #[serde(default)]
    pub extract: HashMap<String, Extractor>,
}

/**
    Wait condition after navigation.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WaitCondition {
    /// Wait for a CSS selector to appear
    #[serde(default)]
    pub selector: Option<String>,
    /// Wait for a JS expression to return truthy
    #[serde(default)]
    pub function: Option<String>,
    /// Additional delay in seconds after other conditions
    #[serde(default)]
    pub delay: Option<f64>,
}

/**
    The kind of step.
*/
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum StepKind {
    /// Navigate browser to a URL
    Navigate,
    /// Wait for a matching network request and extract data
    Sniff,
    /// Collect multiple matching network requests and aggregate extracted data
    SniffMany,
    /// Fetch a URL via HTTP (no browser needed)
    Fetch,
}

/**
    Request matching criteria for Sniff steps.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestMatch {
    /// URL regex pattern
    pub url: String,
    /// HTTP method filter (GET, POST, etc.)
    #[serde(default)]
    pub method: Option<String>,
    /// Timeout in seconds (default: 30)
    #[serde(default)]
    pub timeout: Option<f64>,
    /// For SniffMany: stop collecting after this many seconds of no new matches
    #[serde(default)]
    pub idle_timeout: Option<f64>,
}

/**
    An extractor that pulls data from a response.
*/
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Extractor {
    /**
        The kind of extractor
    */
    pub kind: ExtractorKind,
    /**
        Path/pattern for the extractor (JSONPath, XPath, regex, etc.)
    */
    #[serde(default)]
    pub path: Option<String>,
    /**
        Default value if extraction fails or capture group is missing (regex)
    */
    #[serde(default)]
    pub default: Option<String>,
    /**
        For jsonpath_regex: regex pattern to apply after JSONPath extraction
    */
    #[serde(default)]
    pub regex: Option<String>,
    /**
        For jsonpath_array: sub-extractors to apply to each array element
        Supports $parent.field syntax to reference parent objects in nested paths
    */
    #[serde(default)]
    pub each: Option<HashMap<String, String>>,
    /**
        Unescape JSON unicode sequences (e.g., \\u0026 -> &) in the extracted value
    */
    #[serde(default)]
    pub unescape: bool,
}

/**
    The kind of extractor.
*/
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractorKind {
    /**
        Capture the request URL itself
    */
    Url,
    /**
        Regex with capture group on the request URL
    */
    UrlRegex,
    /**
        JSONPath query on JSON response body (returns single value)
    */
    JsonPath,
    /**
        JSONPath query returning array of objects with sub-extractors
        Supports $parent.field syntax in `each` to reference parent objects
    */
    #[serde(rename = "jsonpath_array")]
    JsonPathArray,
    /**
        JSONPath query followed by regex extraction on the result
    */
    #[serde(rename = "jsonpath_regex")]
    JsonPathRegex,
    /**
        XPath query on XML response body
    */
    XPath,
    /**
        Regex with capture group on response body
    */
    Regex,
    /**
        Regex over response body returning array of objects
        Uses `each` to map fields to capture groups
    */
    #[serde(rename = "regex_array")]
    RegexArray,
    /**
        First line containing ":" (for CDRM key response)
    */
    Line,
    /**
        Extract Widevine PSSH from MPD manifest
    */
    Pssh,
}

/**
    A discovered channel from the discovery phase.
*/
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DiscoveredChannel {
    pub id: String,
    pub name: Option<String>,
    pub image: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub source: String,
}

/**
    Stream info from the content phase.
*/
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub manifest_url: String,
    pub license_url: Option<String>,
    pub expires_at: Option<u64>,
}

/**
    A single EPG programme entry.
*/
#[derive(Debug, Clone)]
pub struct Programme {
    pub title: String,
    pub description: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub episode: Option<String>,
    pub season: Option<String>,
    pub genres: Vec<String>,
    pub image: Option<String>,
}

/**
    Full channel entry combining discovery, metadata, and content info.
*/
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub channel: DiscoveredChannel,
    pub stream_info: Option<StreamInfo>,
    pub programmes: Vec<Programme>,
    pub last_error: Option<String>,
}
