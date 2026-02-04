use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::sync::{Mutex, RwLock, oneshot, watch};

use crate::cdrm;
use crate::manifest::StreamInfo;
use crate::proxy;
use crate::registry::ChannelId;
use crate::segments::SegmentManager;

/**
    State of a pipeline
*/
#[derive(Debug)]
enum PipelineState {
    Idle,
    Starting,
    Running { stop_tx: oneshot::Sender<()> },
    Stopping,
}

/**
    Check if an error message indicates an auth/credential issue
*/
fn is_auth_error(error_msg: &str) -> bool {
    let error_lower = error_msg.to_lowercase();
    error_lower.contains("401")
        || error_lower.contains("403")
        || error_lower.contains("410")
        || error_lower.contains("unauthorized")
        || error_lower.contains("forbidden")
        || error_lower.contains("expired")
        || error_lower.contains("invalid token")
        || error_lower.contains("access denied")
}

/**
    Manages the lifecycle of a single channel's remux pipeline.
*/
pub struct ChannelPipeline {
    channel_id: ChannelId,
    state: Arc<Mutex<PipelineState>>,
    stream_info: Arc<RwLock<StreamInfo>>,
    segment_manager: Arc<SegmentManager>,
    segment_duration: Duration,
    output_dir: PathBuf,
    startup_timeout: Duration,
    last_activity: AtomicU64,
    /// Set to true if pipeline failed due to auth error (needs refresh)
    needs_refresh: Arc<AtomicBool>,
}

impl ChannelPipeline {
    pub fn new(
        channel_id: ChannelId,
        stream_info: StreamInfo,
        segment_manager: Arc<SegmentManager>,
        segment_duration: Duration,
        output_dir: PathBuf,
        startup_timeout: Duration,
    ) -> Self {
        Self {
            channel_id,
            state: Arc::new(Mutex::new(PipelineState::Idle)),
            stream_info: Arc::new(RwLock::new(stream_info)),
            segment_manager,
            needs_refresh: Arc::new(AtomicBool::new(false)),
            segment_duration,
            output_dir,
            startup_timeout,
            last_activity: AtomicU64::new(0),
        }
    }

    pub fn output_dir(&self) -> &std::path::Path {
        &self.output_dir
    }

    pub async fn is_running(&self) -> bool {
        matches!(*self.state.lock().await, PipelineState::Running { .. })
    }

    pub fn record_activity(&self) {
        self.last_activity
            .store(crate::time::now(), Ordering::Relaxed);
    }

    pub fn seconds_since_activity(&self) -> u64 {
        let last = self.last_activity.load(Ordering::Relaxed);
        if last == 0 {
            return 0;
        }
        crate::time::now().saturating_sub(last)
    }

    /**
        Update the stream info (e.g., after refresh)
    */
    pub async fn update_stream_info(&self, info: StreamInfo) {
        *self.stream_info.write().await = info;
        // Clear refresh flag since we have new credentials
        self.needs_refresh.store(false, Ordering::Relaxed);
    }

    /**
        Check if pipeline needs a credential refresh (failed due to auth error)
    */
    pub fn needs_refresh(&self) -> bool {
        self.needs_refresh.load(Ordering::Relaxed)
    }

    /**
        Clear the refresh flag
    */
    #[allow(dead_code)]
    pub fn clear_refresh_flag(&self) {
        self.needs_refresh.store(false, Ordering::Relaxed);
    }

    /**
        Ensure the pipeline is running
    */
    pub async fn ensure_running(&self) -> Result<()> {
        {
            let state = self.state.lock().await;
            match *state {
                PipelineState::Running { .. } => {
                    self.record_activity();
                    return Ok(());
                }
                PipelineState::Starting => {
                    return Ok(());
                }
                PipelineState::Stopping => {
                    return Err(anyhow!("Pipeline is stopping, try again later"));
                }
                PipelineState::Idle => {}
            }
        }
        self.start().await
    }

