use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use sxd_xpath::nodeset::Node;

use super::types::{Extractor, ExtractorKind};

/**
    Result of extracting from an array - a list of objects with string fields.
*/
pub type ExtractedArray = Vec<HashMap<String, Option<String>>>;

/**
    Run an extractor on the given content.
    Returns a single string value.
*/
pub fn extract(extractor: &Extractor, content: &str, url: &str) -> Result<String> {
    let value = match extractor.kind {
        ExtractorKind::Url => Ok(url.to_string()),
        ExtractorKind::UrlRegex => extract_regex(extractor, url),
        ExtractorKind::JsonPath => extract_jsonpath(extractor, content),
        ExtractorKind::JsonPathArray => {
            Err(anyhow!("Use extract_array() for jsonpath_array extractors"))
        }
        ExtractorKind::JsonPathRegex => extract_jsonpath_regex(extractor, content),
        ExtractorKind::Css => extract_css(extractor, content),
        ExtractorKind::CssArray => Err(anyhow!("Use extract_array() for css_array extractors")),
        ExtractorKind::XPath => extract_xpath(extractor, content),
        ExtractorKind::XPathArray => Err(anyhow!("Use extract_array() for xpath_array extractors")),
        ExtractorKind::Regex => extract_regex(extractor, content),
        ExtractorKind::RegexArray => Err(anyhow!("Use extract_array() for regex_array extractors")),
        ExtractorKind::Line => extract_line(content),
        ExtractorKind::Pssh => extract_pssh(content, url),
    }?;

    // Apply unescaping if requested
    if extractor.unescape {
        Ok(unescape_json_string(&value))
    } else {
        Ok(value)
    }
}

/**
    Unescape JSON string escape sequences.
    Handles both \uXXXX and \\uXXXX sequences (e.g., \u0026 or \\u0026 -> &).
    The double-backslash form is common in doubly-escaped JSON (JSON inside JSON).

    Uses serde_json for proper JSON string parsing.
*/
fn unescape_json_string(s: &str) -> String {
    // First, try to parse as a JSON string (wrapped in quotes)
    let quoted = format!("\"{}\"", s);
    if let Ok(serde_json::Value::String(unescaped)) = serde_json::from_str(&quoted) {
        // If it still contains escape sequences (doubly-escaped), unescape again
        if unescaped.contains("\\u") {
            let quoted2 = format!("\"{}\"", unescaped);
            if let Ok(serde_json::Value::String(double_unescaped)) = serde_json::from_str(&quoted2)
            {
                return double_unescaped;
            }
        }
        return unescaped;
    }

    // If parsing fails, return the original string
    s.to_string()
}

/**
    Run a jsonpath_array extractor on the given content.
    Returns an array of objects, each with the fields defined in `each`.

    For discovery: Objects missing the "id" field are skipped.
    For metadata: Objects missing "channel_id" or "title" are skipped.

    Supports `$parent` in field paths to reference parent objects when using
    nested array paths like `$.result[*].content.epg[*]`.
*/
pub fn extract_array(extractor: &Extractor, content: &str) -> Result<ExtractedArray> {
    if extractor.kind != ExtractorKind::JsonPathArray
        && extractor.kind != ExtractorKind::RegexArray
        && extractor.kind != ExtractorKind::XPathArray
        && extractor.kind != ExtractorKind::CssArray
    {
        return Err(anyhow!(
            "extract_array() only works with jsonpath_array, regex_array, xpath_array, or css_array extractors"
        ));
    }

    match extractor.kind {
        ExtractorKind::JsonPathArray => {
            let path = extractor
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("jsonpath_array extractor requires 'path'"))?;

            let each = extractor
                .each
                .as_ref()
                .ok_or_else(|| anyhow!("jsonpath_array extractor requires 'each'"))?;

            extract_jsonpath_array(content, path, each)
        }
        ExtractorKind::RegexArray => extract_regex_array(extractor, content),
        ExtractorKind::XPathArray => extract_xpath_array(extractor, content),
        ExtractorKind::CssArray => extract_css_array(extractor, content),
        _ => unreachable!("checked above"),
    }
}

