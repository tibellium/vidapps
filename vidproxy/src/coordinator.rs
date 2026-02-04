use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::watch;

use crate::proxy;
use crate::segments::SegmentManager;
use crate::stream_info::{StreamInfo, StreamInfoReceiver};

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
    /// Pipeline stopped due to stream expiration (proactive refresh)
    Expired,
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
    stream_info_rx: StreamInfoReceiver,
    refresh_tx: RefreshSender,
    shutdown_rx: watch::Receiver<bool>,
    segment_manager: Arc<SegmentManager>,
    output_dir: std::path::PathBuf,
    segment_duration: Duration,
}

impl Coordinator {
    pub fn new(
        stream_info_rx: StreamInfoReceiver,
        refresh_tx: RefreshSender,
        shutdown_rx: watch::Receiver<bool>,
        segment_manager: Arc<SegmentManager>,
        output_dir: std::path::PathBuf,
        segment_duration: Duration,
    ) -> Self {
        Self {
            stream_info_rx,
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
        println!("[coordinator] Starting, waiting for stream info...");

        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                println!("[coordinator] Shutdown requested");
                break;
            }

            // Wait for stream info
            let stream_info = match self.wait_for_stream_info().await {
                Some(info) => info,
                None => {
                    println!("[coordinator] Shutdown during stream info wait");
                    break;
                }
            };

            println!(
                "[coordinator] Got stream info, starting pipeline for: {}",
                &stream_info.mpd_url[..stream_info.mpd_url.len().min(60)]
            );

            // Calculate refresh time if we have an expiration
            let refresh_at = stream_info.expires_at.map(|expires| {
                let now = Utc::now().timestamp() as u64;
                if expires > now {
                    // Refresh 60 seconds before expiration
                    let refresh_in = expires.saturating_sub(now).saturating_sub(60);
                    if refresh_in > 0 {
                        println!(
                            "[coordinator] Stream expires in {}s, will refresh in {}s",
                            expires - now,
                            refresh_in
                        );
                        Duration::from_secs(refresh_in)
                    } else {
                        // Already close to expiring, refresh soon
                        println!("[coordinator] Stream expiring soon, will refresh in 5s");
                        Duration::from_secs(5)
                    }
                } else {
                    println!("[coordinator] Stream already expired, refreshing immediately");
                    Duration::ZERO
                }
            });

            // Run the pipeline with optional expiration-based refresh
            let result = self.run_pipeline(&stream_info, refresh_at).await;

            match result {
                PipelineResult::Shutdown => {
                    println!("[coordinator] Pipeline shutdown");
                    break;
                }
                PipelineResult::Expired => {
                    println!("[coordinator] Stream expiring, requesting refresh...");
                    let _ = self.refresh_tx.send(true);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = self.refresh_tx.send(false);
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
        Wait for stream info to become available.
    */
    async fn wait_for_stream_info(&mut self) -> Option<StreamInfo> {
        loop {
            // Check if we already have stream info
            if let Some(ref info) = *self.stream_info_rx.borrow() {
                return Some(info.clone());
            }

            // Wait for stream info or shutdown
            tokio::select! {
                result = self.stream_info_rx.changed() => {
                    if result.is_err() {
                        return None;
                    }
                    if let Some(ref info) = *self.stream_info_rx.borrow() {
                        return Some(info.clone());
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
        Run the remux pipeline with the given stream info.
        If `refresh_after` is Some, the pipeline will be stopped for refresh after that duration.
    */
    async fn run_pipeline(
        &self,
        stream_info: &StreamInfo,
        refresh_after: Option<Duration>,
    ) -> PipelineResult {
        let input_url = stream_info.mpd_url.clone();
        let decryption_key = Some(stream_info.decryption_key.clone());
        let output_dir = self.output_dir.clone();
        let segment_duration = self.segment_duration;
        let segment_manager = Arc::clone(&self.segment_manager);
        let shutdown_rx = self.shutdown_rx.clone();

        // Run pipeline in blocking task
        let pipeline_handle = tokio::task::spawn_blocking(move || {
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
        });

        // Wait for pipeline completion or expiration timer
        let result = if let Some(refresh_duration) = refresh_after {
            tokio::select! {
                res = pipeline_handle => res,
                _ = tokio::time::sleep(refresh_duration) => {
                    return PipelineResult::Expired;
                }
            }
        } else {
            pipeline_handle.await
        };

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
