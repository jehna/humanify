use anyhow::anyhow;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(600))
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        let inner = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client init failed");
        Self { inner }
    }

    pub async fn post_json(
        &self,
        url: &str,
        api_key: Option<&str>,
        extra_headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<Value, StrategyError> {
        let mut request = self
            .inner
            .post(url)
            .header(
                "User-Agent",
                concat!("humanify/", env!("CARGO_PKG_VERSION")),
            )
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(key) = api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }

        request = request.json(body);

        let response = request
            .send()
            .await
            .map_err(|e| StrategyError::Transient(anyhow!(e)))?;

        let status = response.status().as_u16();
        let body_text = response
            .text()
            .await
            .map_err(|e| StrategyError::Transient(anyhow!(e)))?;

        if (200..300).contains(&status) {
            // A 2xx whose body isn't JSON is a provider/model behaviour problem,
            // not a network blip: re-sending the identical request would produce
            // the same garbage. Permanent, so the retry loop doesn't multiply it.
            let value: Value = serde_json::from_str(&body_text).map_err(|e| {
                StrategyError::Permanent(anyhow!("response was not valid JSON: {e}"))
            })?;
            Ok(value)
        } else {
            Err(classify_error(status, &body_text))
        }
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

pub enum StrategyError {
    /// Provider rejected this strategy permanently for this endpoint/model combo.
    /// The `Ladder` falls through to the next strategy and marks this one dead.
    NotSupported(String),
    /// Network failure / 408 / 429 / 5xx — the identical request may well succeed
    /// if sent again, so `LlmRenamer` retries these with backoff.
    Transient(anyhow::Error),
    /// The request itself is wrong or the provider's answer is unusable: bad
    /// credentials (401/403), invalid model or malformed request (other 4xx), a
    /// response humanify cannot parse, or a ladder with no strategy left. Sending
    /// the identical request again cannot help, so it is **never** retried —
    /// otherwise a single bad API key would cost `max_retries + 1` requests and
    /// several seconds of backoff for every identifier in the file.
    Permanent(anyhow::Error),
}

impl StrategyError {
    pub fn is_not_supported(&self) -> bool {
        matches!(self, StrategyError::NotSupported(_))
    }

    /// True only for errors worth retrying. This is the retry predicate: adding a
    /// new error kind means deciding, here, whether re-sending can help.
    pub fn is_transient(&self) -> bool {
        matches!(self, StrategyError::Transient(_))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, StrategyError::Permanent(_))
    }
}

impl std::fmt::Display for StrategyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyError::NotSupported(msg) => write!(f, "strategy not supported: {msg}"),
            StrategyError::Transient(e) | StrategyError::Permanent(e) => write!(f, "{e}"),
        }
    }
}

impl std::fmt::Debug for StrategyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyError::NotSupported(msg) => f.debug_tuple("NotSupported").field(msg).finish(),
            StrategyError::Transient(e) => f.debug_tuple("Transient").field(e).finish(),
            StrategyError::Permanent(e) => f.debug_tuple("Permanent").field(e).finish(),
        }
    }
}

impl std::error::Error for StrategyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StrategyError::Transient(e) | StrategyError::Permanent(e) => e.source(),
            StrategyError::NotSupported(_) => None,
        }
    }
}

/// Extract (message, code, param) from either OpenAI or Anthropic error JSON shape.
/// Returns empty strings for missing fields. Returns None if body isn't JSON.
fn extract_error_fields(body: &str) -> Option<(String, String, String)> {
    let v: Value = serde_json::from_str(body).ok()?;

    // OpenAI shape: {"error": {"message": ..., "code": ..., "param": ...}}
    // Anthropic shape: {"type": "error", "error": {"type": ..., "message": ...}}
    let error_obj = v.get("error")?;

    let message = error_obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let code = error_obj
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Anthropic uses "type" as a code-like field
    let param = error_obj
        .get("param")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Some((message, code, param))
}

fn not_supported(status: u16, message: &str) -> StrategyError {
    StrategyError::NotSupported(format!("http {status}: {message}"))
}

