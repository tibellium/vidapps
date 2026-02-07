use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrome_browser::ChromeBrowserTab;

use super::executor::execute_steps;
use super::interpolate::InterpolationContext;
use super::types::{MetadataPhase, Programme};

/**
    Result of running the metadata phase - EPG for all channels.
*/
#[allow(dead_code)]
pub struct MetadataResult {
    /// EPG programmes keyed by channel ID
    pub programmes_by_channel: HashMap<String, Vec<Programme>>,
    /// Expiration timestamp for metadata
    pub expires_at: Option<u64>,
}

/**
    Execute the metadata phase, returning EPG programmes for all channels.

    The metadata phase extracts nested array data where each top-level item
    represents a channel with its EPG programmes.
*/
pub async fn execute_metadata(
    phase: &MetadataPhase,
    tab: &ChromeBrowserTab,
    proxy: Option<&str>,
) -> Result<MetadataResult> {
    let context = InterpolationContext::new();

    let (_context, array_result) = execute_steps(&phase.steps, tab, context, proxy).await?;

    // We expect an array result from metadata extraction
    // Each item in the array represents a channel with nested programmes
    let (_array_key, items) = array_result
        .ok_or_else(|| anyhow!("Metadata phase must have a jsonpath_array, regex_array, xpath_array, or css_array extractor"))?;

    let mut programmes_by_channel: HashMap<String, Vec<Programme>> = HashMap::new();

    for item in items {
        // Get channel ID - required to map programmes to channels
        let channel_id = match item.get("channel_id").and_then(|v| v.clone()) {
            Some(id) => id,
            None => continue, // Skip items without channel_id
        };

        // Get required programme fields
        let title = match item.get("title").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue, // Skip items without title
        };
        let start_time = match item.get("start_time").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue, // Skip items without start_time
        };
        let end_time = match item.get("end_time").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue, // Skip items without end_time
        };

        // Get optional fields
        let description = item.get("description").and_then(|v| v.clone());
        let episode = item.get("episode").and_then(|v| v.clone());
        let season = item.get("season").and_then(|v| v.clone());
        let image = item.get("image").and_then(|v| v.clone());

        // Parse genres if present (comma-separated or single value)
        let genres = item
            .get("genres")
            .and_then(|v| v.clone())
            .map(|g| g.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let programme = Programme {
            title,
            description,
            start_time,
            end_time,
            episode,
            season,
            genres,
            image,
        };

        programmes_by_channel
            .entry(channel_id)
            .or_default()
            .push(programme);
    }

    let total_programmes: usize = programmes_by_channel.values().map(|p| p.len()).sum();
    println!(
        "[metadata] Got {} programmes across {} channels",
        total_programmes,
        programmes_by_channel.len()
    );

    // Resolve expiration
    let expires_at = resolve_expiration(&phase.outputs)?;

    Ok(MetadataResult {
        programmes_by_channel,
        expires_at,
    })
}

/**
    Resolve expiration from outputs (expires_in static duration).
*/
fn resolve_expiration(outputs: &super::types::MetadataOutputs) -> Result<Option<u64>> {
    if let Some(expires_in) = outputs.expires_in {
        return Ok(Some(crate::time::now() + expires_in));
    }
    Ok(None)
}
