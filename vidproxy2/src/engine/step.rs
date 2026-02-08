use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A step in a phase.
///
/// Each variant carries only the fields it requires, validated at parse time.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum Step {
    /// Navigate the browser to a URL.
    Navigate {
        name: String,
        url: String,
        #[serde(default)]
        wait_for: Option<WaitCondition>,
    },

    /// Wait for a matching network request and extract data from the response.
    Sniff {
        name: String,
        request: RequestMatch,
        extract: HashMap<String, Extractor>,
    },

    /// Collect multiple matching network requests and aggregate extracted data.
    SniffMany {
        name: String,
        request: RequestMatch,
        extract: HashMap<String, Extractor>,
    },

    /// Fetch a URL via HTTP (no browser context).
    Fetch {
        name: String,
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        extract: HashMap<String, Extractor>,
    },

    /// Fetch a URL via the browser context (inherits page cookies/headers).
    FetchInBrowser {
        name: String,
        url: String,
        extract: HashMap<String, Extractor>,
    },

    /// Extract data from the current page's DOM.
    Document {
        name: String,
        extract: HashMap<String, Extractor>,
    },

    /// Execute custom JavaScript in page context.
    Script { name: String, script: String },

    /// Execute browser automation actions (clicks, etc.).
    Automation {
        name: String,
        steps: Vec<AutomationAction>,
    },
}

impl Step {
    /// Get the step name, regardless of variant.
    pub fn name(&self) -> &str {
        match self {
            Step::Navigate { name, .. }
            | Step::Sniff { name, .. }
            | Step::SniffMany { name, .. }
            | Step::Fetch { name, .. }
            | Step::FetchInBrowser { name, .. }
            | Step::Document { name, .. }
            | Step::Script { name, .. }
            | Step::Automation { name, .. } => name,
        }
    }
}

/// Wait condition after navigation or actions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WaitCondition {
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub function: Option<String>,
    #[serde(default)]
    pub delay: Option<f64>,
}

/// Request matching criteria for Sniff/SniffMany steps.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequestMatch {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub idle_timeout: Option<f64>,
}

/// An automation action within an Automation step.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum AutomationAction {
    Click {
        selector: String,
        #[serde(default)]
        wait_for: Option<WaitCondition>,
    },
    ClickIframe {
        selector: String,
        #[serde(default)]
        wait_for: Option<WaitCondition>,
    },
}

/// An extractor that pulls data from a response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Extractor {
    pub kind: ExtractorKind,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub each: Option<HashMap<String, String>>,
    #[serde(default)]
    pub unescape: bool,
}

/// The kind of extractor.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractorKind {
    Url,
    #[serde(rename = "url_regex")]
    UrlRegex,
    Header,
    #[serde(rename = "jsonpath")]
    JsonPath,
    #[serde(rename = "jsonpath_array")]
    JsonPathArray,
    #[serde(rename = "jsonpath_regex")]
    JsonPathRegex,
    Css,
    #[serde(rename = "css_array")]
    CssArray,
    #[serde(rename = "xpath")]
    XPath,
    #[serde(rename = "xpath_array")]
    XPathArray,
    Regex,
    #[serde(rename = "regex_array")]
    RegexArray,
    Line,
    Pssh,
}

/// Check if an extractor kind is an array type.
pub fn is_array_extractor(kind: &ExtractorKind) -> bool {
    matches!(
        kind,
        ExtractorKind::JsonPathArray
            | ExtractorKind::CssArray
            | ExtractorKind::XPathArray
            | ExtractorKind::RegexArray
    )
}
