use anyhow::{Result, anyhow};
use chrome_browser::ChromeBrowserTab;

use crate::engine::{
    InterpolationContext, PhaseOutput, Source,
    executor::execute_steps,
    manifest::{DiscoveryOutputs, DiscoveryPhase},
};

use super::types::Channel;

/// Result of running the discovery phase.
pub struct DiscoveryResult {
    pub channels: Vec<Channel>,
    pub expires_at: Option<u64>,
}

/// Execute the discovery phase, returning discovered channels.
///
/// Supports two modes:
/// 1. Multi-channel: An array extractor produces multiple items, each becoming a channel.
/// 2. Single-channel: Scalar extractors produce one channel via output interpolation.
///
/// Domain filtering happens here: items without an `id` field are skipped.
pub async fn execute_discovery(
    phase: &DiscoveryPhase,
    tab: &ChromeBrowserTab,
    source: &Source,
    proxy: Option<&str>,
) -> Result<DiscoveryResult> {
    let context = InterpolationContext::new();
    let output = execute_steps(&phase.steps, tab, context, proxy).await?;

    let channels = if let Some((_key, items)) = first_array(&output) {
        // Multi-channel mode: build channels from extracted array
        items
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(|v| v.clone())?;
                let name = item.get("name").and_then(|v| v.clone());
                let image = item.get("image").and_then(|v| v.clone());
                Some(Channel {
                    id,
                    source_id: source.id.clone(),
                    name,
                    image,
                    category: None,
                    description: None,
                })
            })
            .collect()
    } else {
        // Single-channel mode: interpolate outputs from context
        let id = output.context.interpolate(&phase.outputs.id)?;
        let name = phase
            .outputs
            .name
            .as_ref()
            .map(|t| output.context.interpolate(t))
            .transpose()?;
        let image = phase
            .outputs
            .image
            .as_ref()
            .map(|t| output.context.interpolate(t))
            .transpose()?;

        vec![Channel {
            id,
            source_id: source.id.clone(),
            name,
            image,
            category: None,
            description: None,
        }]
    };

    if channels.is_empty() {
        return Err(anyhow!("Discovery found no channels"));
    }

    println!(
        "[discovery] Found {} channel(s) from '{}'",
        channels.len(),
        source.id
    );

    let expires_at = resolve_expiration(&phase.outputs, &output)?;

    Ok(DiscoveryResult {
        channels,
        expires_at,
    })
}

/// Get the first array from a phase output (there's typically only one).
fn first_array(output: &PhaseOutput) -> Option<(&String, &crate::engine::ExtractedArray)> {
    output.arrays.iter().next()
}

/// Resolve expiration from discovery outputs.
fn resolve_expiration(outputs: &DiscoveryOutputs, output: &PhaseOutput) -> Result<Option<u64>> {
    if let Some(expires_at_template) = &outputs.expires_at
        && let Ok(expires_str) = output.context.interpolate(expires_at_template)
        && let Ok(expires) = expires_str.parse::<u64>()
    {
        return Ok(Some(expires));
    }

    if let Some(expires_in) = outputs.expires_in {
        return Ok(Some(crate::util::time::now() + expires_in));
    }

    Ok(None)
}