pub fn classify_error(status: u16, body: &str) -> StrategyError {
    // Rules 1-4: status-based early returns
    if status >= 500 {
        return StrategyError::Transient(anyhow!("http {status}: {body}"));
    }
    if status == 429 {
        return StrategyError::Transient(anyhow!("http 429: rate limited"));
    }
    if status == 408 {
        return StrategyError::Transient(anyhow!("http 408: request timeout"));
    }
    // Credentials don't fix themselves between attempts.
    if status == 401 || status == 403 {
        return StrategyError::Permanent(anyhow!("http {status}: {body}"));
    }

    // Rule 5: 4xx — inspect body
    if (400..500).contains(&status) {
        let fields = extract_error_fields(body);
        let (message, code, param) = fields
            .as_ref()
            .map(|(m, c, p)| (m.as_str(), c.as_str(), p.as_str()))
            .unwrap_or(("", "", ""));

        // Param-name shortcut (most reliable)
        if param.eq_ignore_ascii_case("response_format")
            || param.eq_ignore_ascii_case("tool_choice")
        {
            return not_supported(status, message);
        }

        // Check all text fields together for substring rules
        let combined = format!("{message} {code} {param}").to_lowercase();
        let body_lower = body.to_lowercase();

        // Check combined parsed fields first, fall back to raw body for plain-text responses
        let search = if fields.is_some() {
            &combined
        } else {
            body_lower.as_str()
        };

        if search.contains("unsupported parameter") {
            return not_supported(status, message);
        }
        if search.contains("unrecognized request argument") {
            return not_supported(status, message);
        }
        if search.contains("unknown parameter") {
            return not_supported(status, message);
        }
        if search.contains("invalid value: response_format")
            || search.contains("invalid value for response_format")
        {
            return not_supported(status, message);
        }
        if search.contains("response_format") && search.contains("not supported") {
            return not_supported(status, message);
        }
        if search.contains("tool_choice") && search.contains("not supported") {
            return not_supported(status, message);
        }
        if search.contains("tool_choice") && search.contains("invalid") {
            return not_supported(status, message);
        }
        if search.contains("json_schema") && search.contains("not supported") {
            return not_supported(status, message);
        }
        if search.contains("structured outputs") && search.contains("not supported") {
            return not_supported(status, message);
        }
        if search.contains("missing required header") && search.contains("anthropic-beta") {
            return not_supported(status, message);
        }
        // "unsupported" + specific strategy param reference
        if search.contains("unsupported")
            && (search.contains("response_format")
                || search.contains("tool_choice")
                || search.contains("tool_call")
                || search.contains("json_schema")
                || search.contains("structured outputs"))
        {
            return not_supported(status, message);
        }

        // Unknown 4xx (invalid model, malformed request, content policy, 404 …).
        // The server understood us and said no; asking again says no again.
        return StrategyError::Permanent(anyhow!("http {status}: {body}"));
    }

    // Rule 6: anything else (3xx, weird codes) — not a retry candidate either.
    StrategyError::Permanent(anyhow!("http {status}: {body}"))
}

#[cfg(test)]
mod tests {
    // DSL not applied: classify_error tests are already ≤ 3 lines each
    // (assert_transient / assert_not_supported + one call). A DSL wrapper
    // would add indirection without adding clarity.
    use super::*;

