use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrome_browser::{ChromeBrowserTab, NetworkRequestStream};
use regex::Regex;
use reqwest::{Client, Proxy};

use super::extractors::{ExtractedArray, extract, extract_array};
use super::interpolate::InterpolationContext;
use super::types::{AutomationAction, Extractor, ExtractorKind, Step, StepKind, WaitCondition};

/**
    User agent for HTTP fetch requests
*/
const FETCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/**
    Interpolate extractor fields that can contain templates.
*/
fn interpolate_extractor(
    extractor: &Extractor,
    context: &InterpolationContext,
) -> Result<Extractor> {
    let mut interpolated = extractor.clone();

    if let Some(path) = &extractor.path {
        interpolated.path = Some(context.interpolate(path)?);
    }

    if let Some(regex) = &extractor.regex {
        interpolated.regex = Some(context.interpolate(regex)?);
    }

    if let Some(default) = &extractor.default {
        interpolated.default = Some(context.interpolate(default)?);
    }

    if let Some(each) = &extractor.each {
        let mut next_each = HashMap::new();
        for (key, value) in each {
            next_each.insert(key.clone(), context.interpolate(value)?);
        }
        interpolated.each = Some(next_each);
    }

    Ok(interpolated)
}

/**
    Execute a Navigate step.
*/
pub async fn execute_navigate(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let url_template = step
        .url
        .as_ref()
        .ok_or_else(|| anyhow!("Navigate step '{}' requires 'url'", step.name))?;

    let url = context.interpolate(url_template)?;
    println!("[executor] Navigating to: {}", url);
    tab.navigate(&url).await?;

    if let Some(wait_for) = &step.wait_for {
        apply_wait_condition(wait_for, tab, context).await?;
    }

    Ok(())
}

/**
    Apply a wait condition with interpolation support.
*/
async fn apply_wait_condition(
    wait_for: &WaitCondition,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    if let Some(selector_template) = &wait_for.selector {
        let selector = context.interpolate(selector_template)?;
        println!("[executor] Waiting for selector: {}", selector);
        tab.wait_for_selector(&selector).await?;
    }
    if let Some(expr_template) = &wait_for.function {
        let expr = context.interpolate(expr_template)?;
        println!("[executor] Waiting for function: {}", expr);
        tab.wait_for_function(&expr).await?;
    }
    if let Some(delay) = wait_for.delay {
        println!("[executor] Waiting {} seconds", delay);
        tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
    }
    Ok(())
}

/**
    Result from executing a sniff step.
*/
pub enum SniffResult {
    /// Single values extracted (normal extractors)
    Single(HashMap<String, String>),
    /// Array of objects extracted (jsonpath_array extractor)
    Array { name: String, items: ExtractedArray },
}

