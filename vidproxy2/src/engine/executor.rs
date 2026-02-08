use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrome_browser::{ChromeBrowserTab, NetworkRequestStream};
use regex::Regex;
use reqwest::{Client, Proxy};

use super::extractor::{ExtractedArray, extract, extract_array};
use super::interpolate::InterpolationContext;
use super::step::{
    AutomationAction, Extractor, ExtractorKind, RequestMatch, Step, WaitCondition,
    is_array_extractor,
};

const FETCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Output from executing a phase's steps.
pub struct PhaseOutput {
    pub context: InterpolationContext,
    pub arrays: HashMap<String, ExtractedArray>,
}

/// Result from a single step's extraction.
enum StepResult {
    Single(HashMap<String, String>),
    Array { name: String, items: ExtractedArray },
    Empty,
}

/// Store a step's result into the phase output.
///
/// Arrays are keyed by output name only (not step name), so multiple steps
/// producing the same array name will merge their items — consistent with
/// how `SniffMany` accumulates items across multiple network responses.
fn store_result(output: &mut PhaseOutput, step_name: &str, result: StepResult) {
    match result {
        StepResult::Single(values) => {
            for (k, v) in values {
                output.context.set(step_name, &k, v);
            }
        }
        StepResult::Array { name, items } => {
            output.arrays.entry(name).or_default().extend(items);
        }
        StepResult::Empty => {}
    }
}

/// Execute a list of steps, returning the accumulated phase output.
pub async fn execute_steps(
    steps: &[Step],
    tab: &ChromeBrowserTab,
    initial_context: InterpolationContext,
    proxy: Option<&str>,
) -> Result<PhaseOutput> {
    let mut output = PhaseOutput {
        context: initial_context,
        arrays: HashMap::new(),
    };

    let mut requests = tab.network().requests();

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
        let step_name = step.name();
        println!("[executor] Running step: {}", step_name);

        let result = match step {
            Step::Navigate { url, wait_for, .. } => {
                execute_navigate(url, wait_for.as_ref(), tab, &output.context).await?;
                StepResult::Empty
            }
            Step::Sniff {
                request,
                extract: extractors,
                ..
            } => execute_sniff(request, extractors, &mut requests, &output.context).await?,
            Step::SniffMany {
                request,
                extract: extractors,
                ..
            } => execute_sniff_many(request, extractors, &mut requests, &output.context).await?,
            Step::Fetch {
                url,
                headers,
                extract: extractors,
                ..
            } => execute_fetch(url, headers, extractors, &output.context, &http_client).await?,
            Step::FetchInBrowser {
                url,
                extract: extractors,
                ..
            } => execute_fetch_in_browser(url, extractors, tab, &output.context).await?,
            Step::Document {
                extract: extractors,
                ..
            } => execute_document(extractors, tab, &output.context).await?,
            Step::Script { script, .. } => {
                execute_script(script, tab, &output.context).await?;
                StepResult::Empty
            }
            Step::Automation { steps: actions, .. } => {
                execute_automation(actions, tab, &output.context).await?;
                StepResult::Empty
            }
        };

        store_result(&mut output, step_name, result);
    }

    Ok(output)
}

// ── Step handlers ────────────────────────────────────────────────────────────

async fn execute_navigate(
    url_template: &str,
    wait_for: Option<&WaitCondition>,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let url = context.interpolate(url_template)?;
    println!("[executor] Navigating to: {}", url);
    tab.navigate(&url).await?;

    if let Some(wait_for) = wait_for {
        apply_wait_condition(wait_for, tab, context).await?;
    }

    Ok(())
}

