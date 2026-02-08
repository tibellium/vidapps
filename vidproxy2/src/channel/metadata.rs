use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrome_browser::ChromeBrowserTab;

use crate::engine::{
    InterpolationContext,
    executor::execute_steps,
    manifest::{MetadataOutputs, MetadataPhase},
};

use super::types::Programme;

/// Result of running the metadata phase.
pub struct MetadataResult {
    pub programmes_by_channel: HashMap<String, Vec<Programme>>,
    pub expires_at: Option<u64>,
}

/// Execute the metadata phase, returning EPG programmes keyed by channel ID.
///
/// Domain filtering happens here: items without `channel_id` or `title` are skipped.
pub async fn execute_metadata(
    phase: &MetadataPhase,
    tab: &ChromeBrowserTab,
    proxy: Option<&str>,
) -> Result<MetadataResult> {
    let context = InterpolationContext::new();
    let output = execute_steps(&phase.steps, tab, context, proxy).await?;

    let (_key, items) = output
        .arrays
        .iter()
        .next()
        .ok_or_else(|| anyhow!("Metadata phase must have an array extractor"))?;

    let mut programmes_by_channel: HashMap<String, Vec<Programme>> = HashMap::new();

    for item in items {
        let channel_id = match item.get("channel_id").and_then(|v| v.clone()) {
            Some(id) => id,
            None => continue,
        };
        let title = match item.get("title").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue,
        };
        let start_time = match item.get("start_time").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue,
        };
        let end_time = match item.get("end_time").and_then(|v| v.clone()) {
            Some(t) => t,
            None => continue,
        };

        let description = item.get("description").and_then(|v| v.clone());
        let episode = item.get("episode").and_then(|v| v.clone());
        let season = item.get("season").and_then(|v| v.clone());
        let image = item.get("image").and_then(|v| v.clone());
        let genres = item
            .get("genres")
            .and_then(|v| v.clone())
            .map(|g| g.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        programmes_by_channel
            .entry(channel_id)
            .or_default()
            .push(Programme {
                title,
                description,
                start_time,
                end_time,
                episode,
                season,
                genres,
                image,
            });
    }

    let total: usize = programmes_by_channel.values().map(|p| p.len()).sum();
    println!(
        "[metadata] Got {} programmes across {} channels",
        total,
        programmes_by_channel.len()
    );

    let expires_at = resolve_expiration(&phase.outputs)?;

    Ok(MetadataResult {
        programmes_by_channel,
        expires_at,
    })
}

/// Resolve expiration from metadata outputs.
fn resolve_expiration(outputs: &MetadataOutputs) -> Result<Option<u64>> {
    if let Some(expires_in) = outputs.expires_in {
        return Ok(Some(crate::util::time::now() + expires_in));
    }
    Ok(None)
}