/**
    Execute a Sniff step, returning extracted values.
*/
pub async fn execute_sniff(
    step: &Step,
    requests: &mut NetworkRequestStream,
    context: &InterpolationContext,
) -> Result<SniffResult> {
    use std::time::Duration;

    let request_match = step
        .request
        .as_ref()
        .ok_or_else(|| anyhow!("Sniff step '{}' requires 'request'", step.name))?;

    let url_pattern = &request_match.url;
    let method_filter = request_match.method.as_deref();
    let timeout_secs = request_match.timeout.unwrap_or(30.0);

    let url_regex = Regex::new(url_pattern)
        .map_err(|e| anyhow!("Invalid URL regex '{}': {}", url_pattern, e))?;

    println!(
        "[executor] Waiting for request matching: {} (timeout: {}s)",
        url_pattern, timeout_secs
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);

    // Check if any extractor is array-capable
    let has_array_extractor = step.extract.values().any(|e| {
        e.kind == ExtractorKind::JsonPathArray
            || e.kind == ExtractorKind::RegexArray
            || e.kind == ExtractorKind::XPathArray
            || e.kind == ExtractorKind::CssArray
    });

    // Wait for matching request
    loop {
        let next_request = tokio::time::timeout_at(deadline, requests.next()).await;

        let request = match next_request {
            Ok(Some(req)) => req,
            Ok(None) => {
                return Err(anyhow!(
                    "Network stream closed before finding match for step '{}'",
                    step.name
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "Timeout waiting for request matching '{}' in step '{}'",
                    url_pattern,
                    step.name
                ));
            }
        };

        // Check URL pattern (regex)
        let url = request.url().to_string();
        if !url_regex.is_match(&url) {
            continue;
        }

        // Check method filter
        let method = request.method();
        if let Some(expected_method) = method_filter
            && method.as_str() != expected_method
        {
            continue;
        }

        let headers = request.headers().clone();
        println!("[executor] Matched request: {}", &url[..url.len().min(80)]);

        // Get response body
        let body = if let Ok(response) = request.response().await {
            response.text().await.unwrap_or_default()
        } else {
            String::new()
        };

        // Handle array extractor specially
        if has_array_extractor {
            // Find the array extractor
            for (output_name, extractor) in &step.extract {
                if extractor.kind == ExtractorKind::JsonPathArray
                    || extractor.kind == ExtractorKind::RegexArray
                    || extractor.kind == ExtractorKind::XPathArray
                    || extractor.kind == ExtractorKind::CssArray
                {
                    let extractor = interpolate_extractor(extractor, context)?;
                    match extract_array(&extractor, &body) {
                        Ok(items) => {
                            println!(
                                "[executor] Extracted {} items from {}.{}",
                                items.len(),
                                step.name,
                                output_name
                            );
                            return Ok(SniffResult::Array {
                                name: output_name.clone(),
                                items,
                            });
                        }
                        Err(e) => {
                            println!(
                                "[executor] Array extraction failed: {}, trying next request...",
                                e
                            );
                            break;
                        }
                    }
                }
            }
            continue;
        }

        // Run normal extractors - all must succeed for this request to be accepted
        let mut extracted = HashMap::new();
        let mut all_succeeded = true;

        for (output_name, extractor) in &step.extract {
            let extractor = interpolate_extractor(extractor, context)?;
            match extract(&extractor, &body, &url, Some(&headers)) {
                Ok(value) => {
                    extracted.insert(output_name.clone(), value);
                }
                Err(_) => {
                    // Missing request headers are optional: keep output
                    // empty so content header resolution can skip them.
                    if extractor.kind == ExtractorKind::Header {
                        extracted.insert(output_name.clone(), String::new());
                        continue;
                    }
                    all_succeeded = false;
                    break;
                }
            }
        }

        if all_succeeded {
            for output_name in extracted.keys() {
                println!("[executor] Extracted {}.{}", step.name, output_name);
            }
            return Ok(SniffResult::Single(extracted));
        }

        // Extraction failed, try next matching request
        println!("[executor] Extraction failed, trying next request...");
    }
}

