use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::sync::RwLock;

use crate::engine::{browser::create_browser_for_phase, manifest::Manifest};

use super::content::execute_content;
use super::discovery::execute_discovery;
use super::metadata::execute_metadata;
use super::process::apply_process_phase;
use super::registry::ChannelRegistry;
use super::types::{ChannelContentState, ChannelEntry, ChannelId, StreamInfo};

const CONTENT_WAIT_TIMEOUT: Duration = Duration::from_secs(120);

/**
    Store for loaded manifests, keyed by source ID.
*/
pub struct ManifestStore {
    manifests: RwLock<HashMap<String, Manifest>>,
}

impl ManifestStore {
    pub fn new() -> Self {
        Self {
            manifests: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add(&self, manifest: Manifest) {
        let mut manifests = self.manifests.write().await;
        manifests.insert(manifest.source.id.clone(), manifest);
    }

    pub async fn get(&self, source_id: &str) -> Option<Manifest> {
        self.manifests.read().await.get(source_id).cloned()
    }

    pub async fn list(&self) -> Vec<Manifest> {
        self.manifests.read().await.values().cloned().collect()
    }
}

impl Default for ManifestStore {
    fn default() -> Self {
        Self::new()
    }
}

/**
    Content resolution orchestrator.

    Owns the manifest store and registry, and provides the high-level operations:
    - `run_initial_discovery()` — startup discovery for a single source
    - `refresh_discovery_if_needed()` — re-run discovery when expired
    - `ensure_stream_info()` — on-demand content resolution with concurrent coalescing
*/
pub struct Resolver {
    pub registry: Arc<ChannelRegistry>,
    pub manifest_store: Arc<ManifestStore>,
}

impl Resolver {
    pub fn new(registry: Arc<ChannelRegistry>, manifest_store: Arc<ManifestStore>) -> Self {
        Self {
            registry,
            manifest_store,
        }
    }

    /**
        Run initial discovery for a source (no content phase — content is on-demand).

        Creates a browser, runs discovery + optional metadata, registers channels,
        then closes the browser.
    */
    pub async fn run_initial_discovery(&self, manifest: &Manifest) -> Result<()> {
        let source = &manifest.source;
        println!(
            "[resolver] Starting discovery for: {} ({})",
            source.name, source.id
        );

        self.registry.mark_source_loading(&source.id);

        match self.run_discovery_inner(manifest).await {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("[resolver] Discovery failed for '{}': {}", source.id, e);
                self.registry.mark_source_failed(&source.id, e.to_string());
                Err(e)
            }
        }
    }

    async fn run_discovery_inner(&self, manifest: &Manifest) -> Result<()> {
        let source = &manifest.source;

        // Create browser for discovery
        let (browser, resolved_config) =
            create_browser_for_phase(&manifest.discovery.browser, source).await?;
        let tab = browser
            .get_tab(0)
            .await
            .ok_or_else(|| anyhow!("No browser tab available"))?;

        let proxy = resolved_config.proxy.as_deref();

        // Run discovery phase
        let discovery_result = execute_discovery(&manifest.discovery, &tab, source, proxy).await?;

        // Close discovery browser
        let _ = tab.navigate("about:blank").await;
        let _ = browser.close().await;

        let mut channels = discovery_result.channels;
        println!("[resolver] Discovery found {} channels", channels.len());

        // Apply process phase (filter + transforms)
        if let Some(ref process) = manifest.process {
            channels = apply_process_phase(channels, process);
        }

        // Run metadata phase if present (creates its own browser)
        let mut channel_programmes = HashMap::new();
        if let Some(ref metadata_phase) = manifest.metadata {
            println!("[resolver] Running metadata phase...");

            let (meta_browser, meta_config) =
                create_browser_for_phase(&metadata_phase.browser, source).await?;
            let meta_tab = meta_browser
                .get_tab(0)
                .await
                .ok_or_else(|| anyhow!("No browser tab available for metadata"))?;

            let meta_proxy = meta_config.proxy.as_deref();
            match execute_metadata(metadata_phase, &meta_tab, meta_proxy).await {
                Ok(result) => {
                    channel_programmes = result.programmes_by_channel;
                    self.registry
                        .set_metadata_expiration(&source.id, result.expires_at);
                }
                Err(e) => {
                    eprintln!("[resolver] Metadata phase failed: {}", e);
                    // Not fatal — continue without metadata
                }
            }

            let _ = meta_tab.navigate("about:blank").await;
            let _ = meta_browser.close().await;
        }

        // Build channel entries and register
        let entries: Vec<ChannelEntry> = channels
            .into_iter()
            .map(|channel| {
                let programmes = channel_programmes.remove(&channel.id).unwrap_or_default();
                ChannelEntry {
                    channel,
                    stream_info: None,
                    programmes,
                    last_error: None,
                }
            })
            .collect();

        println!(
            "[resolver] Registering {} channels for '{}'",
            entries.len(),
            source.id
        );

        self.registry
            .register_source(&source.id, entries, discovery_result.expires_at);

        Ok(())
    }

    /**
        Re-run discovery for a source if its results have expired.
    */
    pub async fn refresh_discovery_if_needed(&self, source_id: &str) -> Result<bool> {
        if !self.registry.is_discovery_expired(source_id) {
            return Ok(false);
        }

        let manifest = self
            .manifest_store
            .get(source_id)
            .await
            .ok_or_else(|| anyhow!("No manifest for source '{}'", source_id))?;

        println!(
            "[resolver] Discovery expired for '{}', refreshing...",
            source_id
        );
        self.run_initial_discovery(&manifest).await?;

        Ok(true)
    }

    /**
        Re-run metadata for a source if its EPG data has expired.

        Unlike discovery refresh, this only re-runs the metadata phase and updates
        programmes in-place without touching discovery or content state.
    */
    pub async fn refresh_metadata_if_needed(&self, source_id: &str) -> Result<bool> {
        if !self.registry.is_metadata_expired(source_id) {
            return Ok(false);
        }

        let manifest = self
            .manifest_store
            .get(source_id)
            .await
            .ok_or_else(|| anyhow!("No manifest for source '{}'", source_id))?;

        let Some(ref metadata_phase) = manifest.metadata else {
            return Ok(false);
        };

        println!(
            "[resolver] Metadata expired for '{}', refreshing...",
            source_id
        );

        let (browser, config) =
            create_browser_for_phase(&metadata_phase.browser, &manifest.source).await?;
        let tab = browser
            .get_tab(0)
            .await
            .ok_or_else(|| anyhow!("No browser tab available for metadata refresh"))?;

        let proxy = config.proxy.as_deref();
        match execute_metadata(metadata_phase, &tab, proxy).await {
            Ok(result) => {
                self.registry
                    .update_programmes(source_id, result.programmes_by_channel);
                self.registry
                    .set_metadata_expiration(source_id, result.expires_at);

                let total: usize = self
                    .registry
                    .list_by_source(source_id)
                    .iter()
                    .map(|e| e.programmes.len())
                    .sum();
                println!(
                    "[resolver] Metadata refreshed for '{}': {} total programmes",
                    source_id, total
                );
            }
            Err(e) => {
                eprintln!(
                    "[resolver] Metadata refresh failed for '{}': {}",
                    source_id, e
                );
                // Not fatal — keep existing stale data rather than wiping it
            }
        }

        let _ = tab.navigate("about:blank").await;
        let _ = browser.close().await;

        Ok(true)
    }

    /**
        Ensure stream info is available for a channel, resolving on-demand if needed.

        Handles concurrent coalescing: if another caller is already resolving,
        this waits for that resolution instead of starting a duplicate.
    */
    pub async fn ensure_stream_info(&self, id: &ChannelId) -> Result<StreamInfo> {
        // Check if channel is currently live before doing anything
        if let Some(entry) = self.registry.get(id)
            && !entry.is_live_now()
        {
            let name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);
            return Err(anyhow!(
                "Channel '{}' is not currently available (copyrighted content airing)",
                name
            ));
        }

        // Check if we already have valid (non-expired) stream info
        if let Some(entry) = self.registry.get(id)
            && let Some(ref info) = entry.stream_info
        {
            if !self.registry.is_stream_expired(id) {
                return Ok(info.clone());
            }
            // Expired — reset state so we can re-resolve
            self.registry.reset_channel_content_state(id);
        }

        // Atomic check-and-mark: try to become the resolver
        if self.registry.try_mark_resolving(id) {
            // We won the race — do the actual resolution
            match self.resolve_content(id).await {
                Ok(info) => {
                    self.registry.update_stream_info(id, info.clone());
                    self.registry.mark_channel_resolved(id);
                    Ok(info)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    self.registry.set_error(id, err_str.clone());
                    self.registry.mark_channel_failed(id, &err_str);
                    Err(e)
                }
            }
        } else {
            // Another caller is resolving — wait for them
            self.wait_for_resolution(id).await
        }
    }