async fn execute_sniff(
    request_match: &RequestMatch,
    extractors: &HashMap<String, Extractor>,
    requests: &mut NetworkRequestStream,
    context: &InterpolationContext,
) -> Result<StepResult> {
    use std::time::Duration;

    let url_regex = Regex::new(&request_match.url)
        .map_err(|e| anyhow!("Invalid URL regex '{}': {}", request_match.url, e))?;

    let timeout_secs = request_match.timeout.unwrap_or(30.0);

    println!(
        "[executor] Waiting for request matching: {} (timeout: {}s)",
        request_match.url, timeout_secs
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);

    let has_array = extractors.values().any(|e| is_array_extractor(&e.kind));

    loop {
        let next_request = tokio::time::timeout_at(deadline, requests.next()).await;

        let request = match next_request {
            Ok(Some(req)) => req,
            Ok(None) => return Err(anyhow!("Network stream closed before finding match")),
            Err(_) => {
                return Err(anyhow!(
                    "Timeout waiting for request matching '{}'",
                    request_match.url
                ));
            }
        };

        let url = request.url().to_string();
        if !url_regex.is_match(&url) {
            continue;
        }

        if let Some(expected_method) = &request_match.method
            && request.method().as_str() != expected_method.as_str()
        {
            continue;
        }

        let headers = request.headers().clone();
        println!("[executor] Matched request: {}", &url[..url.len().min(80)]);

        let body = if let Ok(response) = request.response().await {
            response.text().await.unwrap_or_default()
        } else {
            String::new()
        };

        match run_extractors(extractors, &body, &url, Some(&headers), has_array, context) {
            Ok(result) => return Ok(result),
            Err(_) => {
                println!("[executor] Extraction failed, trying next request...");
                continue;
            }
        }
    }
}