/**
    Extract using JSONPath, returning array of objects.
    Supports $parent references in field paths for nested array extractions.
*/
fn extract_jsonpath_array(
    content: &str,
    path: &str,
    each: &HashMap<String, String>,
) -> Result<ExtractedArray> {
    let json: serde_json::Value =
        serde_json::from_str(content).map_err(|e| anyhow!("Failed to parse JSON: {}", e))?;

    // Check if any field uses $parent - if so, we need to track parent context
    let needs_parent = each.values().any(|p| p.contains("$parent"));

    if needs_parent {
        extract_with_parent_context(&json, path, each)
    } else {
        extract_simple(&json, path, each)
    }
}

/**
    Simple extraction without parent context tracking.
*/
fn extract_simple(
    json: &serde_json::Value,
    path: &str,
    each: &HashMap<String, String>,
) -> Result<ExtractedArray> {
    use jsonpath_rust::JsonPath;
    use std::str::FromStr;

    let jsonpath =
        JsonPath::from_str(path).map_err(|e| anyhow!("Invalid JSONPath '{}': {}", path, e))?;

    let results = jsonpath.find_slice(json);

    if results.is_empty() {
        return Err(anyhow!("JSONPath '{}' returned no results", path));
    }

    let mut extracted = Vec::new();

    for result in results {
        let obj = result.clone().to_data();

        let mut fields: HashMap<String, Option<String>> = HashMap::new();

        for (field_name, field_path) in each {
            let value = extract_jsonpath_value(&obj, field_path);
            fields.insert(field_name.clone(), value);
        }

        // Skip based on what fields are present
        if should_skip_item(&fields) {
            continue;
        }

        extracted.push(fields);
    }

    if extracted.is_empty() {
        return Err(anyhow!(
            "JSONPath '{}' returned results but none had required fields",
            path
        ));
    }

    Ok(extracted)
}

/**
    Extraction with parent context tracking for $parent references.
    Handles paths like `$.result[*].content.epg[*]` where we need to access
    fields from the parent `result[*]` item while iterating over `epg[*]`.
*/
fn extract_with_parent_context(
    json: &serde_json::Value,
    path: &str,
    each: &HashMap<String, String>,
) -> Result<ExtractedArray> {
    use jsonpath_rust::JsonPath;
    use std::str::FromStr;

    // Parse the path to find where the nested arrays are
    // e.g., "$.result[*].content.epg[*]" -> parent: "$.result[*]", child: "$.content.epg[*]"
    let (parent_path, child_path) = split_nested_path(path)?;

    let parent_jsonpath = JsonPath::from_str(&parent_path)
        .map_err(|e| anyhow!("Invalid parent JSONPath '{}': {}", parent_path, e))?;

    let child_jsonpath = JsonPath::from_str(&child_path)
        .map_err(|e| anyhow!("Invalid child JSONPath '{}': {}", child_path, e))?;

    let parent_results = parent_jsonpath.find_slice(json);

    if parent_results.is_empty() {
        return Err(anyhow!(
            "Parent JSONPath '{}' returned no results",
            parent_path
        ));
    }

    let mut extracted = Vec::new();

    for parent_result in parent_results {
        let parent_obj = parent_result.clone().to_data();
        let child_results = child_jsonpath.find_slice(&parent_obj);

        for child_result in child_results {
            let child_obj = child_result.clone().to_data();

            let mut fields: HashMap<String, Option<String>> = HashMap::new();

            for (field_name, field_path) in each {
                let value = if field_path.starts_with("$parent") {
                    // Extract from parent object
                    let parent_field_path = field_path.replacen("$parent", "$", 1);
                    extract_jsonpath_value(&parent_obj, &parent_field_path)
                } else {
                    // Extract from child object
                    extract_jsonpath_value(&child_obj, field_path)
                };
                fields.insert(field_name.clone(), value);
            }

            if should_skip_item(&fields) {
                continue;
            }

            extracted.push(fields);
        }
    }

    if extracted.is_empty() {
        return Err(anyhow!(
            "Nested extraction returned no valid items with required fields"
        ));
    }

    Ok(extracted)
}

/**
    Split a nested JSONPath into parent and child portions.
    e.g., "$.result[*].content.epg[*]" -> ("$.result[*]", "$.content.epg[*]")

    Finds the last `[*]` and splits there, as that's typically where
    the parent/child boundary is for nested array access.
*/
fn split_nested_path(path: &str) -> Result<(String, String)> {
    // Find all occurrences of [*]
    let indices: Vec<_> = path.match_indices("[*]").collect();

    if indices.len() < 2 {
        return Err(anyhow!(
            "Path '{}' needs at least two [*] for $parent support",
            path
        ));
    }

    // Split at the first [*] - that's the parent array
    let (first_idx, _) = indices[0];
    let split_point = first_idx + 3; // After the first [*]

    let parent_path = path[..split_point].to_string();
    let child_path = format!("${}", &path[split_point..]);

    Ok((parent_path, child_path))
}

