use anyhow::Result;
use chrome_browser::ChromeBrowserTab;

use super::executor::execute_steps;
use super::interpolate::InterpolationContext;
use super::types::{ContentPhase, DiscoveredChannel, StreamInfo};

/**
    Execute the content phase for a single channel, returning stream info.
*/
pub async fn execute_content(
    phase: &ContentPhase,
    tab: &ChromeBrowserTab,
    channel: &DiscoveredChannel,
    proxy: Option<&str>,
) -> Result<StreamInfo> {
    // Build initial context with channel fields
    let mut context = InterpolationContext::new();
    context.set("channel", "id", channel.id.clone());
    if let Some(name) = &channel.name {
        context.set("channel", "name", name.clone());
    }
    if let Some(image) = &channel.image {
        context.set("channel", "image", image.clone());
    }

    let (context, _) = execute_steps(&phase.steps, tab, context, proxy).await?;

    // Resolve outputs
    let manifest_url = context.interpolate(&phase.outputs.manifest_url)?;

    let license_url = phase
        .outputs
        .license_url
        .as_ref()
        .map(|t| context.interpolate(t))
        .transpose()?;

    let expires_at = resolve_expiration(&phase.outputs, &context)?;
    let headers = resolve_headers(&phase.outputs, &context)?;

    println!(
        "[content] Got stream info for channel '{}'",
        channel.name.as_deref().unwrap_or(&channel.id)
    );

    Ok(StreamInfo {
        manifest_url,
        license_url,
        expires_at,
        headers,
    })
}

/**
    Resolve expiration from outputs (either expires_at interpolation or expires_in static).
*/
fn resolve_expiration(
    outputs: &super::types::ContentOutputs,
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
        return Ok(Some(crate::time::now() + expires_in));
    }

    Ok(None)
}

/**
    Resolve optional headers from content outputs.
*/
fn resolve_headers(
    outputs: &super::types::ContentOutputs,
    context: &InterpolationContext,
) -> Result<Vec<(String, String)>> {
    let Some(headers) = &outputs.headers else {
        return Ok(Vec::new());
    };

    let mut resolved = Vec::with_capacity(headers.len());
    for (key, value) in headers {
        let value = context.interpolate(value)?;
        if !value.trim().is_empty() {
            resolved.push((key.clone(), value));
        }
    }

    Ok(resolved)
}