    async fn start(&self) -> Result<()> {
        {
            let mut state = self.state.lock().await;
            if !matches!(*state, PipelineState::Idle) {
                return Ok(());
            }
            *state = PipelineState::Starting;
        }

        let stream_info = self.stream_info.read().await.clone();
        self.segment_manager.clear();
        self.record_activity();

        let (stop_tx, stop_rx) = oneshot::channel();

        let mpd_url = stream_info.manifest_url.clone();
        let license_url = stream_info.license_url.clone();
        let output_dir = self.output_dir.clone();
        let segment_duration = self.segment_duration;
        let segment_manager = Arc::clone(&self.segment_manager);
        let state = Arc::clone(&self.state);
        let channel_id = self.channel_id.to_string();

        // Clone the Arc to needs_refresh so we can set it from the spawned task
        let needs_refresh = Arc::clone(&self.needs_refresh);

        tokio::spawn(async move {
            let reset_state = |set_needs_refresh: bool| {
                let state = Arc::clone(&state);
                let needs_refresh = Arc::clone(&needs_refresh);
                async move {
                    let mut state_guard = state.lock().await;
                    if matches!(*state_guard, PipelineState::Running { .. }) {
                        *state_guard = PipelineState::Idle;
                    }
                    if set_needs_refresh {
                        needs_refresh.store(true, Ordering::Relaxed);
                    }
                }
            };

            // Fetch decryption keys if needed
            let decryption_keys: Vec<String> = if let Some(ref lic_url) = license_url {
                match cdrm::get_decryption_keys(&mpd_url, lic_url).await {
                    Ok(keys) => {
                        println!(
                            "[pipeline:{}] Got {} decryption key(s)",
                            channel_id,
                            keys.len()
                        );
                        keys
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        eprintln!(
                            "[pipeline:{}] Failed to get decryption keys: {}",
                            channel_id, error_str
                        );
                        let is_auth = is_auth_error(&error_str);
                        reset_state(is_auth).await;
                        return;
                    }
                }
            } else {
                Vec::new()
            };

            let (shutdown_tx, shutdown_rx) = watch::channel(false);

            let shutdown_tx_clone = shutdown_tx.clone();
            tokio::spawn(async move {
                let _ = stop_rx.await;
                let _ = shutdown_tx_clone.send(true);
            });

            println!("[pipeline:{}] Starting remux pipeline", channel_id);
            let channel_id_clone = channel_id.clone();
            let result = tokio::task::spawn_blocking(move || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(proxy::run_remux_pipeline(
                    &mpd_url,
                    &[],
                    &decryption_keys,
                    &output_dir,
                    segment_duration,
                    segment_manager,
                    shutdown_rx,
                ))
            })
            .await;

            let is_auth = match &result {
                Ok(Ok(())) => {
                    println!(
                        "[pipeline:{}] Pipeline completed normally",
                        channel_id_clone
                    );
                    false
                }
                Ok(Err(e)) => {
                    let error_str = e.to_string();
                    let is_auth = is_auth_error(&error_str);
                    if is_auth {
                        eprintln!(
                            "[pipeline:{}] Pipeline auth error (needs refresh): {}",
                            channel_id_clone, error_str
                        );
                    } else {
                        eprintln!(
                            "[pipeline:{}] Pipeline error: {}",
                            channel_id_clone, error_str
                        );
                    }
                    is_auth
                }
                Err(e) => {
                    eprintln!(
                        "[pipeline:{}] Pipeline task panicked: {}",
                        channel_id_clone, e
                    );
                    false
                }
            };

            reset_state(is_auth).await;
        });

        {
            let mut state = self.state.lock().await;
            *state = PipelineState::Running { stop_tx };
        }

