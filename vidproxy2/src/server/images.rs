use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::sync::RwLock;

use crate::channel::ChannelId;

/// A cached image with its data and content type.
#[derive(Clone)]
pub struct CachedImage {
    pub data: Arc<Vec<u8>>,
    pub content_type: String,
}

/// In-memory cache for channel images and proxied EPG images.
pub struct ImageCache {
    channel_cache: RwLock<HashMap<ChannelId, CachedImage>>,
    proxy_cache: RwLock<HashMap<String, (String, Option<CachedImage>)>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            channel_cache: RwLock::new(HashMap::new()),
            proxy_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get a channel image from cache, or fetch from URL if not cached.
    pub async fn get_or_fetch(
        &self,
        id: &ChannelId,
        url: &str,
        proxy: Option<&str>,
    ) -> Result<CachedImage> {
        {
            let cache = self.channel_cache.read().await;
            if let Some(cached) = cache.get(id) {
                return Ok(cached.clone());
            }
        }

        let image = fetch_image(url, proxy).await?;

        {
            let mut cache = self.channel_cache.write().await;
            cache.insert(id.clone(), image.clone());
        }

        Ok(image)
    }

    /// Register a URL for proxying and return its hash ID.
    pub async fn register_proxy_url(&self, url: &str) -> String {
        let id = hash_url(url);
        let mut cache = self.proxy_cache.write().await;
        cache.entry(id.clone()).or_insert((url.to_string(), None));
        id
    }

    /// Get a proxied image by its hash ID, fetching if not cached.
    pub async fn get_by_id(&self, id: &str) -> Result<CachedImage> {
        let url = {
            let cache = self.proxy_cache.read().await;
            let (url, cached) = cache.get(id).ok_or_else(|| anyhow!("Unknown image ID"))?;
            if let Some(img) = cached {
                return Ok(img.clone());
            }
            url.clone()
        };

        let image = fetch_image(&url, None).await?;

        {
            let mut cache = self.proxy_cache.write().await;
            if let Some((_, cached)) = cache.get_mut(id) {
                *cached = Some(image.clone());
            }
        }

        Ok(image)
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_url(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

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
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch image: {}", e))?;

    if !response.status().is_success() {
        return Err(anyhow!("Failed to fetch image: HTTP {}", response.status()));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data = response
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read image data: {}", e))?;

    let content_type = content_type.unwrap_or_else(|| detect_content_type(&data));

    Ok(CachedImage {
        data: Arc::new(data.to_vec()),
        content_type,
    })
}

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