/**
    Execute a SniffMany step, collecting multiple matching requests and aggregating results.
*/
pub async fn execute_sniff_many(
    step: &Step,
    requests: &mut NetworkRequestStream,
    context: &InterpolationContext,
) -> Result<SniffResult> {
    use std::time::Duration;

    let request_match = step
        .request
        .as_ref()
        .ok_or_else(|| anyhow!("SniffMany step '{}' requires 'request'", step.name))?;

    let url_pattern = &request_match.url;
    let method_filter = request_match.method.as_deref();
    let timeout_secs = request_match.timeout.unwrap_or(30.0);
    let idle_timeout_secs = request_match.idle_timeout.unwrap_or(2.0);

    let url_regex = Regex::new(url_pattern)
        .map_err(|e| anyhow!("Invalid URL regex '{}': {}", url_pattern, e))?;

    println!(
        "[executor] SniffMany: collecting requests matching: {} (timeout: {}s, idle: {}s)",
        url_pattern, timeout_secs, idle_timeout_secs
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
    let idle_duration = Duration::from_secs_f64(idle_timeout_secs);

    // Check if any extractor is array-capable
    let has_array_extractor = step.extract.values().any(|e| {
        e.kind == ExtractorKind::JsonPathArray
            || e.kind == ExtractorKind::RegexArray
            || e.kind == ExtractorKind::XPathArray
            || e.kind == ExtractorKind::CssArray
    });

    // Collect all matching requests
    let mut all_items: ExtractedArray = Vec::new();
    let mut array_extractor_name: Option<String> = None;
    let mut match_count = 0;

    loop {
        // Use idle timeout for subsequent requests, but overall deadline still applies
        let wait_timeout = if match_count == 0 {
            deadline
        } else {
            let idle_deadline = tokio::time::Instant::now() + idle_duration;
            std::cmp::min(idle_deadline, deadline)
        };

        let next_request = tokio::time::timeout_at(wait_timeout, requests.next()).await;

        let request = match next_request {
            Ok(Some(req)) => req,
            Ok(None) => {
                // Stream closed
                break;
            }
            Err(_) => {
                // Timeout - if we have matches, we're done; otherwise it's an error
                if match_count > 0 {
                    println!(
                        "[executor] SniffMany: idle timeout, collected {} matches",
                        match_count
                    );
                    break;
                } else {
                    return Err(anyhow!(
                        "Timeout waiting for request matching '{}' in step '{}'",
                        url_pattern,
                        step.name
                    ));
                }
            }
        };

        let url = request.url().to_string();
        let method = request.method();

        // Check URL pattern (regex)
        if !url_regex.is_match(&url) {
            continue;
        }

        // Check method filter
        if let Some(expected_method) = method_filter
            && method.as_str() != expected_method
        {
            continue;
        }

        println!(
            "[executor] SniffMany: matched request #{}: {}",
            match_count + 1,
            &url[..url.len().min(80)]
        );

        // Get response body
        let body = if let Ok(response) = request.response().await {
            response.text().await.unwrap_or_default()
        } else {
            continue;
        };

        // Handle array extractor - aggregate items from all responses
        if has_array_extractor {
            for (output_name, extractor) in &step.extract {
                if extractor.kind == ExtractorKind::JsonPathArray
                    || extractor.kind == ExtractorKind::RegexArray
                    || extractor.kind == ExtractorKind::XPathArray
                    || extractor.kind == ExtractorKind::CssArray
                {
                    if array_extractor_name.is_none() {
                        array_extractor_name = Some(output_name.clone());
                    }
                    let extractor = interpolate_extractor(extractor, context)?;
                    match extract_array(&extractor, &body) {
                        Ok(items) => {
                            println!(
                                "[executor] SniffMany: extracted {} items from response",
                                items.len()
                            );
                            all_items.extend(items);
                            match_count += 1;
                        }
                        Err(e) => {
                            println!("[executor] SniffMany: extraction failed: {}", e);
                        }
                    }
                    break;
                }
            }
        }
    }

    if has_array_extractor && let Some(name) = array_extractor_name {
        println!(
            "[executor] SniffMany: total {} items from {} responses",
            all_items.len(),
            match_count
        );
        return Ok(SniffResult::Array {
            name,
            items: all_items,
        });
    }

    Err(anyhow!(
        "SniffMany step '{}' requires a jsonpath_array, regex_array, xpath_array, or css_array extractor",
        step.name
    ))
}

/**
    Execute a Fetch step - simple HTTP GET request without browser.
*/
async fn execute_fetch(
    step: &Step,
    context: &InterpolationContext,
    http_client: &Client,
) -> Result<SniffResult> {
    let url_template = step
        .url
        .as_ref()
        .ok_or_else(|| anyhow!("Fetch step '{}' requires 'url'", step.name))?;

    let url = context.interpolate(url_template)?;
    println!("[executor] Fetching: {}", url);

    let response = http_client
        .get(&url)
        .header("User-Agent", FETCH_USER_AGENT)
        .send()
        .await
        .map_err(|e| anyhow!("HTTP request failed for '{}': {}", url, e))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "HTTP request failed for '{}': status {}",
            url,
            response.status()
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| anyhow!("Failed to read response body: {}", e))?;

    println!("[executor] Fetched {} bytes", body.len());

    // Check if any extractor is array-capable
    let has_array_extractor = step.extract.values().any(|e| {
        e.kind == ExtractorKind::JsonPathArray
            || e.kind == ExtractorKind::RegexArray
            || e.kind == ExtractorKind::XPathArray
            || e.kind == ExtractorKind::CssArray
    });

    // Handle array extractor specially
    if has_array_extractor {
        for (output_name, extractor) in &step.extract {
            if extractor.kind == ExtractorKind::JsonPathArray
                || extractor.kind == ExtractorKind::RegexArray
                || extractor.kind == ExtractorKind::XPathArray
                || extractor.kind == ExtractorKind::CssArray
            {
                let extractor = interpolate_extractor(extractor, context)?;
                let items = extract_array(&extractor, &body)?;
                println!(
                    "[executor] Extracted {} items from {}.{}",
                    items.len(),
                    step.name,
                    output_name
                );
                return Ok(SniffResult::Array {
                    name: output_name.clone(),
                    items,
                });
            }
        }
    }

    // Run normal extractors
    let mut extracted = HashMap::new();
    for (output_name, extractor) in &step.extract {
        let extractor = interpolate_extractor(extractor, context)?;
        let value = extract(&extractor, &body, &url, None)?;
        println!("[executor] Extracted {}.{}", step.name, output_name);
        extracted.insert(output_name.clone(), value);
    }

    Ok(SniffResult::Single(extracted))
}

