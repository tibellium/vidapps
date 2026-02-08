use std::collections::HashMap;

use anyhow::{Result, anyhow};
use regex::Regex;

/**
    Context for variable interpolation, storing outputs from each step.
*/
#[derive(Debug, Default)]
pub struct InterpolationContext {
    steps: HashMap<String, HashMap<String, String>>,
}

impl InterpolationContext {
    pub fn new() -> Self {
        Self::default()
    }

    /**
        Add an output value for a step.
    */
    pub fn set(&mut self, step_name: &str, output_name: &str, value: String) {
        self.steps
            .entry(step_name.to_string())
            .or_default()
            .insert(output_name.to_string(), value);
    }

    /**
        Get an output value from a step.
    */
    pub fn get(&self, step_name: &str, output_name: &str) -> Option<&String> {
        self.steps.get(step_name)?.get(output_name)
    }

    /**
        Interpolate a string, replacing `${{step_name.output_name}}` with values.
    */
    pub fn interpolate(&self, template: &str) -> Result<String> {
        let re = Regex::new(r"\$\{\{([a-zA-Z_][a-zA-Z0-9_]*)\.([a-zA-Z_][a-zA-Z0-9_]*)\}\}")?;

        let mut result = template.to_string();
        let mut last_err: Option<anyhow::Error> = None;

        for cap in re.captures_iter(template) {
            let full_match = cap.get(0).unwrap().as_str();
            let step_name = &cap[1];
            let output_name = &cap[2];

            match self.get(step_name, output_name) {
                Some(value) => {
                    result = result.replace(full_match, value);
                }
                None => {
                    last_err = Some(anyhow!("Undefined variable: {}.{}", step_name, output_name));
                }
            }
        }

        if let Some(err) = last_err {
            return Err(err);
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolation() {
        let mut ctx = InterpolationContext::new();
        ctx.set("find_content", "content_id", "abc123".to_string());
        ctx.set(
            "get_manifest",
            "mpd_url",
            "https://example.com/stream.mpd".to_string(),
        );

        let result = ctx
            .interpolate("https://example.com/player/${{find_content.content_id}}")
            .unwrap();
        assert_eq!(result, "https://example.com/player/abc123");

        let result = ctx.interpolate("${{get_manifest.mpd_url}}").unwrap();
        assert_eq!(result, "https://example.com/stream.mpd");
    }

    #[test]
    fn test_undefined_variable() {
        let ctx = InterpolationContext::new();
        let result = ctx.interpolate("${{missing.value}}");
        assert!(result.is_err());
    }

    #[test]
    fn test_no_placeholders() {
        let ctx = InterpolationContext::new();
        let result = ctx.interpolate("plain string").unwrap();
        assert_eq!(result, "plain string");
    }
}
