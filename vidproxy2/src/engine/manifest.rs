use anyhow::{Result, anyhow};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};

use super::step::Step;

/// Embedded source manifests directory.
static SOURCES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/sources");

/// A source manifest defining how to discover channels and extract stream info.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub source: Source,
    pub discovery: DiscoveryPhase,
    #[serde(default)]
    pub process: Option<ProcessPhase>,
    #[serde(default)]
    pub metadata: Option<MetadataPhase>,
    pub content: ContentPhase,
}

/// Source metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub headless: Option<bool>,
}

/// Browser configuration for a phase (proxy and headless settings).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub headless: Option<bool>,
}

impl BrowserConfig {
    /// Resolve with fallback to source-level defaults.
    pub fn resolve(&self, source: &Source) -> ResolvedBrowserConfig {
        ResolvedBrowserConfig {
            proxy: self.proxy.clone().or_else(|| source.proxy.clone()),
            headless: self.headless.or(source.headless).unwrap_or(true),
        }
    }
}

/// Resolved browser configuration with concrete values.
pub struct ResolvedBrowserConfig {
    pub proxy: Option<String>,
    pub headless: bool,
}

/// Discovery phase - finds all channels from a source.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryPhase {
    #[serde(flatten)]
    pub browser: BrowserConfig,
    pub steps: Vec<Step>,
    pub outputs: DiscoveryOutputs,
}

/// Outputs from the discovery phase (per-channel).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscoveryOutputs {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// Processing phase - filter and transform discovered channels.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProcessPhase {
    #[serde(default)]
    pub filter: Option<ChannelFilter>,
    #[serde(default)]
    pub transforms: Vec<Transform>,
}

/// Filter to apply to discovered channels.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChannelFilter {
    #[serde(default)]
    pub name: Vec<String>,
    #[serde(default)]
    pub id: Vec<String>,
}

/// A transform to apply to channels.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind")]
pub enum Transform {
    AddCategory {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        id: Option<String>,
        category: String,
    },
    AddDescription {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        id: Option<String>,
        description: String,
    },
    Rename {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        id: Option<String>,
        to: String,
    },
}

/// Metadata phase - extracts EPG data per channel.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetadataPhase {
    #[serde(flatten)]
    pub browser: BrowserConfig,
    pub steps: Vec<Step>,
    pub outputs: MetadataOutputs,
}

/// Outputs from the metadata phase.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetadataOutputs {
    pub programmes: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// Content phase - fetches stream info for a single channel.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentPhase {
    #[serde(flatten)]
    pub browser: BrowserConfig,
    pub steps: Vec<Step>,
    pub outputs: ContentOutputs,
}

/// Outputs from the content phase.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentOutputs {
    pub manifest_url: String,
    #[serde(default)]
    pub license_url: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
}

/// Load all available source manifests.
pub fn load_all() -> Result<Vec<Manifest>> {
    let mut manifests = Vec::new();

    for file in SOURCES_DIR.files() {
        let path = file.path();
        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            let content = file
                .contents_utf8()
                .ok_or_else(|| anyhow!("Failed to read {:?} as UTF-8", path))?;

            let manifest: Manifest = serde_yaml::from_str(content)
                .map_err(|e| anyhow!("Failed to parse {:?}: {}", path, e))?;

            manifests.push(manifest);
        }
    }

    Ok(manifests)
}

/// Find a source manifest by ID (case-insensitive, partial match).
pub fn _find_by_id(id: &str) -> Result<Manifest> {
    let manifests = load_all()?;
    let id_lower = id.to_lowercase();

    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.source.id.to_lowercase() == id_lower)
    {
        return Ok(manifest.clone());
    }

    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.source.id.to_lowercase().contains(&id_lower))
    {
        return Ok(manifest.clone());
    }

    Err(anyhow!("Source '{}' not found", id))
}

/// List all available source IDs.
pub fn list_sources() -> Result<Vec<String>> {
    let manifests = load_all()?;
    Ok(manifests.into_iter().map(|m| m.source.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_all_manifests() {
        let manifests = load_all().expect("Failed to load manifests");
        assert!(!manifests.is_empty(), "No manifests found");

        for manifest in &manifests {
            assert!(!manifest.source.id.is_empty());
            assert!(!manifest.source.name.is_empty());
            assert!(!manifest.discovery.steps.is_empty());
            assert!(!manifest.content.steps.is_empty());
        }
    }

    #[test]
    fn test_list_sources() {
        let sources = list_sources().expect("Failed to list sources");
        assert!(sources.contains(&"caracol".to_string()));
        assert!(sources.contains(&"canal_rcn".to_string()));
        assert!(sources.contains(&"canal_1".to_string()));
    }

    #[test]
    fn test_find_by_id() {
        let manifest = _find_by_id("caracol").expect("Failed to find caracol");
        assert_eq!(manifest.source.id, "caracol");
        assert_eq!(manifest.source.name, "Caracol TV");
    }
}