/**
    Determine if an extracted item should be skipped based on required fields.
    - For discovery: requires "id"
    - For metadata: requires "channel_id" and "title"
*/
fn should_skip_item(fields: &HashMap<String, Option<String>>) -> bool {
    let has_id = fields.get("id").and_then(|v| v.as_ref()).is_some();
    let has_channel_id = fields.get("channel_id").and_then(|v| v.as_ref()).is_some();
    let has_title = fields.get("title").and_then(|v| v.as_ref()).is_some();

    // If it has channel_id, it's metadata - require channel_id and title
    if fields.contains_key("channel_id") {
        return !has_channel_id || !has_title;
    }

    // Otherwise it's discovery - require id
    !has_id
}

/**
    Extract a single value from a JSON object using JSONPath.
    Returns None if the path doesn't match or returns null.
*/
fn extract_jsonpath_value(obj: &serde_json::Value, path: &str) -> Option<String> {
    use jsonpath_rust::JsonPath;
    use std::str::FromStr;

    let jsonpath = JsonPath::from_str(path).ok()?;
    let results = jsonpath.find_slice(obj);

    if results.is_empty() {
        return None;
    }

    match results[0].clone().to_data() {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

/**
    Extract using JSONPath (single value).
*/
fn extract_jsonpath(extractor: &Extractor, content: &str) -> Result<String> {
    use jsonpath_rust::JsonPath;
    use std::str::FromStr;

    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("JSONPath extractor requires 'path'"))?;

    let json: serde_json::Value =
        serde_json::from_str(content).map_err(|e| anyhow!("Failed to parse JSON: {}", e))?;

    let jsonpath =
        JsonPath::from_str(path).map_err(|e| anyhow!("Invalid JSONPath '{}': {}", path, e))?;

    let results = jsonpath.find_slice(&json);

    if results.is_empty() {
        return Err(anyhow!("JSONPath '{}' returned no results", path));
    }

    // Return the first result as a string
    match results[0].clone().to_data() {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Err(anyhow!("JSONPath '{}' returned null", path)),
        other => Ok(other.to_string()),
    }
}

/**
    Extract using JSONPath followed by regex on the result.
*/
fn extract_jsonpath_regex(extractor: &Extractor, content: &str) -> Result<String> {
    use jsonpath_rust::JsonPath;
    use std::str::FromStr;

    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("JSONPath regex extractor requires 'path'"))?;

    let regex_pattern = extractor
        .regex
        .as_ref()
        .ok_or_else(|| anyhow!("JSONPath regex extractor requires 'regex'"))?;

    let json: serde_json::Value =
        serde_json::from_str(content).map_err(|e| anyhow!("Failed to parse JSON: {}", e))?;

    let jsonpath =
        JsonPath::from_str(path).map_err(|e| anyhow!("Invalid JSONPath '{}': {}", path, e))?;

    let results = jsonpath.find_slice(&json);

    if results.is_empty() {
        return Err(anyhow!("JSONPath '{}' returned no results", path));
    }

    // Get the first result as a string
    let value = match results[0].clone().to_data() {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => return Err(anyhow!("JSONPath '{}' returned null", path)),
        other => other.to_string(),
    };

    // Apply regex to the extracted value
    let re = Regex::new(regex_pattern)
        .map_err(|e| anyhow!("Invalid regex '{}': {}", regex_pattern, e))?;

    let captures = re
        .captures(&value)
        .ok_or_else(|| anyhow!("Regex '{}' did not match value '{}'", regex_pattern, value))?;

    // Return first capture group, or whole match if no groups
    if captures.len() > 1 {
        Ok(captures.get(1).unwrap().as_str().to_string())
    } else {
        Ok(captures.get(0).unwrap().as_str().to_string())
    }
}

