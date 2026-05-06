use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::llm::{
    http::{HttpClient, StrategyError},
    JsonStrategy,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_BETA_STRUCTURED_OUTPUTS: &str = "structured-outputs-2025-11-13";
const MAX_TOKENS: u32 = 4096; // TODO(post-e2e): consider tuning per-call

const TOOL_NAME: &str = "humanify_response";
const TOOL_DESCRIPTION: &str =
    "Submit the response payload conforming to the provided JSON schema.";
const ANTHROPIC_TOOL_NUDGE: &str = "\n\nIMPORTANT: respond by calling the `humanify_response` tool with input matching the provided JSON schema. Do not respond with plain text.";

fn join_messages_endpoint(base: &str) -> String {
    format!("{}/messages", base.trim_end_matches('/'))
}

/// Build Anthropic auth headers. Anthropic uses x-api-key, not Authorization: Bearer.
/// Returns owned Vec because the lifetime dance with Optional fields is otherwise awkward.
fn anthropic_headers(api_key: Option<&str>) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(key) = api_key {
        headers.push(("x-api-key".to_string(), key.to_string()));
    }
    headers.push((
        "anthropic-version".to_string(),
        ANTHROPIC_VERSION.to_string(),
    ));
    headers
}

fn anthropic_headers_with_beta(api_key: Option<&str>) -> Vec<(String, String)> {
    let mut headers = anthropic_headers(api_key);
    headers.push((
        "anthropic-beta".to_string(),
        ANTHROPIC_BETA_STRUCTURED_OUTPUTS.to_string(),
    ));
    headers
}

// --- AnthropicNativeJsonSchema ---

pub struct AnthropicNativeJsonSchema {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl AnthropicNativeJsonSchema {
    pub fn new(
        client: HttpClient,
        base_url: String,
        api_key: Option<String>,
        model: String,
    ) -> Self {
        Self {
            client,
            base_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl JsonStrategy for AnthropicNativeJsonSchema {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
            "output_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "humanify_response",
                    "schema": schema,
                    "strict": true
                }
            }
        });

        let headers = anthropic_headers_with_beta(self.api_key.as_deref());
        let headers_ref: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let response = self
            .client
            .post_json(
                &join_messages_endpoint(&self.base_url),
                None, // x-api-key goes in extra_headers
                &headers_ref,
                &body,
            )
            .await?;

        extract_anthropic_native_json(&response)
    }

    // Note: "anthropic-native" matches plan §4 --json-mode value.
    // "tool-call-and-prompt" in OpenAI context means openai_compat::ToolCallAndPrompt;
    // Anthropic's variant uses a separate name below.
    fn name(&self) -> &'static str {
        "anthropic-native"
    }
}

// --- AnthropicToolCallAndPrompt ---

pub struct AnthropicToolCallAndPrompt {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
}

impl AnthropicToolCallAndPrompt {
    pub fn new(
        client: HttpClient,
        base_url: String,
        api_key: Option<String>,
        model: String,
    ) -> Self {
        Self {
            client,
            base_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl JsonStrategy for AnthropicToolCallAndPrompt {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let augmented_system = format!("{system}{ANTHROPIC_TOOL_NUDGE}");

        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": augmented_system,
            "messages": [{ "role": "user", "content": user }],
            "tools": [{
                "name": TOOL_NAME,
                "description": TOOL_DESCRIPTION,
                "input_schema": schema
            }],
            "tool_choice": { "type": "tool", "name": TOOL_NAME }
        });

        let headers = anthropic_headers(self.api_key.as_deref());
        let headers_ref: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let response = self
            .client
            .post_json(
                &join_messages_endpoint(&self.base_url),
                None, // x-api-key goes in extra_headers
                &headers_ref,
                &body,
            )
            .await?;

        extract_anthropic_tool_input(&response)
    }

    // Note: "anthropic-tool-call-and-prompt" is an Anthropic-specific fallback strategy.
    // The --json-mode flag value "tool-call-and-prompt" refers to openai_compat::ToolCallAndPrompt.
    fn name(&self) -> &'static str {
        "anthropic-tool-call-and-prompt"
    }
}

// --- Response extraction helpers ---

// TODO(e2e): Anthropic's exact response shape for output_format=json_schema needs
// verification against a live response. The block type and key name are best-guess.
fn extract_anthropic_native_json(response: &Value) -> Result<Value, StrategyError> {
    let content = response
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Transient(anyhow!("AnthropicNativeJsonSchema: no content array"))
        })?;

    if content.is_empty() {
        return Err(StrategyError::Transient(anyhow!(
            "AnthropicNativeJsonSchema: content array was empty"
        )));
    }

    // Find first block with a JSON-output type (be permissive — Anthropic may use any of these).
    let json_block = content.iter().find(|block| {
        matches!(
            block.get("type").and_then(|t| t.as_str()),
            Some("json") | Some("json_schema") | Some("output_json")
        )
    });

    let block = json_block.ok_or_else(|| {
        StrategyError::Transient(anyhow!(
            "AnthropicNativeJsonSchema: no JSON block in content"
        ))
    })?;

    block.get("json").cloned().ok_or_else(|| {
        StrategyError::Transient(anyhow!(
            "AnthropicNativeJsonSchema: JSON block had no 'json' field"
        ))
    })
}

