use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::RwLock;

use crate::registry::ChannelId;

/**
    A cached image with its data and content type.
*/
#[derive(Clone)]
pub struct CachedImage {
    pub data: Arc<Vec<u8>>,
    pub content_type: String,
}

/**
    In-memory cache for channel images, fetched on-demand.
*/
pub struct ImageCache {
    cache: RwLock<HashMap<ChannelId, CachedImage>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /**
        Get an image from cache, or fetch it from the URL if not cached.
    */
    pub async fn get_or_fetch(
        &self,
        id: &ChannelId,
        url: &str,
        proxy: Option<&str>,
    ) -> Result<CachedImage> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(id) {
                return Ok(cached.clone());
            }
        }

        // Fetch the image
        let image = fetch_image(url, proxy).await?;

        // Store in cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(id.clone(), image.clone());
        }

        Ok(image)
    }

    /**
        Invalidate cached image for a channel (e.g., when discovery refreshes).
    */
    #[allow(dead_code)]
    pub async fn invalidate(&self, id: &ChannelId) {
        let mut cache = self.cache.write().await;
        cache.remove(id);
    }

    /**
        Invalidate all cached images for a source.
    */
    #[allow(dead_code)]
    pub async fn invalidate_source(&self, source: &str) {
        let mut cache = self.cache.write().await;
        cache.retain(|id, _| id.source != source);
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/**
    Fetch an image from a URL, optionally using a proxy.
*/
async fn fetch_image(url: &str, proxy: Option<&str>) -> Result<CachedImage> {
    let client = if let Some(proxy_url) = proxy {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| anyhow!("Invalid proxy URL '{}': {}", proxy_url, e))?;
        reqwest::Client::builder()
            .proxy(proxy)
            .build()
            .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?
    } else {
        reqwest::Client::new()
    };

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch image: {}", e))?;

    if !response.status().is_success() {
        return Err(anyhow!("Failed to fetch image: HTTP {}", response.status()));
    }

    // Get content type from response headers, or detect from bytes
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data = response
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read image data: {}", e))?;

    // Determine content type from headers or magic bytes
    let content_type = content_type.unwrap_or_else(|| detect_content_type(&data));

    Ok(CachedImage {
        data: Arc::new(data.to_vec()),
        content_type,
    })
}

/**
    Detect image content type from magic bytes.
*/
fn detect_content_type(data: &[u8]) -> String {
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png".to_string()
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg".to_string()
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "image/webp".to_string()
    } else if data.starts_with(b"<svg") || data.starts_with(b"<?xml") {
        "image/svg+xml".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}
