use anyhow::{Result, anyhow};
use regex::Regex;

use super::types::{Extractor, ExtractorKind};

/**
    Run an extractor on the given content.
*/
pub fn extract(extractor: &Extractor, content: &str, url: &str) -> Result<String> {
    match extractor.kind {
        ExtractorKind::Url => Ok(url.to_string()),
        ExtractorKind::UrlRegex => extract_regex(extractor, url),
        ExtractorKind::JsonPath => extract_jsonpath(extractor, content),
        ExtractorKind::XPath => extract_xpath(extractor, content),
        ExtractorKind::Regex => extract_regex(extractor, content),
        ExtractorKind::Line => extract_line(content),
        ExtractorKind::Pssh => extract_pssh(content, url),
    }
}

/**
    Extract using JSONPath.
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
    Extract using XPath.
*/
fn extract_xpath(extractor: &Extractor, content: &str) -> Result<String> {
    let path = extractor
        .path
        .as_ref()
        .ok_or_else(|| anyhow!("XPath extractor requires 'path'"))?;

    let package = sxd_document::parser::parse(content)
        .map_err(|e| anyhow!("Failed to parse XML: {:?}", e))?;
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

    // Return first capture group, or whole match if no groups
    if captures.len() > 1 {
        Ok(captures.get(1).unwrap().as_str().to_string())
    } else {
        Ok(captures.get(0).unwrap().as_str().to_string())
    }
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
        };
        let result = extract(&extractor, "body content", "https://example.com/test.mpd").unwrap();
        assert_eq!(result, "https://example.com/test.mpd");
    }

    #[test]
    fn test_extract_line() {
        let extractor = Extractor {
            kind: ExtractorKind::Line,
            path: None,
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
        };
        let result = extract(&extractor, "content?id=12345&other=value", "").unwrap();
        assert_eq!(result, "12345");
    }
}