fn extract_anthropic_tool_input(response: &Value) -> Result<Value, StrategyError> {
    let content = response
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Transient(anyhow!("AnthropicToolCallAndPrompt: no content array"))
        })?;

    if content.is_empty() {
        return Err(StrategyError::Transient(anyhow!(
            "AnthropicToolCallAndPrompt: content array was empty"
        )));
    }

    // Skip text/thinking blocks; find the first tool_use block.
    let tool_block = content
        .iter()
        .find(|block| block.get("type").and_then(|t| t.as_str()) == Some("tool_use"));

    let block = tool_block.ok_or_else(|| {
        StrategyError::Transient(anyhow!(
            "AnthropicToolCallAndPrompt: no tool_use block in content"
        ))
    })?;

    // Verify the model called the right tool.
    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
    if name != TOOL_NAME {
        return Err(StrategyError::Transient(anyhow!(
            "AnthropicToolCallAndPrompt: model called unexpected tool '{name}'"
        )));
    }

    // input is already a parsed JSON object (not a string, unlike OpenAI).
    // It's not expected to be a string in practice, but we return whatever shape
    // Anthropic gave us and let the caller validate.
    block.get("input").cloned().ok_or_else(|| {
        StrategyError::Transient(anyhow!(
            "AnthropicToolCallAndPrompt: tool_use block had no 'input' field"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::llm::test_dsl::{extract_fails_with, extract_succeeds};

    // --- extract_anthropic_native_json ---

    #[test]
    fn native_happy_path_block_type_json() {
        extract_succeeds(
            extract_anthropic_native_json(&json!({"content":[{"type":"json","json":{"x":1}}]})),
            &json!({"x":1}),
        );
    }

    #[test]
    fn native_happy_path_with_text_first() {
        extract_succeeds(
            extract_anthropic_native_json(
                &json!({"content":[{"type":"text","text":"thinking..."},{"type":"json","json":{"x":1}}]}),
            ),
            &json!({"x":1}),
        );
    }

    #[test]
    fn native_block_type_json_schema() {
        extract_succeeds(
            extract_anthropic_native_json(
                &json!({"content":[{"type":"json_schema","json":{"x":1}}]}),
            ),
            &json!({"x":1}),
        );
    }

    #[test]
    fn native_block_type_output_json() {
        extract_succeeds(
            extract_anthropic_native_json(
                &json!({"content":[{"type":"output_json","json":{"x":1}}]}),
            ),
            &json!({"x":1}),
        );
    }

    #[test]
    fn native_no_json_block() {
        extract_fails_with(
            extract_anthropic_native_json(&json!({"content":[{"type":"text","text":"hello"}]})),
            "",
        );
    }

    #[test]
    fn native_content_empty() {
        extract_fails_with(extract_anthropic_native_json(&json!({"content":[]})), "");
    }

    #[test]
    fn native_content_missing() {
        extract_fails_with(extract_anthropic_native_json(&json!({})), "");
    }

    #[test]
    fn native_content_not_an_array() {
        extract_fails_with(
            extract_anthropic_native_json(&json!({"content":"hello"})),
            "",
        );
    }

    // --- extract_anthropic_tool_input ---

    #[test]
    fn tool_happy_path() {
        extract_succeeds(
            extract_anthropic_tool_input(
                &json!({"content":[{"type":"tool_use","name":"humanify_response","input":{"x":1}}]}),
            ),
            &json!({"x":1}),
        );
    }

    #[test]
    fn tool_text_then_tool_use() {
        extract_succeeds(
            extract_anthropic_tool_input(
                &json!({"content":[{"type":"text","text":"Let me think..."},{"type":"tool_use","name":"humanify_response","input":{"x":1}}]}),
            ),
            &json!({"x":1}),
        );
    }

    #[test]
    fn tool_wrong_tool_name() {
        extract_fails_with(
            extract_anthropic_tool_input(
                &json!({"content":[{"type":"tool_use","name":"something_else","input":{"x":1}}]}),
            ),
            "",
        );
    }

    #[test]
    fn tool_no_tool_use() {
        extract_fails_with(
            extract_anthropic_tool_input(&json!({"content":[{"type":"text","text":"hi"}]})),
            "",
        );
    }

    #[test]
    fn tool_content_empty() {
        extract_fails_with(extract_anthropic_tool_input(&json!({"content":[]})), "");
    }

    #[test]
    fn tool_content_missing() {
        extract_fails_with(extract_anthropic_tool_input(&json!({})), "");
    }

    #[test]
    fn tool_input_is_string_not_object() {
        // Not expected in practice, but we return whatever Anthropic gave us.
        extract_succeeds(
            extract_anthropic_tool_input(
                &json!({"content":[{"type":"tool_use","name":"humanify_response","input":"{\"x\":1}"}]}),
            ),
            &json!("{\"x\":1}"),
        );
    }

    #[test]
    fn tool_input_missing() {
        extract_fails_with(
            extract_anthropic_tool_input(
                &json!({"content":[{"type":"tool_use","name":"humanify_response"}]}),
            ),
            "",
        );
    }

    // --- anthropic_headers ---

    #[test]
    fn headers_with_api_key() {
        let h = anthropic_headers(Some("sk-ant-123"));
        assert!(
            h.iter().any(|(k, v)| k == "x-api-key" && v == "sk-ant-123"),
            "expected x-api-key: {h:?}"
        );
        assert!(
            h.iter()
                .any(|(k, v)| k == "anthropic-version" && v == ANTHROPIC_VERSION),
            "expected anthropic-version: {h:?}"
        );
    }

    #[test]
    fn headers_without_api_key() {
        let h = anthropic_headers(None);
        assert!(
            !h.iter().any(|(k, _)| k == "x-api-key"),
            "should have no x-api-key: {h:?}"
        );
        assert!(
            h.iter()
                .any(|(k, v)| k == "anthropic-version" && v == ANTHROPIC_VERSION),
            "expected anthropic-version: {h:?}"
        );
    }
}