/**
    Extract using XPath.
*/
fn extract_xpath(extractor: &Extractor, content: &str) -> Result<String> {
    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("XPath extractor requires 'path'"))?;

    let package = parse_xpath_document(content)?;
    let document = package.as_document();

    let factory = sxd_xpath::Factory::new();
    let xpath = factory
        .build(path)
        .map_err(|e| anyhow!("Invalid XPath '{}': {:?}", path, e))?
        .ok_or_else(|| anyhow!("XPath '{}' is empty", path))?;

    let context = sxd_xpath::Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|e| anyhow!("XPath evaluation failed: {:?}", e))?;

    match value {
        sxd_xpath::Value::String(s) => Ok(s),
        sxd_xpath::Value::Number(n) => Ok(n.to_string()),
        sxd_xpath::Value::Boolean(b) => Ok(b.to_string()),
        sxd_xpath::Value::Nodeset(nodes) => {
            if nodes.size() == 0 {
                return Err(anyhow!("XPath '{}' returned no nodes", path));
            }
            // Get text content of first node
            let node = nodes.iter().next().unwrap();
            Ok(node.string_value())
        }
    }
}

/**
    Extract multiple matches using XPath, returning an array of objects.
*/
fn extract_xpath_array(extractor: &Extractor, content: &str) -> Result<ExtractedArray> {
    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("xpath_array extractor requires 'path'"))?;

    let each = extractor
        .each
        .as_ref()
        .ok_or_else(|| anyhow!("xpath_array extractor requires 'each'"))?;

    let package = parse_xpath_document(content)?;
    let document = package.as_document();

    let factory = sxd_xpath::Factory::new();
    let xpath = factory
        .build(path)
        .map_err(|e| anyhow!("Invalid XPath '{}': {:?}", path, e))?
        .ok_or_else(|| anyhow!("XPath '{}' is empty", path))?;

    let context = sxd_xpath::Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|e| anyhow!("XPath evaluation failed: {:?}", e))?;

    let nodes = match value {
        sxd_xpath::Value::Nodeset(nodes) => nodes,
        _ => {
            return Err(anyhow!(
                "XPath '{}' must return a nodeset for xpath_array",
                path
            ));
        }
    };

    if nodes.size() == 0 {
        return Err(anyhow!("XPath '{}' returned no nodes", path));
    }

    let mut extracted = Vec::new();

    for node in nodes.iter() {
        let mut fields: HashMap<String, Option<String>> = HashMap::new();

        for (field_name, selector) in each {
            let mut value: Option<String> = None;

            for candidate in selector.split('|').map(|s| s.trim()) {
                if candidate.is_empty() {
                    continue;
                }

                if let Some(const_value) = candidate.strip_prefix("const:") {
                    value = Some(const_value.to_string());
                    break;
                }

                let extracted_value = extract_xpath_value(candidate, node)?;
                if extracted_value.is_some() {
                    value = extracted_value;
                    break;
                }
            }

            let value = if extractor.unescape {
                value.map(|v| unescape_html_string(&v))
            } else {
                value
            };

            fields.insert(field_name.clone(), value);
        }

        if should_skip_item(&fields) {
            continue;
        }

        extracted.push(fields);
    }

    if extracted.is_empty() {
        return Err(anyhow!("XPath '{}' returned no results", path));
    }

    Ok(extracted)
}

