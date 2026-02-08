use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::channel::{
    content::execute_content, discovery::execute_discovery, metadata::execute_metadata,
    process::apply_process_phase,
};
use crate::engine::browser::create_browser_for_phase;

#[derive(Parser, Debug)]
pub struct TestSourceCommand {
    /// Source ID (or partial match)
    pub source: String,

    /// Skip the content phase (only run discovery + metadata)
    #[arg(long)]
    pub skip_content: bool,
}

impl TestSourceCommand {
    pub async fn run(self) -> Result<()> {
        let manifest = crate::engine::find_by_id(&self.source)?;
        let source = &manifest.source;
        let proxy = source.proxy.as_deref();

        println!("Testing source: {} ({})", source.name, source.id);
        println!();

        // --- Discovery phase ---
        println!("=== Discovery Phase ===");

        let (browser, _config) =
            create_browser_for_phase(&manifest.discovery.browser, source).await?;
        let tab = browser
            .get_tab(0)
            .await
            .context("No browser tab available")?;

        let discovery_result = execute_discovery(&manifest.discovery, &tab, source, proxy).await?;

        drop(tab);
        drop(browser);

        let mut channels = discovery_result.channels;

        println!("  Discovered {} channel(s)", channels.len());
        if let Some(expires_at) = discovery_result.expires_at {
            println!(
                "  Expires at: {}",
                expires_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
        }

        // --- Process phase ---
        if let Some(process) = &manifest.process {
            println!();
            println!("=== Process Phase ===");

            let before = channels.len();
            channels = apply_process_phase(channels, process);
            println!(
                "  {} -> {} channel(s) after filtering/transforms",
                before,
                channels.len()
            );
        }

        if channels.is_empty() {
            bail!("No channels after discovery + processing");
        }

        println!();
        println!("Channels:");
        for (i, ch) in channels.iter().enumerate() {
            println!(
                "  {:>3}. {} (id: {})",
                i + 1,
                ch.name.as_deref().unwrap_or("(unnamed)"),
                ch.id
            );
        }

        // --- Metadata phase ---
        if let Some(metadata_phase) = &manifest.metadata {
            println!();
            println!("=== Metadata Phase ===");

            let (browser, _config) =
                create_browser_for_phase(&metadata_phase.browser, source).await?;
            let tab = browser
                .get_tab(0)
                .await
                .context("No browser tab available")?;

            match execute_metadata(metadata_phase, &tab, proxy).await {
                Ok(result) => {
                    let total: usize = result.programmes_by_channel.values().map(|p| p.len()).sum();
                    println!(
                        "  {} programme(s) across {} channel(s)",
                        total,
                        result.programmes_by_channel.len()
                    );
                    if let Some(expires_at) = result.expires_at {
                        println!(
                            "  Expires at: {}",
                            expires_at.format("%Y-%m-%d %H:%M:%S UTC")
                        );
                    }

                    // Show a sample of programmes per channel
                    for (channel_id, programmes) in &result.programmes_by_channel {
                        println!(
                            "  Channel '{}': {} programme(s)",
                            channel_id,
                            programmes.len()
                        );
                        for prog in programmes.iter().take(3) {
                            let live_tag = match prog.is_live {
                                Some(true) => " [LIVE]",
                                Some(false) => " [OFFLINE]",
                                None => "",
                            };
                            println!(
                                "    - {} ({} -> {}){}",
                                prog.title,
                                prog.start_time.format("%H:%M"),
                                prog.end_time.format("%H:%M"),
                                live_tag
                            );
                        }
                        if programmes.len() > 3 {
                            println!("    ... and {} more", programmes.len() - 3);
                        }
                    }
                }
                Err(e) => {
                    println!("  FAILED: {}", e);
                }
            }

            drop(tab);
            drop(browser);
        }

        // --- Content phase ---
        let mut success_count = 0;
        let mut fail_count = 0;

        if self.skip_content {
            println!();
            println!("=== Content Phase (skipped) ===");
        } else {
            println!();
            println!("=== Content Phase ===");

            let (browser, _config) =
                create_browser_for_phase(&manifest.content.browser, source).await?;
            let tab = browser
                .get_tab(0)
                .await
                .context("No browser tab available")?;

            for ch in &channels {
                let label = ch.name.as_deref().unwrap_or(&ch.id);

                match execute_content(&manifest.content, &tab, ch, proxy).await {
                    Ok(info) => {
                        success_count += 1;
                        println!("  OK  {}", label);
                        println!("       manifest: {}", info.manifest_url);
                        if let Some(license) = &info.license_url {
                            println!("       license:  {}", license);
                        }
                        if let Some(expires_at) = info.expires_at {
                            println!(
                                "       expires:  {}",
                                expires_at.format("%Y-%m-%d %H:%M:%S UTC")
                            );
                        }
                        if !info.headers.is_empty() {
                            for (k, v) in &info.headers {
                                println!("       header:   {}: {}", k, v);
                            }
                        }
                    }
                    Err(e) => {
                        fail_count += 1;
                        println!("  FAIL  {}", label);
                        println!("        {}", e);
                    }
                }
            }

            drop(tab);
            drop(browser);
        }

        // --- Summary ---
        println!();
        println!("=== Summary ===");
        println!("  Source:    {} ({})", source.name, source.id);
        println!("  Channels: {}", channels.len());
        if !self.skip_content {
            println!("  Content:  {} ok, {} failed", success_count, fail_count);
        }

        if fail_count > 0 {
            bail!(
                "{} of {} channel(s) failed content resolution",
                fail_count,
                channels.len()
            );
        }

        Ok(())
    }
}