    /**
        Actually resolve content for a channel (creates browser, runs content phase).
    */
    async fn resolve_content(&self, id: &ChannelId) -> Result<StreamInfo> {
        let entry = self
            .registry
            .get(id)
            .ok_or_else(|| anyhow!("Channel {} not found", id.to_string()))?;

        // Check if channel is currently available
        if !entry.is_live_now() {
            let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);
            return Err(anyhow!(
                "Channel '{}' is not currently available",
                channel_name
            ));
        }

        let manifest = self
            .manifest_store
            .get(&id.source)
            .await
            .ok_or_else(|| anyhow!("No manifest for source '{}'", id.source))?;

        let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);
        println!("[resolver] Resolving content for '{}'...", channel_name);

        let (browser, resolved_config) =
            create_browser_for_phase(&manifest.content.browser, &manifest.source).await?;
        let tab = browser
            .get_tab(0)
            .await
            .ok_or_else(|| anyhow!("No browser tab available for content"))?;

        let proxy = resolved_config.proxy.as_deref();
        let stream_info = execute_content(&manifest.content, &tab, &entry.channel, proxy).await?;

        println!(
            "[resolver] Content resolved for '{}': {}",
            channel_name, stream_info.manifest_url
        );

        let _ = tab.navigate("about:blank").await;
        let _ = browser.close().await;

        Ok(stream_info)
    }

    /**
        Wait for another caller's resolution to complete.
    */
    async fn wait_for_resolution(&self, id: &ChannelId) -> Result<StreamInfo> {
        println!(
            "[resolver] Waiting for content resolution of {}...",
            id.to_string()
        );

        match self
            .registry
            .wait_for_channel_content(id, CONTENT_WAIT_TIMEOUT)
            .await
        {
            Some(ChannelContentState::Resolved) => {
                let entry = self.registry.get(id).ok_or_else(|| {
                    anyhow!("Channel {} not found after resolution", id.to_string())
                })?;
                entry.stream_info.ok_or_else(|| {
                    anyhow!(
                        "Content resolved but stream_info still None for {}",
                        id.to_string()
                    )
                })
            }
            Some(ChannelContentState::Failed(err)) => Err(anyhow!(
                "Content resolution failed for {}: {}",
                id.to_string(),
                err
            )),
            _ => Err(anyhow!(
                "Timeout waiting for content resolution of {}",
                id.to_string()
            )),
        }
    }
}