fn extract_xpath_value(path: &str, node: Node) -> Result<Option<String>> {
    let factory = sxd_xpath::Factory::new();
    let xpath = factory
        .build(path)
        .map_err(|e| anyhow!("Invalid XPath '{}': {:?}", path, e))?
        .ok_or_else(|| anyhow!("XPath '{}' is empty", path))?;

    let context = sxd_xpath::Context::new();
    let value = xpath
        .evaluate(&context, node)
        .map_err(|e| anyhow!("XPath evaluation failed: {:?}", e))?;

    let result = match value {
        sxd_xpath::Value::String(s) => s,
        sxd_xpath::Value::Number(n) => n.to_string(),
        sxd_xpath::Value::Boolean(b) => b.to_string(),
        sxd_xpath::Value::Nodeset(nodes) => {
            if nodes.size() == 0 {
                return Ok(None);
            }
            let node = nodes.iter().next().unwrap();
            node.string_value()
        }
    };

    if result.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

fn parse_xpath_document(content: &str) -> Result<sxd_document::Package> {
    match sxd_document::parser::parse(content) {
        Ok(package) => Ok(package),
        Err(_) => {
            let sanitized = sanitize_html_for_xpath(content);
            sxd_document::parser::parse(&sanitized)
                .map_err(|e| anyhow!("Failed to parse XML: {:?}", e))
        }
    }
}

fn sanitize_html_for_xpath(input: &str) -> String {
    let mut output = input.to_string();

    if let Ok(re) = Regex::new(r"(?is)<!doctype[^>]*>") {
        output = re.replace_all(&output, "").into_owned();
    }
    if let Ok(re) = Regex::new(r"(?is)<script[^>]*>.*?</script>") {
        output = re.replace_all(&output, "").into_owned();
    }
    if let Ok(re) = Regex::new(r"(?is)<style[^>]*>.*?</style>") {
        output = re.replace_all(&output, "").into_owned();
    }

    let void_tags = [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];

    for tag in void_tags {
        if let Ok(re) = Regex::new(&format!(r"(?i)<{}\\b[^>]*?>", tag)) {
            output = re
                .replace_all(&output, |caps: &regex::Captures| {
                    let m = caps.get(0).unwrap().as_str();
                    if m.ends_with("/>") {
                        m.to_string()
                    } else {
                        let mut s = m.to_string();
                        if s.ends_with('>') {
                            s.pop();
                            s.push_str("/>");
                        }
                        s
                    }
                })
                .into_owned();
        }
    }

    // Normalize boolean attributes to key="key" to satisfy XML parsing
    let re = boolean_attr_regex();
    loop {
        let next = re
            .replace_all(&output, |caps: &regex::Captures| {
                let prefix = caps.get(1).unwrap().as_str();
                let name = caps.get(2).unwrap().as_str();
                let suffix = caps.get(3).unwrap().as_str();
                format!(r#"{} {}="{}"{}"#, prefix, name, name, suffix)
            })
            .into_owned();

        if next == output {
            break;
        }
        output = next;
    }

    output
}

fn boolean_attr_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(<[^/!][^>]*?)\s([A-Za-z_:][A-Za-z0-9_:.:-]*)(\s|/?>)"#)
            .expect("boolean attribute regex should compile")
    })
}

#[derive(Debug)]
enum CssTarget {
    Text,
    Attr(String),
}

/**
    Extract using CSS selectors (HTML).
    Supports `selector::text` and `selector::attr(name)` suffixes.
*/
fn extract_css(extractor: &Extractor, content: &str) -> Result<String> {
    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("CSS extractor requires 'path'"))?;

    let (selector, target) = parse_css_path(path)?;
    let document = Html::parse_document(content);

    let value = if selector.is_empty() {
        None
    } else {
        let selector = Selector::parse(&selector)
            .map_err(|e| anyhow!("Invalid CSS selector '{}': {:?}", selector, e))?;

        document
            .select(&selector)
            .next()
            .and_then(|element| extract_css_value_from_element(element, &target))
    };

    if let Some(value) = value {
        return Ok(value);
    }

    if let Some(default) = &extractor.default {
        return Ok(default.clone());
    }

    Err(anyhow!("CSS selector '{}' returned no results", path))
}

/**
    Extract multiple matches using CSS selectors, returning an array of objects.
*/
fn extract_css_array(extractor: &Extractor, content: &str) -> Result<ExtractedArray> {
    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("css_array extractor requires 'path'"))?;

    let each = extractor
        .each
        .as_ref()
        .ok_or_else(|| anyhow!("css_array extractor requires 'each'"))?;

    if path.contains("::") {
        return Err(anyhow!(
            "css_array path '{}' should be a selector without ::text/::attr",
            path
        ));
    }

    let selector =
        Selector::parse(path).map_err(|e| anyhow!("Invalid CSS selector '{}': {:?}", path, e))?;

    let document = Html::parse_document(content);
    let mut extracted = Vec::new();

    for element in document.select(&selector) {
        let mut fields: HashMap<String, Option<String>> = HashMap::new();

        for (field_name, selector_ref) in each {
            let mut value: Option<String> = None;

            for candidate in selector_ref.split('|').map(|s| s.trim()) {
                if candidate.is_empty() {
                    continue;
                }

                if let Some(const_value) = candidate.strip_prefix("const:") {
                    value = Some(const_value.to_string());
                    break;
                }

                let extracted_value = extract_css_value_from_node(candidate, element);
                if extracted_value.is_some() {
                    value = extracted_value;
                    break;
                }
            }

            let value = if extractor.unescape {
                value.map(|v| unescape_html_string(&v))
            } else {
                value
            };

            fields.insert(field_name.clone(), value);
        }

        if should_skip_item(&fields) {
            continue;
        }

        extracted.push(fields);
    }

    if extracted.is_empty() {
        return Err(anyhow!("CSS selector '{}' returned no results", path));
    }

    Ok(extracted)
}

