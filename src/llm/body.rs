use serde_json::{json, Map, Value};
use std::fs;

/// Keys humanify owns. Letting `--extra-body` set them would either destroy the
/// prompt (`messages`, and `system` on the Anthropic endpoint) or hand back a
/// body the response parsers cannot read (`stream`, which switches the provider
/// to SSE frames).
const RESERVED_KEYS: [&str; 3] = ["messages", "stream", "system"];

/// Provider-agnostic request-body knobs, merged into every chat/completions
/// body just before it is sent.
///
/// This exists because request-shaping parameters are provider-specific and
/// move faster than humanify can track: GLM wants `"thinking":{"type":
/// "disabled"}` to stop a reasoning model from spending minutes of chain of
/// thought on a one-word answer, other providers spell the same idea
/// differently. Rather than grow a flag per provider, the raw JSON is passed
/// through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyOptions {
    /// Sent as `max_tokens` when set. Left unset by default on OpenAI-compatible
    /// providers: reasoning tokens count against this budget, so a cap that is
    /// comfortable for the answer alone can truncate a thinking model
    /// mid-reasoning and yield empty content — a hard failure, worse than a slow
    /// success. Set it once thinking is off.
    pub max_tokens: Option<u32>,
    /// Top-level keys merged into the body, overriding anything humanify set
    /// (including `max_tokens`).
    pub extra: Map<String, Value>,
}

impl BodyOptions {
    pub fn is_empty(&self) -> bool {
        self.max_tokens.is_none() && self.extra.is_empty()
    }

    /// Shallow-merges the options into `body`. Top-level keys only: an `extra`
    /// value replaces humanify's key outright rather than being merged into it,
    /// so `{"response_format":{...}}` substitutes the whole object.
    pub fn apply(&self, body: &mut Value) {
        let Some(obj) = body.as_object_mut() else {
            return;
        };

        if let Some(n) = self.max_tokens {
            obj.insert("max_tokens".to_string(), json!(n));
        }

        // Last writer wins, so an explicit `--extra-body max_tokens` beats
        // `--max-tokens`.
        for (key, value) in &self.extra {
            obj.insert(key.clone(), value.clone());
        }
    }

    /// One-line rendering for `--verbose` config output.
    pub fn extra_display(&self) -> String {
        Value::Object(self.extra.clone()).to_string()
    }
}

/// Parses the `--extra-body` argument: either inline JSON, or `@path` to read
/// the JSON from a file (inline JSON is painful to quote in some shells).
///
/// Returns a user-facing message on failure — every error here is a CLI usage
/// error, reported before any request is sent.
pub fn parse_extra_body(arg: &str) -> Result<Map<String, Value>, String> {
    let text = match arg.strip_prefix('@') {
        Some(path) => fs::read_to_string(path)
            .map_err(|e| format!("--extra-body: cannot read '{path}': {e}"))?,
        None => arg.to_string(),
    };

    let value: Value = serde_json::from_str(text.trim())
        .map_err(|e| format!("--extra-body: not valid JSON: {e}"))?;

    let obj = match value {
        Value::Object(map) => map,
        other => {
            return Err(format!(
                "--extra-body: expected a JSON object like '{{\"thinking\":{{\"type\":\"disabled\"}}}}', got {}",
                json_type_name(&other)
            ))
        }
    };

    for key in obj.keys() {
        if RESERVED_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "--extra-body: `{key}` is built by humanify and cannot be overridden"
            ));
        }
    }

    Ok(obj)
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(max_tokens: Option<u32>, extra: &str) -> BodyOptions {
        BodyOptions {
            max_tokens,
            extra: parse_extra_body(extra).expect("test fixture should parse"),
        }
    }

    fn parse_err(arg: &str) -> String {
        parse_extra_body(arg).expect_err("expected a parse error")
    }

    // --- parse_extra_body ---

    #[test]
    fn parses_object() {
        let map = parse_extra_body(r#"{"thinking":{"type":"disabled"}}"#).unwrap();
        assert_eq!(map["thinking"], json!({"type": "disabled"}));
    }

    #[test]
    fn parses_empty_object() {
        assert!(parse_extra_body("{}").unwrap().is_empty());
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert!(parse_extra_body("  {\"temperature\":0}\n").is_ok());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_err("{not json").contains("not valid JSON"));
    }

    #[test]
    fn rejects_non_object_json() {
        assert!(parse_err("[1,2]").contains("an array"));
        assert!(parse_err("\"hi\"").contains("a string"));
    }

    #[test]
    fn rejects_reserved_messages_key() {
        assert!(parse_err(r#"{"messages":[]}"#).contains("messages"));
    }

    #[test]
    fn rejects_reserved_stream_key() {
        assert!(parse_err(r#"{"stream":true}"#).contains("stream"));
    }

    #[test]
    fn rejects_reserved_system_key() {
        assert!(parse_err(r#"{"system":"be brief"}"#).contains("system"));
    }

    #[test]
    fn reads_from_at_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extra.json");
        fs::write(&path, r#"{"temperature":0}"#).unwrap();
        let map = parse_extra_body(&format!("@{}", path.display())).unwrap();
        assert_eq!(map["temperature"], json!(0));
    }

    #[test]
    fn missing_at_path_is_an_error() {
        let msg = parse_err("@no/such/file.json");
        assert!(msg.contains("cannot read"), "{msg}");
    }

    // --- apply ---

    #[test]
    fn default_options_change_nothing() {
        let mut body = json!({"model": "m", "messages": []});
        let before = body.clone();
        BodyOptions::default().apply(&mut body);
        assert_eq!(body, before);
    }

    #[test]
    fn max_tokens_is_inserted() {
        let mut body = json!({"model": "m"});
        opts(Some(64), "{}").apply(&mut body);
        assert_eq!(body["max_tokens"], json!(64));
    }

    #[test]
    fn extra_keys_are_merged() {
        let mut body = json!({"model": "m"});
        opts(None, r#"{"thinking":{"type":"disabled"},"temperature":0}"#).apply(&mut body);
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
        assert_eq!(body["temperature"], json!(0));
        assert_eq!(body["model"], json!("m"), "existing keys are preserved");
    }

    #[test]
    fn extra_overrides_existing_key() {
        let mut body = json!({"model": "m", "max_tokens": 4096});
        opts(None, r#"{"max_tokens":128}"#).apply(&mut body);
        assert_eq!(body["max_tokens"], json!(128));
    }

    #[test]
    fn extra_wins_over_max_tokens_flag() {
        let mut body = json!({"model": "m"});
        opts(Some(64), r#"{"max_tokens":128}"#).apply(&mut body);
        assert_eq!(body["max_tokens"], json!(128));
    }

    #[test]
    fn non_object_body_is_left_alone() {
        let mut body = json!("not an object");
        opts(Some(64), r#"{"temperature":0}"#).apply(&mut body);
        assert_eq!(body, json!("not an object"));
    }

    #[test]
    fn is_empty_tracks_both_fields() {
        assert!(BodyOptions::default().is_empty());
        assert!(!opts(Some(1), "{}").is_empty());
        assert!(!opts(None, r#"{"temperature":0}"#).is_empty());
    }
}