/**
    Execute a Document step - extract data from the current page HTML.
*/
async fn execute_document(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<SniffResult> {
    println!("[executor] Reading document HTML");

    let value = tab
        .eval_json("document.documentElement.outerHTML", false)
        .await?;

    let body = match value {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };

    // Check if any extractor is array-capable
    let has_array_extractor = step.extract.values().any(|e| {
        e.kind == ExtractorKind::JsonPathArray
            || e.kind == ExtractorKind::RegexArray
            || e.kind == ExtractorKind::XPathArray
            || e.kind == ExtractorKind::CssArray
    });

    // Handle array extractor specially
    if has_array_extractor {
        for (output_name, extractor) in &step.extract {
            if extractor.kind == ExtractorKind::JsonPathArray
                || extractor.kind == ExtractorKind::RegexArray
                || extractor.kind == ExtractorKind::XPathArray
                || extractor.kind == ExtractorKind::CssArray
            {
                let extractor = interpolate_extractor(extractor, context)?;
                let items = extract_array(&extractor, &body)?;
                println!(
                    "[executor] Extracted {} items from {}.{}",
                    items.len(),
                    step.name,
                    output_name
                );
                return Ok(SniffResult::Array {
                    name: output_name.clone(),
                    items,
                });
            }
        }
    }

    // Run normal extractors
    let mut extracted = HashMap::new();
    for (output_name, extractor) in &step.extract {
        let extractor = interpolate_extractor(extractor, context)?;
        let value = extract(&extractor, &body, "", None)?;
        println!("[executor] Extracted {}.{}", step.name, output_name);
        extracted.insert(output_name.clone(), value);
    }

    Ok(SniffResult::Single(extracted))
}

/**
    Execute a Script step in page context.
*/
async fn execute_script(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<SniffResult> {
    let script_template = step
        .script
        .as_ref()
        .ok_or_else(|| anyhow!("Script step '{}' requires 'script'", step.name))?;
    let script = context.interpolate(script_template)?;
    println!("[executor] Evaluating script step: {}", step.name);
    let _ = tab.eval_json(script, true).await?;
    Ok(SniffResult::Single(HashMap::new()))
}

/**
    Execute an Automation step containing browser interaction actions.
*/
async fn execute_automation(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    if step.steps.is_empty() {
        return Err(anyhow!(
            "Automation step '{}' requires at least one sub-action in 'steps'",
            step.name
        ));
    }

    for action in &step.steps {
        match action {
            AutomationAction::Click { selector, wait_for } => {
                execute_click_action(selector, wait_for.as_ref(), tab, context).await?;
            }
            AutomationAction::ClickIframe { selector, wait_for } => {
                execute_click_iframe_action(selector, wait_for.as_ref(), tab, context).await?;
            }
        }
    }

    Ok(())
}

/**
    Execute a Click action on an element in the main frame.

    Uses the chrome-browser Element API to wait for the element,
    scroll it into view, and click its center with real mouse events.
*/
async fn execute_click_action(
    selector_template: &str,
    wait_for: Option<&WaitCondition>,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let selector = context.interpolate(selector_template)?;
    println!("[executor] Clicking element: {}", selector);

    let element = tab.wait_for_selector(&selector).await?;
    element.click().await?;

    if let Some(wait_condition) = wait_for {
        apply_wait_condition(wait_condition, tab, context).await?;
    }

    Ok(())
}

/**
    Execute a Click action on an iframe element.

    Finds the iframe element in the main frame by CSS selector, scrolls it
    into view, and clicks its center. Chrome routes the click into the
    cross-origin iframe, which is useful for starting embedded video players.
*/
async fn execute_click_iframe_action(
    selector_template: &str,
    wait_for: Option<&WaitCondition>,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let selector = context.interpolate(selector_template)?;
    println!("[executor] Clicking iframe: {}", selector);

    let element = tab.wait_for_selector(&selector).await?;
    element.click().await?;

    if let Some(wait_condition) = wait_for {
        apply_wait_condition(wait_condition, tab, context).await?;
    }

    Ok(())
}

/**
    Execute a BrowserFetch step - fetches via the page context to inherit cookies.
*/
async fn execute_fetch_in_browser(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<SniffResult> {
    let url_template = step
        .url
        .as_ref()
        .ok_or_else(|| anyhow!("FetchInBrowser step '{}' requires 'url'", step.name))?;

    let url = context.interpolate(url_template)?;
    println!("[executor] FetchInBrowser: {}", url);

    let script = format!(
        r#"(async () => {{
            const tryFetch = async (options) => {{
                const res = await fetch({url:?}, options);
                if (!res.ok) throw new Error('HTTP ' + res.status);
                const body = await res.text();
                return {{ url: res.url, body }};
            }};
            try {{
                return await tryFetch({{ credentials: 'include', mode: 'cors' }});
            }} catch (err) {{
                return await tryFetch({{ credentials: 'omit', mode: 'cors' }});
            }}
        }})()"#
    );

    let value = tab.eval_json(script, true).await?;
    let (response_url, body) = match value {
        serde_json::Value::Object(mut obj) => {
            let response_url = obj
                .remove("url")
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| url.clone());
            let body = obj
                .remove("body")
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_default();
            (response_url, body)
        }
        serde_json::Value::String(s) => (url.clone(), s),
        other => (url.clone(), other.to_string()),
    };

    println!("[executor] Browser fetched {} bytes", body.len());

    // Check if any extractor is array-capable
    let has_array_extractor = step.extract.values().any(|e| {
        e.kind == ExtractorKind::JsonPathArray
            || e.kind == ExtractorKind::RegexArray
            || e.kind == ExtractorKind::XPathArray
            || e.kind == ExtractorKind::CssArray
    });

    // Handle array extractor specially
    if has_array_extractor {
        for (output_name, extractor) in &step.extract {
            if extractor.kind == ExtractorKind::JsonPathArray
                || extractor.kind == ExtractorKind::RegexArray
                || extractor.kind == ExtractorKind::XPathArray
                || extractor.kind == ExtractorKind::CssArray
            {
                let extractor = interpolate_extractor(extractor, context)?;
                let items = extract_array(&extractor, &body)?;
                println!(
                    "[executor] Extracted {} items from {}.{}",
                    items.len(),
                    step.name,
                    output_name
                );
                return Ok(SniffResult::Array {
                    name: output_name.clone(),
                    items,
                });
            }
        }
    }

    // Run normal extractors
    let mut extracted = HashMap::new();
    for (output_name, extractor) in &step.extract {
        let extractor = interpolate_extractor(extractor, context)?;
        let value = extract(&extractor, &body, &response_url, None)?;
        println!("[executor] Extracted {}.{}", step.name, output_name);
        extracted.insert(output_name.clone(), value);
    }

    Ok(SniffResult::Single(extracted))
}

