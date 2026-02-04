use std::time::Duration;

use anyhow::Result;
use chrome_browser::{ChromeBrowser, ChromeLaunchOptions};
use tokio::sync::watch;

use crate::manifest::{self, Manifest};
use crate::stream_info::{StreamInfo, StreamInfoSender};

/**
    DRM sniffer that discovers stream info using Chrome browser automation.
*/
pub struct DrmSniffer {
    manifest: Manifest,
    headless: bool,
    stream_info_tx: StreamInfoSender,
}

impl DrmSniffer {
    pub fn new(manifest: Manifest, headless: bool, stream_info_tx: StreamInfoSender) -> Self {
        Self {
            manifest,
            headless,
            stream_info_tx,
        }
    }

    /**
        Run the sniffer loop. Discovers credentials and publishes them.
        Re-discovers when refresh is requested.
    */
    pub async fn run(
        &mut self,
        mut shutdown_rx: watch::Receiver<bool>,
        mut refresh_rx: watch::Receiver<bool>,
    ) -> Result<()> {
        loop {
            // Check for shutdown
            if *shutdown_rx.borrow() {
                println!("[sniffer] Shutdown requested");
                break;
            }

            // Attempt to discover stream info
            match self.discover_stream_info(&mut shutdown_rx).await {
                Ok(Some(info)) => {
                    println!("[sniffer] Stream info discovered successfully");
                    println!(
                        "[sniffer] MPD URL: {}...",
                        &info.mpd_url[..info.mpd_url.len().min(60)]
                    );
                    let _ = self.stream_info_tx.send(Some(info));

                    // Wait for refresh request or shutdown
                    loop {
                        tokio::select! {
                            _ = shutdown_rx.changed() => {
                                if *shutdown_rx.borrow() {
                                    println!("[sniffer] Shutdown requested");
                                    return Ok(());
                                }
                            }
                            _ = refresh_rx.changed() => {
                                if *refresh_rx.borrow() {
                                    println!("[sniffer] Refresh requested, re-discovering...");
                                    break;
                                }
                            }
                        }
                    }
                }
                Ok(None) => {
                    // Shutdown requested during discovery
                    println!("[sniffer] Shutdown during discovery");
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("[sniffer] Discovery failed: {}", e);
                    // Wait before retrying
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /**
        Discover stream info by executing the manifest.
        Returns None if shutdown was requested during discovery.
    */
    async fn discover_stream_info(
        &self,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<Option<StreamInfo>> {
        println!("[sniffer] Launching Chrome...");

        let mut options = ChromeLaunchOptions::default()
            .headless(self.headless)
            .devtools(false);

        if let Some(ref proxy) = self.manifest.channel.proxy {
            options = options.proxy_server(proxy);
        }

        let browser = ChromeBrowser::new(options).await?;

        // Execute the manifest with shutdown monitoring
        let outputs = tokio::select! {
            result = manifest::execute(&self.manifest, &browser) => {
                let _ = browser.close().await;
                result?
            }
            _ = shutdown_rx.changed() => {
                println!("[sniffer] Shutdown during discovery, closing browser...");
                let _ = browser.close().await;
                return Ok(None);
            }
        };

        if let Some(ts) = outputs.expires_at {
            println!("[sniffer] Stream expires at: {}", ts);
        }

        Ok(Some(StreamInfo {
            channel_name: outputs.channel_name,
            mpd_url: outputs.mpd_url,
            decryption_key: outputs.decryption_key,
            license_url: String::new(), // Not needed for now
            pssh: String::new(),        // Not needed for now
            thumbnail_url: outputs.thumbnail_url,
            expires_at: outputs.expires_at,
        }))
    }
}