    fn not_supported_reason(status: u16, body: &str) -> String {
        match classify_error(status, body) {
            StrategyError::NotSupported(r) => r,
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    /// Retryable: a later attempt could plausibly succeed.
    fn assert_transient(status: u16, body: &str) {
        match classify_error(status, body) {
            StrategyError::Transient(_) => {}
            other => panic!("expected Transient, got {other:?}"),
        }
    }

    /// Not retryable: the same request would fail the same way.
    fn assert_permanent(status: u16, body: &str) {
        match classify_error(status, body) {
            StrategyError::Permanent(_) => {}
            other => panic!("expected Permanent, got {other:?}"),
        }
    }

    fn assert_not_supported(status: u16, body: &str) {
        match classify_error(status, body) {
            StrategyError::NotSupported(_) => {}
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[test]
    fn status_500_is_transient() {
        assert_transient(500, "server error");
    }

    #[test]
    fn status_502_is_transient() {
        assert_transient(502, "bad gateway");
    }

    #[test]
    fn status_429_is_transient() {
        assert_transient(429, "rate limited");
    }

    #[test]
    fn status_408_is_transient() {
        assert_transient(408, "timeout");
    }

    // 401/403 and unknown 4xx are Permanent, not Transient: retrying a bad key or
    // a bad model name burns `max_retries + 1` requests per identifier and cannot
    // succeed. See StrategyError::Permanent.
    #[test]
    fn status_401_is_permanent() {
        assert_permanent(401, r#"{"error": {"message": "invalid api key"}}"#);
    }

    #[test]
    fn status_403_is_permanent() {
        assert_permanent(403, r#"{"error": {"message": "forbidden"}}"#);
    }

    #[test]
    fn status_400_unsupported_parameter_response_format() {
        let body = r#"{"error":{"message":"Unsupported parameter: 'response_format'.","param":"response_format","type":"invalid_request_error"}}"#;
        let reason = not_supported_reason(400, body);
        assert!(
            reason.contains("response_format"),
            "reason should mention response_format: {reason}"
        );
    }

    #[test]
    fn status_400_unsupported_tool_choice() {
        let body =
            r#"{"error":{"message":"Unsupported value: 'tool_choice'.","param":"tool_choice"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_unrecognized_request_argument_anthropic() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Unrecognized request argument: response_format"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_unknown_parameter() {
        let body = r#"{"error":{"message":"Unknown parameter: response_format"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_invalid_value_response_format() {
        let body = r#"{"error":{"message":"Invalid value: response_format json_schema not supported on this model"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_response_format_not_supported_combo() {
        let body =
            r#"{"error":{"message":"response_format json_schema is not supported by this model"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_tool_choice_not_supported() {
        let body = r#"{"error":{"message":"tool_choice required is not supported"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_json_schema_not_supported() {
        let body = r#"{"error":{"message":"json_schema mode not supported"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_structured_outputs_not_supported_anthropic() {
        let body = r#"{"type":"error","error":{"message":"Structured outputs are not supported for this model"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_missing_anthropic_beta_header() {
        let body = r#"{"type":"error","error":{"message":"Missing required header: anthropic-beta for structured outputs"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_param_name_response_format_only() {
        let body =
            r#"{"error":{"param":"response_format","message":"some other phrasing entirely"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn status_400_invalid_model_name_is_permanent() {
        let body = r#"{"error":{"message":"The model 'gpt-9000' does not exist"}}"#;
        assert_permanent(400, body);
    }

    #[test]
    fn status_400_content_policy_is_permanent() {
        let body = r#"{"error":{"message":"Your request was rejected by content moderation"}}"#;
        assert_permanent(400, body);
    }

    #[test]
    fn status_400_malformed_prompt_is_permanent() {
        let body = r#"{"error":{"message":"Invalid prompt format"}}"#;
        assert_permanent(400, body);
    }

    #[test]
    fn status_400_plain_text_body_is_permanent() {
        assert_permanent(400, "Bad Request");
    }

    #[test]
    fn status_400_non_json_body_is_permanent() {
        assert_permanent(400, "<html>...</html>");
    }

    #[test]
    fn status_400_malformed_json_body_is_permanent() {
        assert_permanent(400, "{not valid json");
    }

    #[test]
    fn status_404_is_permanent() {
        assert_permanent(404, r#"{"error":{"message":"not found"}}"#);
    }

    #[test]
    fn status_300_is_permanent() {
        assert_permanent(300, "");
    }

    #[test]
    fn case_insensitive_match() {
        let body = r#"{"error":{"message":"UNSUPPORTED PARAMETER: response_format"}}"#;
        assert_not_supported(400, body);
    }

    #[test]
    fn reason_string_includes_status_and_message() {
        let body = r#"{"error":{"message":"Unsupported parameter: foo"}}"#;
        let reason = not_supported_reason(400, body);
        assert!(
            reason.starts_with("http 400"),
            "reason should start with 'http 400': {reason}"
        );
        assert!(
            reason.contains("Unsupported parameter: foo"),
            "reason should contain message: {reason}"
        );
    }
}