/**
    Execute a list of steps, returning the interpolation context.
    This is used by both discovery and content phases.
*/
pub async fn execute_steps(
    steps: &[Step],
    tab: &ChromeBrowserTab,
    initial_context: InterpolationContext,
    proxy: Option<&str>,
) -> Result<(InterpolationContext, Option<(String, ExtractedArray)>)> {
    let mut context = initial_context;
    let mut requests = tab.network().requests();
    let mut array_result: Option<(String, ExtractedArray)> = None;

    // Create HTTP client for Fetch steps with optional proxy
    let http_client = if let Some(proxy_url) = proxy {
        let proxy = Proxy::all(proxy_url)
            .map_err(|e| anyhow!("Invalid proxy URL '{}': {}", proxy_url, e))?;
        Client::builder()
            .proxy(proxy)
            .build()
            .map_err(|e| anyhow!("Failed to create HTTP client with proxy: {}", e))?
    } else {
        Client::new()
    };

    for step in steps {
        println!("[executor] Running step: {}", step.name);

        match step.kind {
            StepKind::Navigate => {
                execute_navigate(step, tab, &context).await?;
            }
            StepKind::Sniff => {
                match execute_sniff(step, &mut requests, &context).await? {
                    SniffResult::Single(values) => {
                        for (output_name, value) in values {
                            context.set(&step.name, &output_name, value);
                        }
                    }
                    SniffResult::Array { name, items } => {
                        // Store array result for later processing
                        // The step.name and extractor name form the reference
                        array_result = Some((format!("{}.{}", step.name, name), items));
                    }
                }
            }
            StepKind::SniffMany => {
                match execute_sniff_many(step, &mut requests, &context).await? {
                    SniffResult::Single(values) => {
                        for (output_name, value) in values {
                            context.set(&step.name, &output_name, value);
                        }
                    }
                    SniffResult::Array { name, items } => {
                        // Store array result for later processing
                        // The step.name and extractor name form the reference
                        array_result = Some((format!("{}.{}", step.name, name), items));
                    }
                }
            }
            StepKind::Fetch => match execute_fetch(step, &context, &http_client).await? {
                SniffResult::Single(values) => {
                    for (output_name, value) in values {
                        context.set(&step.name, &output_name, value);
                    }
                }
                SniffResult::Array { name, items } => {
                    array_result = Some((format!("{}.{}", step.name, name), items));
                }
            },
            StepKind::FetchInBrowser => {
                match execute_fetch_in_browser(step, tab, &context).await? {
                    SniffResult::Single(values) => {
                        for (output_name, value) in values {
                            context.set(&step.name, &output_name, value);
                        }
                    }
                    SniffResult::Array { name, items } => {
                        array_result = Some((format!("{}.{}", step.name, name), items));
                    }
                }
            }
            StepKind::Document => match execute_document(step, tab, &context).await? {
                SniffResult::Single(values) => {
                    for (output_name, value) in values {
                        context.set(&step.name, &output_name, value);
                    }
                }
                SniffResult::Array { name, items } => {
                    array_result = Some((format!("{}.{}", step.name, name), items));
                }
            },
            StepKind::Script => {
                let _ = execute_script(step, tab, &context).await?;
            }
            StepKind::Automation => {
                execute_automation(step, tab, &context).await?;
            }
        }
    }

    Ok((context, array_result))
}
