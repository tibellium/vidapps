use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A channel manifest defining how to discover and extract stream credentials.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub channel: Channel,
    pub steps: Vec<Step>,
    pub outputs: Outputs,
}

/// Channel metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Channel {
    /// Display name for the channel
    pub name: String,
    /// Optional SOCKS5 proxy URL (e.g., "socks5://127.0.0.1:1080")
    #[serde(default)]
    pub proxy: Option<String>,
}

/// A step in the discovery process.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Step {
    /// Unique name for this step (used in interpolation)
    pub name: String,
    /// The kind of step
    pub kind: StepKind,
    /// URL for Navigate steps (supports interpolation)
    #[serde(default)]
    pub url: Option<String>,
    /// Request matching for Sniff steps
    #[serde(default)]
    pub request: Option<RequestMatch>,
    /// PSSH for CdrmRequest steps (supports interpolation)
    #[serde(default)]
    pub pssh: Option<String>,
    /// License URL for CdrmRequest steps (supports interpolation)
    #[serde(default)]
    pub license_url: Option<String>,
    /// Extractors to run on the response
    #[serde(default)]
    pub extract: HashMap<String, Extractor>,
}

/// The kind of step.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum StepKind {
    /// Navigate browser to a URL
    Navigate,
    /// Wait for a matching network request and extract data
    Sniff,
    /// Call CDRM API to get decryption keys
    CdrmRequest,
}

/// Request matching criteria for Sniff steps.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestMatch {
    /// URL glob pattern (e.g., "*unity.tbxapis.com*/items/*.json")
    pub url: String,
    /// HTTP method filter (GET, POST, etc.)
    #[serde(default)]
    pub method: Option<String>,
}

/// An extractor that pulls data from a response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Extractor {
    /// The kind of extractor
    pub kind: ExtractorKind,
    /// Path/pattern for the extractor (JSONPath, XPath, regex, etc.)
    #[serde(default)]
    pub path: Option<String>,
}

/// The kind of extractor.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractorKind {
    /// Capture the request URL itself
    Url,
    /// JSONPath query on JSON response body
    JsonPath,
    /// XPath query on XML response body
    XPath,
    /// Regex with capture group on response body
    Regex,
    /// First line containing ":" (for CDRM key response)
    Line,
    /// Extract Widevine PSSH from MPD manifest (uses ffmpeg-source DASH parser)
    Pssh,
}

/// Final outputs from manifest execution.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Outputs {
    /// The manifest/stream URL (supports interpolation)
    pub mpd_url: String,
    /// The decryption key in "kid:key" format (supports interpolation)
    pub decryption_key: String,
}

/// Resolved outputs after execution.
#[derive(Debug, Clone)]
pub struct ManifestOutputs {
    pub mpd_url: String,
    pub decryption_key: String,
}
