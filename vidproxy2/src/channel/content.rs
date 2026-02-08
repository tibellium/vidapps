use anyhow::Result;
use chrome_browser::ChromeBrowserTab;

use crate::engine::{
    InterpolationContext,
    executor::execute_steps,
    manifest::{ContentOutputs, ContentPhase},
};

use super::types::{Channel, StreamInfo};

/// Execute the content phase for a single channel, returning stream info.
pub async fn execute_content(
    phase: &ContentPhase,
    tab: &ChromeBrowserTab,
    channel: &Channel,
    proxy: Option<&str>,
) -> Result<StreamInfo> {
    let mut context = InterpolationContext::new();
    context.set("channel", "id", channel.id.clone());
    if let Some(name) = &channel.name {
        context.set("channel", "name", name.clone());
    }
    if let Some(image) = &channel.image {
        context.set("channel", "image", image.clone());
    }

    let output = execute_steps(&phase.steps, tab, context, proxy).await?;

    let manifest_url = output.context.interpolate(&phase.outputs.manifest_url)?;
    let license_url = phase
        .outputs
        .license_url
        .as_ref()
        .map(|t| output.context.interpolate(t))
        .transpose()?;
    let expires_at = resolve_expiration(&phase.outputs, &output.context)?;
    let headers = resolve_headers(&phase.outputs, &output.context)?;

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

/// Resolve expiration from content outputs.
fn resolve_expiration(
    outputs: &ContentOutputs,
    context: &InterpolationContext,
) -> Result<Option<u64>> {
    if let Some(expires_at_template) = &outputs.expires_at
        && let Ok(expires_str) = context.interpolate(expires_at_template)
        && let Ok(expires) = expires_str.parse::<u64>()
    {
        return Ok(Some(expires));
    }

    if let Some(expires_in) = outputs.expires_in {
        return Ok(Some(crate::util::time::now() + expires_in));
    }

    Ok(None)
}

/// Resolve optional headers from content outputs.
fn resolve_headers(
    outputs: &ContentOutputs,
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