fn parse_css_path(path: &str) -> Result<(String, CssTarget)> {
    if let Some(idx) = path.rfind("::attr(") {
        let (selector, rest) = path.split_at(idx);
        let attr_part = rest.trim_start_matches("::attr(");
        let attr_name = attr_part
            .strip_suffix(')')
            .ok_or_else(|| anyhow!("CSS attr selector '{}' missing ')'", path))?;
        return Ok((
            selector.trim().to_string(),
            CssTarget::Attr(attr_name.to_string()),
        ));
    }

    if path.ends_with("::text()") {
        let selector = path.trim_end_matches("::text()").trim();
        return Ok((selector.to_string(), CssTarget::Text));
    }

    if path.ends_with("::text") {
        let selector = path.trim_end_matches("::text").trim();
        return Ok((selector.to_string(), CssTarget::Text));
    }

    Ok((path.trim().to_string(), CssTarget::Text))
}

fn extract_css_value_from_node(path: &str, node: ElementRef) -> Option<String> {
    let (selector, target) = parse_css_path(path).ok()?;
    let selector = selector.trim();

    if selector.is_empty() {
        return extract_css_value_from_element(node, &target);
    }

    let selector = Selector::parse(selector).ok()?;

    if let Some(element) = node.select(&selector).next() {
        return extract_css_value_from_element(element, &target);
    }

    if node.value().name() == "template" {
        let fragment = Html::parse_fragment(&node.inner_html());
        if let Some(element) = fragment.select(&selector).next() {
            return extract_css_value_from_element(element, &target);
        }
    }

    None
}

fn extract_css_value_from_element(element: ElementRef, target: &CssTarget) -> Option<String> {
    match target {
        CssTarget::Text => {
            let text = element.text().collect::<Vec<_>>().join("");
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
        CssTarget::Attr(name) => element.value().attr(name).map(|s| s.to_string()),
    }
}

/**
    Extract using regex with capture group.
*/
fn extract_regex(extractor: &Extractor, content: &str) -> Result<String> {
    let pattern = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("Regex extractor requires 'path'"))?;

    let re = Regex::new(pattern).map_err(|e| anyhow!("Invalid regex '{}': {}", pattern, e))?;

    let captures = re
        .captures(content)
        .ok_or_else(|| anyhow!("Regex '{}' did not match", pattern))?;

    // Return first capture group, or fall back to later groups if empty
    if captures.len() > 1 {
        if let Some(m) = captures.get(1) {
            return Ok(m.as_str().to_string());
        }

        for idx in 2..captures.len() {
            if let Some(m) = captures.get(idx) {
                return Ok(m.as_str().to_string());
            }
        }

        if let Some(default) = &extractor.default {
            return Ok(default.clone());
        }
        return Err(anyhow!(
            "Regex '{}' matched but all capture groups were empty",
            pattern
        ));
    }

    if let Some(m) = captures.get(0) {
        return Ok(m.as_str().to_string());
    }

    if let Some(default) = &extractor.default {
        return Ok(default.clone());
    }

    Err(anyhow!("Regex '{}' matched but returned empty", pattern))
}

/**
    Extract multiple matches using regex, returning an array of objects.
*/
fn extract_regex_array(extractor: &Extractor, content: &str) -> Result<ExtractedArray> {
    let pattern = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("regex_array extractor requires 'path'"))?;

    let each = extractor
        .each
        .as_ref()
        .ok_or_else(|| anyhow!("regex_array extractor requires 'each'"))?;

    let re = Regex::new(pattern).map_err(|e| anyhow!("Invalid regex '{}': {}", pattern, e))?;

    let mut extracted = Vec::new();

    for captures in re.captures_iter(content) {
        let mut fields: HashMap<String, Option<String>> = HashMap::new();

        for (field_name, group_ref) in each {
            let mut value: Option<String> = None;

            for candidate in group_ref.split('|').map(|s| s.trim()) {
                if candidate.is_empty() {
                    continue;
                }

                if let Some(const_value) = candidate.strip_prefix("const:") {
                    value = Some(const_value.to_string());
                    break;
                }

                if let Ok(index) = candidate.parse::<usize>() {
                    if let Some(m) = captures.get(index) {
                        value = Some(m.as_str().to_string());
                        break;
                    }
                    continue;
                }

                if let Some(m) = captures.name(candidate) {
                    value = Some(m.as_str().to_string());
                    break;
                }
            }

            let value = if extractor.unescape {
                value.map(|v| unescape_html_string(&v))
            } else {
                value
            };

            fields.insert(field_name.clone(), value);
        }

        if should_skip_item(&fields) {
            continue;
        }

        extracted.push(fields);
    }

    if extracted.is_empty() {
        return Err(anyhow!("Regex '{}' returned no results", pattern));
    }

    Ok(extracted)
}

