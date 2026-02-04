mod executor;
mod extractors;
mod interpolate;
mod types;

use anyhow::{Result, anyhow};
use include_dir::{Dir, include_dir};

pub use executor::execute;
pub use types::Manifest;

/// Embedded channel manifests directory.
static CHANNELS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/channels");

/// Load all available channel manifests.
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

/// Find a channel manifest by name (case-insensitive, partial match).
pub fn find_by_name(name: &str) -> Result<Manifest> {
    let manifests = load_all()?;
    let name_lower = name.to_lowercase();

    // Try exact match first
    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.channel.name.to_lowercase() == name_lower)
    {
        return Ok(manifest.clone());
    }

    // Try partial match
    if let Some(manifest) = manifests
        .iter()
        .find(|m| m.channel.name.to_lowercase().contains(&name_lower))
    {
        return Ok(manifest.clone());
    }

    // Try matching filename
    for file in CHANNELS_DIR.files() {
        let path = file.path();
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && (stem.to_lowercase() == name_lower || stem.to_lowercase().contains(&name_lower))
        {
            let content = file
                .contents_utf8()
                .ok_or_else(|| anyhow!("Failed to read {:?} as UTF-8", path))?;

            let manifest: Manifest = serde_yaml::from_str(content)
                .map_err(|e| anyhow!("Failed to parse {:?}: {}", path, e))?;

            return Ok(manifest);
        }
    }

    Err(anyhow!("Channel '{}' not found", name))
}

/// List all available channel names.
pub fn list_channels() -> Result<Vec<String>> {
    let manifests = load_all()?;
    Ok(manifests.into_iter().map(|m| m.channel.name).collect())
}
