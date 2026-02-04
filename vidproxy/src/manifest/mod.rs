use anyhow::{Result, anyhow};
use include_dir::{Dir, include_dir};

mod content;
mod discovery;
mod executor;
mod extractors;
mod interpolate;
mod metadata;
mod types;

pub use content::execute_content;
pub use discovery::execute_discovery;
pub use metadata::execute_metadata;
pub use types::{ChannelEntry, DiscoveredChannel, Manifest, Programme, StreamInfo, Transform};

/**
    Embedded channel manifests directory.
*/
static CHANNELS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/channels");

/**
    Load all available source manifests.
*/
pub fn load_all() -> Result<Vec<Manifest>> {
    let mut manifests = Vec::new();

    for file in CHANNELS_DIR.files() {
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

/**
    Find a source manifest by name (case-insensitive, partial match).
*/
#[allow(dead_code)]
pub fn find_by_id(id: &str) -> Result<Manifest> {
    let manifests = load_all()?;
    let id_lower = id.to_lowercase();

    // Try exact match on id first
    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.source.id.to_lowercase() == id_lower)
    {
        return Ok(manifest.clone());
    }

    // Try partial match on id
    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.source.id.to_lowercase().contains(&id_lower))
    {
        return Ok(manifest.clone());
    }

    // Try matching filename
    for file in CHANNELS_DIR.files() {
        let path = file.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && (stem.to_lowercase() == id_lower || stem.to_lowercase().contains(&id_lower))
        {
            let content = file
                .contents_utf8()
                .ok_or_else(|| anyhow!("Failed to read {:?} as UTF-8", path))?;

            let manifest: Manifest = serde_yaml::from_str(content)
                .map_err(|e| anyhow!("Failed to parse {:?}: {}", path, e))?;

            return Ok(manifest);
        }
    }

    Err(anyhow!("Source '{}' not found", id))
}

/**
    List all available source IDs.
*/
pub fn list_sources() -> Result<Vec<String>> {
    let manifests = load_all()?;
    Ok(manifests.into_iter().map(|m| m.source.id).collect())
}