/**
    Decode common HTML entities and numeric character references.
*/
fn unescape_html_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }

        let mut entity = String::new();
        while let Some(&c) = chars.peek() {
            chars.next();
            if c == ';' {
                break;
            }
            entity.push(c);
            // Avoid runaway entity parsing
            if entity.len() > 12 {
                break;
            }
        }

        let decoded = match entity.as_str() {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "#39" | "#x27" => Some("'".to_string()),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string())
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .map(|c| c.to_string())
                } else {
                    None
                }
            }
        };

        if let Some(decoded) = decoded {
            out.push_str(&decoded);
        } else {
            out.push('&');
            out.push_str(&entity);
            out.push(';');
        }
    }

    out
}

/**
    Extract first line containing ":".
*/
fn extract_line(content: &str) -> Result<String> {
    content
        .lines()
        .find(|line| line.contains(':'))
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("No line containing ':' found"))
}

/**
    Extract Widevine PSSH from MPD manifest using ffmpeg-source DASH parser.
*/
fn extract_pssh(content: &str, url: &str) -> Result<String> {
    use ffmpeg_source::reader::stream::StreamFormat;
    use ffmpeg_source::reader::stream::dash::DashFormat;

    let dash = DashFormat::from_manifest(url, content.as_bytes())
        .map_err(|e| anyhow!("Failed to parse MPD: {}", e))?;

    let drm_info = dash.drm_info();

    // Get Widevine PSSH first, fall back to any PSSH
    let pssh = drm_info
        .widevine_pssh()
        .into_iter()
        .next()
        .map(|p| &p.data_base64)
        .or_else(|| drm_info.pssh_boxes.first().map(|p| &p.data_base64))
        .ok_or_else(|| anyhow!("No PSSH found in MPD"))?;

    Ok(pssh.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_url() {
        let extractor = Extractor {
            kind: ExtractorKind::Url,
            path: None,
            default: None,
            regex: None,
            each: None,
            unescape: false,
        };
        let result = extract(&extractor, "body content", "https://example.com/test.mpd").unwrap();
        assert_eq!(result, "https://example.com/test.mpd");
    }

    #[test]
    fn test_extract_line() {
        let extractor = Extractor {
            kind: ExtractorKind::Line,
            path: None,
            default: None,
            regex: None,
            each: None,
            unescape: false,
        };
        let content = "some header\nabc123:def456\nmore stuff";
        let result = extract(&extractor, content, "").unwrap();
        assert_eq!(result, "abc123:def456");
    }

    #[test]
    fn test_extract_regex() {
        let extractor = Extractor {
            kind: ExtractorKind::Regex,
            path: Some(r"id=(\d+)".to_string()),
            default: None,
            regex: None,
            each: None,
            unescape: false,
        };
        let result = extract(&extractor, "content?id=12345&other=value", "").unwrap();
        assert_eq!(result, "12345");
    }

    #[test]
    fn test_unescape_json_string() {
        // Test unicode escape sequences
        assert_eq!(
            unescape_json_string(r"https://example.com?a=1\u0026b=2"),
            "https://example.com?a=1&b=2"
        );

        // Test multiple escapes
        assert_eq!(
            unescape_json_string(r"foo\u0026bar\u0026baz"),
            "foo&bar&baz"
        );

        // Test doubly-escaped unicode (\\uXXXX -> character)
        // This is common in JSON-inside-JSON like TikTok's SIGI_STATE
        assert_eq!(
            unescape_json_string(r"https://example.com?a=1\\u0026b=2"),
            "https://example.com?a=1&b=2"
        );

        // Test other escape sequences
        assert_eq!(unescape_json_string(r"line1\nline2"), "line1\nline2");
        assert_eq!(unescape_json_string(r"tab\there"), "tab\there");
        assert_eq!(unescape_json_string(r#"quote\"here"#), "quote\"here");
        assert_eq!(unescape_json_string(r"back\\slash"), "back\\slash");

        // Test no escapes
        assert_eq!(
            unescape_json_string("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn test_extract_with_unescape() {
        let extractor = Extractor {
            kind: ExtractorKind::Regex,
            path: Some(r"url=(https://[^\s]+)".to_string()),
            default: None,
            regex: None,
            each: None,
            unescape: true,
        };
        let content = r"url=https://example.com?a=1\u0026b=2";
        let result = extract(&extractor, content, "").unwrap();
        assert_eq!(result, "https://example.com?a=1&b=2");
    }

    #[test]
    fn test_extract_jsonpath_array() {
        let mut each = HashMap::new();
        each.insert("id".to_string(), "$.id".to_string());
        each.insert("name".to_string(), "$.title".to_string());
        each.insert("image".to_string(), "$.thumbnail".to_string());

        let extractor = Extractor {
            kind: ExtractorKind::JsonPathArray,
            path: Some("$.items[*]".to_string()),
            default: None,
            regex: None,
            each: Some(each),
            unescape: false,
        };

        let content = r#"{
            "items": [
                {"id": "123", "title": "Channel One", "thumbnail": "http://img1.jpg"},
                {"id": "456", "title": "Channel Two"},
                {"title": "No ID Channel"}
            ]
        }"#;

        let result = extract_array(&extractor, content).unwrap();

        // Should have 2 results (the one without id is skipped)
        assert_eq!(result.len(), 2);

        // First channel
        assert_eq!(result[0].get("id").unwrap(), &Some("123".to_string()));
        assert_eq!(
            result[0].get("name").unwrap(),
            &Some("Channel One".to_string())
        );
        assert_eq!(
            result[0].get("image").unwrap(),
            &Some("http://img1.jpg".to_string())
        );

        // Second channel (no image)
        assert_eq!(result[1].get("id").unwrap(), &Some("456".to_string()));
        assert_eq!(
            result[1].get("name").unwrap(),
            &Some("Channel Two".to_string())
        );
        assert_eq!(result[1].get("image").unwrap(), &None);
    }

    #[test]
    fn test_extract_nested_with_parent() {
        let mut each = HashMap::new();
        each.insert("channel_id".to_string(), "$parent.id".to_string());
        each.insert("title".to_string(), "$.title".to_string());
        each.insert("start_time".to_string(), "$.startTime".to_string());

        let extractor = Extractor {
            kind: ExtractorKind::JsonPathArray,
            path: Some("$.result[*].content.epg[*]".to_string()),
            default: None,
            regex: None,
            each: Some(each),
            unescape: false,
        };

        let content = r#"{
            "result": [
                {
                    "id": "channel1",
                    "content": {
                        "epg": [
                            {"title": "Show A", "startTime": "2026-01-01T00:00:00Z"},
                            {"title": "Show B", "startTime": "2026-01-01T01:00:00Z"}
                        ]
                    }
                },
                {
                    "id": "channel2",
                    "content": {
                        "epg": [
                            {"title": "Show C", "startTime": "2026-01-01T00:00:00Z"}
                        ]
                    }
                }
            ]
        }"#;

        let result = extract_array(&extractor, content).unwrap();

        // Should have 3 programmes total
        assert_eq!(result.len(), 3);

        // First programme from channel1
        assert_eq!(
            result[0].get("channel_id").unwrap(),
            &Some("channel1".to_string())
        );
        assert_eq!(result[0].get("title").unwrap(), &Some("Show A".to_string()));

        // Second programme from channel1
        assert_eq!(
            result[1].get("channel_id").unwrap(),
            &Some("channel1".to_string())
        );
        assert_eq!(result[1].get("title").unwrap(), &Some("Show B".to_string()));

        // Programme from channel2
        assert_eq!(
            result[2].get("channel_id").unwrap(),
            &Some("channel2".to_string())
        );
        assert_eq!(result[2].get("title").unwrap(), &Some("Show C".to_string()));
    }

    #[test]
    fn test_split_nested_path() {
        let (parent, child) = split_nested_path("$.result[*].content.epg[*]").unwrap();
        assert_eq!(parent, "$.result[*]");
        assert_eq!(child, "$.content.epg[*]");
    }
}
