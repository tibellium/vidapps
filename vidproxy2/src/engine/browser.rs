use anyhow::Result;
use chrome_browser::{ChromeBrowser, ChromeLaunchOptions};

use super::manifest::{BrowserConfig, ResolvedBrowserConfig, Source};

/**
    Create a browser instance from resolved browser config.
*/
pub async fn create_browser(config: &ResolvedBrowserConfig) -> Result<ChromeBrowser> {
    let mut options = ChromeLaunchOptions::default()
        .headless(config.headless)
        .devtools(false)
        .enable_gpu(config.headless);

    if let Some(ref proxy) = config.proxy {
        options = options.proxy_server(proxy);
    }

    ChromeBrowser::new(options).await
}

/**
    Create a browser from a phase's browser config + source defaults.
*/
pub async fn create_browser_for_phase(
    browser_config: &BrowserConfig,
    source: &Source,
) -> Result<(ChromeBrowser, ResolvedBrowserConfig)> {
    let resolved = browser_config.resolve(source);
    let browser = create_browser(&resolved).await?;
    Ok((browser, resolved))
}