        println!(
            "[pipeline:{}] Pipeline started",
            self.channel_id.to_string()
        );
        Ok(())
    }

    pub async fn stop(&self) {
        let stop_tx = {
            let mut state = self.state.lock().await;
            match std::mem::replace(&mut *state, PipelineState::Stopping) {
                PipelineState::Running { stop_tx } => Some(stop_tx),
                other => {
                    *state = other;
                    None
                }
            }
        };

        if let Some(tx) = stop_tx {
            println!(
                "[pipeline:{}] Stopping pipeline",
                self.channel_id.to_string()
            );
            let _ = tx.send(());
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        {
            let mut state = self.state.lock().await;
            *state = PipelineState::Idle;
        }
    }

    pub async fn wait_for_ready(&self) -> Result<()> {
        let deadline = Instant::now() + self.startup_timeout;

        loop {
            if self.segment_manager.segment_count() > 0 {
                return Ok(());
            }

            if Instant::now() > deadline {
                return Err(anyhow!("Timeout waiting for first segment"));
            }

            {
                let state = self.state.lock().await;
                if matches!(*state, PipelineState::Idle) {
                    return Err(anyhow!("Pipeline failed to start"));
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/**
    Configuration for pipeline creation
*/
#[derive(Clone)]
pub struct PipelineConfig {
    pub segment_count: usize,
    pub segment_duration: Duration,
    pub idle_timeout: Duration,
    pub startup_timeout: Duration,
    pub base_output_dir: PathBuf,
}

/**
    Manages multiple channel pipelines
*/
pub struct PipelineStore {
    pipelines: RwLock<HashMap<ChannelId, Arc<ChannelPipeline>>>,
    config: PipelineConfig,
    shutdown_rx: watch::Receiver<bool>,
}

impl PipelineStore {
    pub fn new(config: PipelineConfig, shutdown_rx: watch::Receiver<bool>) -> Self {
        Self {
            pipelines: RwLock::new(HashMap::new()),
            config,
            shutdown_rx,
        }
    }

    /**
        Get or create a pipeline for a channel
    */
    pub async fn get_or_create(
        &self,
        channel_id: &ChannelId,
        stream_info: &StreamInfo,
    ) -> Result<Arc<ChannelPipeline>> {
        // Check if pipeline exists
        {
            let pipelines = self.pipelines.read().await;
            if let Some(pipeline) = pipelines.get(channel_id) {
                return Ok(Arc::clone(pipeline));
            }
        }

        // Create new pipeline
        let mut pipelines = self.pipelines.write().await;

        // Double-check after acquiring write lock
        if let Some(pipeline) = pipelines.get(channel_id) {
            return Ok(Arc::clone(pipeline));
        }

        // Create channel-specific output directory
        let channel_dir = self
            .config
            .base_output_dir
            .join(format!("{}__{}", channel_id.source, channel_id.id));
        std::fs::create_dir_all(&channel_dir)?;

        let segment_manager = Arc::new(SegmentManager::new(
            channel_dir.clone(),
            self.config.segment_count,
        ));

        let pipeline = Arc::new(ChannelPipeline::new(
            channel_id.clone(),
            stream_info.clone(),
            segment_manager,
            self.config.segment_duration,
            channel_dir,
            self.config.startup_timeout,
        ));

        // Start idle check task for this pipeline
        let pipeline_clone = Arc::clone(&pipeline);
        let idle_timeout = self.config.idle_timeout;
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        if pipeline_clone.is_running().await {
                            let idle_secs = pipeline_clone.seconds_since_activity();
                            if idle_secs > idle_timeout.as_secs() {
                                println!(
                                    "[pipeline:{}] Idle for {}s, stopping",
                                    pipeline_clone.channel_id.to_string(),
                                    idle_secs
                                );
                                pipeline_clone.stop().await;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            pipeline_clone.stop().await;
                            return;
                        }
                    }
                }
            }
        });

        pipelines.insert(channel_id.clone(), Arc::clone(&pipeline));
        Ok(pipeline)
    }

    /**
        Get an existing pipeline (without creating)
    */
    pub async fn get(&self, channel_id: &ChannelId) -> Option<Arc<ChannelPipeline>> {
        self.pipelines.read().await.get(channel_id).cloned()
    }

    /**
        Stop all pipelines
    */
    pub async fn stop_all(&self) {
        let pipelines = self.pipelines.read().await;
        for pipeline in pipelines.values() {
            pipeline.stop().await;
        }
    }
}
