use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::credentials::{CredentialsReceiver, StreamCredentials};
use crate::proxy;
use crate::segments::SegmentManager;

/**
    Result of a pipeline run.
*/
#[derive(Debug)]
pub enum PipelineResult {
    /// Pipeline stopped due to shutdown signal
    Shutdown,
    /// Pipeline stopped due to source error (credentials may be expired)
    SourceError(String),
    /// Pipeline stopped due to sink error (may retry with same credentials)
    SinkError(String),
}

/**
    Refresh signal sender type.
*/
pub type RefreshSender = watch::Sender<bool>;
/**
    Refresh signal receiver type.
*/
pub type RefreshReceiver = watch::Receiver<bool>;

/**
    Create a refresh signal channel.
*/
pub fn refresh_channel() -> (RefreshSender, RefreshReceiver) {
    watch::channel(false)
}

/**
    Coordinator that orchestrates the sniffer and remux pipeline.
*/
pub struct Coordinator {
    credentials_rx: CredentialsReceiver,
    refresh_tx: RefreshSender,
    shutdown_rx: watch::Receiver<bool>,
    segment_manager: Arc<SegmentManager>,
    output_dir: std::path::PathBuf,
    segment_duration: Duration,
}

impl Coordinator {
    pub fn new(
        credentials_rx: CredentialsReceiver,
        refresh_tx: RefreshSender,
        shutdown_rx: watch::Receiver<bool>,
        segment_manager: Arc<SegmentManager>,
        output_dir: std::path::PathBuf,
        segment_duration: Duration,
    ) -> Self {
        Self {
            credentials_rx,
            refresh_tx,
            shutdown_rx,
            segment_manager,
            output_dir,
            segment_duration,
        }
    }

    /**
        Run the coordinator loop.
    */
    pub async fn run(&mut self) -> anyhow::Result<()> {
        println!("[coordinator] Starting, waiting for credentials...");

        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                println!("[coordinator] Shutdown requested");
                break;
            }

            // Wait for credentials
            let credentials = match self.wait_for_credentials().await {
                Some(creds) => creds,
                None => {
                    println!("[coordinator] Shutdown during credential wait");
                    break;
                }
            };

            println!(
                "[coordinator] Got credentials, starting pipeline for: {}",
                &credentials.mpd_url[..credentials.mpd_url.len().min(60)]
            );

            // Run the pipeline
            let result = self.run_pipeline(&credentials).await;

            match result {
                PipelineResult::Shutdown => {
                    println!("[coordinator] Pipeline shutdown");
                    break;
                }
                PipelineResult::SourceError(e) => {
                    println!("[coordinator] Source error: {}, requesting refresh...", e);
                    let _ = self.refresh_tx.send(true);
                    // Reset refresh signal for next use
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = self.refresh_tx.send(false);
                }
                PipelineResult::SinkError(e) => {
                    println!("[coordinator] Sink error: {}, retrying...", e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }

        Ok(())
    }

    /**
        Wait for credentials to become available.
    */
    async fn wait_for_credentials(&mut self) -> Option<StreamCredentials> {
        loop {
            // Check if we already have credentials
            if let Some(ref creds) = *self.credentials_rx.borrow() {
                return Some(creds.clone());
            }

            // Wait for credentials or shutdown
            tokio::select! {
                result = self.credentials_rx.changed() => {
                    if result.is_err() {
                        return None;
                    }
                    if let Some(ref creds) = *self.credentials_rx.borrow() {
                        return Some(creds.clone());
                    }
                }
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        return None;
                    }
                }
            }
        }
    }

    /**
        Run the remux pipeline with the given credentials.
    */
    async fn run_pipeline(&self, credentials: &StreamCredentials) -> PipelineResult {
        let input_url = credentials.mpd_url.clone();
        let decryption_key = Some(credentials.decryption_key.clone());
        let output_dir = self.output_dir.clone();
        let segment_duration = self.segment_duration;
        let segment_manager = Arc::clone(&self.segment_manager);
        let shutdown_rx = self.shutdown_rx.clone();

        // Run pipeline in blocking task
        let result = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(proxy::run_remux_pipeline(
                &input_url,
                &[], // headers not used
                decryption_key.as_deref(),
                &output_dir,
                segment_duration,
                segment_manager,
                shutdown_rx,
            ))
        })
        .await;

        match result {
            Ok(Ok(())) => {
                // Pipeline completed normally (source ended)
                PipelineResult::SourceError("Source ended".to_string())
            }
            Ok(Err(e)) => {
                let err_str = e.to_string();
                // Try to categorize the error
                if err_str.contains("403")
                    || err_str.contains("410")
                    || err_str.contains("connection")
                    || err_str.contains("timeout")
                {
                    PipelineResult::SourceError(err_str)
                } else {
                    PipelineResult::SinkError(err_str)
                }
            }
            Err(e) => {
                // Task panicked or was cancelled
                if e.is_cancelled() {
                    PipelineResult::Shutdown
                } else {
                    PipelineResult::SinkError(e.to_string())
                }
            }
        }
    }
}