async fn execute_sniff_many(
    request_match: &RequestMatch,
    extractors: &HashMap<String, Extractor>,
    requests: &mut NetworkRequestStream,
    context: &InterpolationContext,
) -> Result<StepResult> {
    use std::time::Duration;

    let url_regex = Regex::new(&request_match.url)
        .map_err(|e| anyhow!("Invalid URL regex '{}': {}", request_match.url, e))?;

    let timeout_secs = request_match.timeout.unwrap_or(30.0);
    let idle_timeout_secs = request_match.idle_timeout.unwrap_or(2.0);

    println!(
        "[executor] SniffMany: collecting requests matching: {} (timeout: {}s, idle: {}s)",
        request_match.url, timeout_secs, idle_timeout_secs
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
    let idle_duration = Duration::from_secs_f64(idle_timeout_secs);

    let mut all_items: ExtractedArray = Vec::new();
    let mut array_extractor_name: Option<String> = None;
    let mut match_count = 0;

    loop {
        let wait_timeout = if match_count == 0 {
            deadline
        } else {
            let idle_deadline = tokio::time::Instant::now() + idle_duration;
            std::cmp::min(idle_deadline, deadline)
        };

        let next_request = tokio::time::timeout_at(wait_timeout, requests.next()).await;

        let request = match next_request {
            Ok(Some(req)) => req,
            Ok(None) => break,
            Err(_) => {
                if match_count > 0 {
                    println!(
                        "[executor] SniffMany: idle timeout, collected {} matches",
                        match_count
                    );
                    break;
                } else {
                    return Err(anyhow!(
                        "Timeout waiting for request matching '{}'",
                        request_match.url
                    ));
                }
            }
        };

        let url = request.url().to_string();
        if !url_regex.is_match(&url) {
            continue;
        }

        if let Some(expected_method) = &request_match.method
            && request.method().as_str() != expected_method.as_str()
        {
            continue;
        }

        println!(
            "[executor] SniffMany: matched request #{}: {}",
            match_count + 1,
            &url[..url.len().min(80)]
        );

        let body = if let Ok(response) = request.response().await {
            response.text().await.unwrap_or_default()
        } else {
            continue;
        };

        for (output_name, extractor) in extractors {
            if is_array_extractor(&extractor.kind) {
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

    if let Some(name) = array_extractor_name {
        println!(
            "[executor] SniffMany: total {} items from {} responses",
            all_items.len(),
            match_count
        );
        return Ok(StepResult::Array {
            name,
            items: all_items,
        });
    }

    Err(anyhow!("SniffMany requires an array extractor"))
}

async fn execute_fetch(
    url_template: &str,
    step_headers: &HashMap<String, String>,
    extractors: &HashMap<String, Extractor>,
    context: &InterpolationContext,
    http_client: &Client,
) -> Result<StepResult> {
    let url = context.interpolate(url_template)?;
    println!("[executor] Fetching: {}", url);

    let mut request = http_client.get(&url).header("User-Agent", FETCH_USER_AGENT);

    for (key, value_template) in step_headers {
        let value = context.interpolate(value_template)?;
        if !value.trim().is_empty() {
            request = request.header(key.as_str(), value);
        }
    }

    let response = request
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

    let has_array = extractors.values().any(|e| is_array_extractor(&e.kind));
    run_extractors(extractors, &body, &url, None, has_array, context)
}

async fn execute_fetch_in_browser(
    url_template: &str,
    extractors: &HashMap<String, Extractor>,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<StepResult> {
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

    let has_array = extractors.values().any(|e| is_array_extractor(&e.kind));
    run_extractors(extractors, &body, &response_url, None, has_array, context)
}

async fn execute_document(
    extractors: &HashMap<String, Extractor>,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<StepResult> {
    println!("[executor] Reading document HTML");

    let value = tab
        .eval_json("document.documentElement.outerHTML", false)
        .await?;

    let body = match value {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };

    let has_array = extractors.values().any(|e| is_array_extractor(&e.kind));
    run_extractors(extractors, &body, "", None, has_array, context)
}

async fn execute_script(
    script_template: &str,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let script = context.interpolate(script_template)?;
    println!("[executor] Evaluating script");
    let _ = tab.eval_json(script, true).await?;
    Ok(())
}

async fn execute_automation(
    actions: &[AutomationAction],
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    for action in actions {
        match action {
            AutomationAction::Click { selector, wait_for } => {
                let selector = context.interpolate(selector)?;
                println!("[executor] Clicking element: {}", selector);
                let element = tab.wait_for_selector(&selector).await?;
                element.click().await?;
                if let Some(wait_condition) = wait_for {
                    apply_wait_condition(wait_condition, tab, context).await?;
                }
            }
            AutomationAction::ClickIframe { selector, wait_for } => {
                let selector = context.interpolate(selector)?;
                println!("[executor] Clicking iframe: {}", selector);
                let element = tab.wait_for_selector(&selector).await?;
                element.click().await?;
                if let Some(wait_condition) = wait_for {
                    apply_wait_condition(wait_condition, tab, context).await?;
                }
            }
        }
    }
    Ok(())
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Run extractors on content, handling both array and scalar results.
/// This is the single function that replaces the 6 duplicated patterns.
fn run_extractors(
    extractors: &HashMap<String, Extractor>,
    body: &str,
    url: &str,
    headers: Option<&axum::http::HeaderMap>,
    has_array: bool,
    context: &InterpolationContext,
) -> Result<StepResult> {
    if has_array {
        for (output_name, extractor) in extractors {
            if is_array_extractor(&extractor.kind) {
                let extractor = interpolate_extractor(extractor, context)?;
                let items = extract_array(&extractor, body)?;
                println!(
                    "[executor] Extracted {} items from {}",
                    items.len(),
                    output_name
                );
                return Ok(StepResult::Array {
                    name: output_name.clone(),
                    items,
                });
            }
        }
    }

    let mut extracted = HashMap::new();
    for (output_name, extractor) in extractors {
        let extractor = interpolate_extractor(extractor, context)?;
        match extract(&extractor, body, url, headers) {
            Ok(value) => {
                extracted.insert(output_name.clone(), value);
            }
            Err(_) => {
                if extractor.kind == ExtractorKind::Header {
                    extracted.insert(output_name.clone(), String::new());
                    continue;
                }
                return Err(anyhow!("Extraction failed for '{}'", output_name));
            }
        }
    }

    for output_name in extracted.keys() {
        println!("[executor] Extracted {}", output_name);
    }

    Ok(StepResult::Single(extracted))
}

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
