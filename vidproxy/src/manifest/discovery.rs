use anyhow::{Result, anyhow};
use chrome_browser::ChromeBrowserTab;

use super::executor::execute_steps;
use super::interpolate::InterpolationContext;
use super::types::{DiscoveredChannel, DiscoveryPhase};

/**
    Result of running the discovery phase.
*/
pub struct DiscoveryResult {
    /// Discovered channels
    pub channels: Vec<DiscoveredChannel>,
    /// Expiration timestamp (if extracted or specified)
    pub expires_at: Option<u64>,
}

/**
    Execute the discovery phase, returning a list of discovered channels.
*/
pub async fn execute_discovery(
    phase: &DiscoveryPhase,
    tab: &ChromeBrowserTab,
    source_id: &str,
) -> Result<DiscoveryResult> {
    let context = InterpolationContext::new();

    let (context, array_result) = execute_steps(&phase.steps, tab, context).await?;

    // We expect an array result from discovery
    let (_array_key, items) = array_result
        .ok_or_else(|| anyhow!("Discovery phase must have a jsonpath_array extractor"))?;

    // Build channels from the extracted array
    let mut channels = Vec::new();

    for item in items {
        // Get required id field
        let id = match item.get("id").and_then(|v| v.clone()) {
            Some(id) => id,
            None => continue, // Skip items without id (shouldn't happen, filtered in extractor)
        };

        // Get optional fields
        let name = item.get("name").and_then(|v| v.clone());
        let image = item.get("image").and_then(|v| v.clone());

        channels.push(DiscoveredChannel {
            id,
            name,
            image,
            category: None,
            description: None,
            source: source_id.to_string(),
        });
    }

    if channels.is_empty() {
        return Err(anyhow!("Discovery found no channels"));
    }

    println!(
        "[discovery] Found {} channels from '{}'",
        channels.len(),
        source_id
    );

    // Resolve expiration
    let expires_at = resolve_expiration(&phase.outputs, &context)?;

    Ok(DiscoveryResult {
        channels,
        expires_at,
    })
}

/**
    Resolve expiration from outputs (either expires_at interpolation or expires_in static).
*/
fn resolve_expiration(
    outputs: &super::types::DiscoveryOutputs,
    context: &InterpolationContext,
) -> Result<Option<u64>> {
    // Try expires_at first (interpolated)
    if let Some(expires_at_template) = &outputs.expires_at
        && let Ok(expires_str) = context.interpolate(expires_at_template)
        && let Ok(expires) = expires_str.parse::<u64>()
    {
        return Ok(Some(expires));
    }

    // Fall back to expires_in (static duration from now)
    if let Some(expires_in) = outputs.expires_in {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        return Ok(Some(now + expires_in));
    }

    Ok(None)
}
