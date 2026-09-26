use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::llm::{
    body::BodyOptions,
    http::{HttpClient, StrategyError},
    JsonStrategy,
};

const TOOL_NAME: &str = "humanify_response";
const TOOL_DESCRIPTION: &str =
    "Submit the response payload conforming to the provided JSON schema.";
const TOOL_NUDGE: &str = "\n\nIMPORTANT: respond by calling the `humanify_response` function with arguments matching the provided JSON schema. Do not respond with plain text.";
const JSON_INSTRUCTION: &str = "\n\nYou MUST respond with a single JSON object that exactly matches this schema. Do not include any prose, markdown fences, or commentary outside the JSON. The JSON object MUST start with `{` and end with `}` on the outermost level.\n\nSchema:\n";

// --- Shared URL helper ---

fn join_chat_completions(base: &str) -> String {
    format!("{}/chat/completions", base.trim_end_matches('/'))
}

// --- OpenAIJsonSchema ---

pub struct OpenAIJsonSchema {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
    body_options: BodyOptions,
}

impl OpenAIJsonSchema {
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
            body_options: BodyOptions::default(),
        }
    }

    pub fn with_body_options(mut self, body_options: BodyOptions) -> Self {
        self.body_options = body_options;
        self
    }
}

#[async_trait]
impl JsonStrategy for OpenAIJsonSchema {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let mut body = build_json_schema_body(&self.model, system, user, schema);
        self.body_options.apply(&mut body);

        let response = self
            .client
            .post_json(
                &join_chat_completions(&self.base_url),
                self.api_key.as_deref(),
                &[],
                &body,
            )
            .await?;

        extract_content(&response)
    }

    fn name(&self) -> &'static str {
        "openai-json-schema"
    }
}

// --- ForcedToolCall ---

pub struct ForcedToolCall {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
    body_options: BodyOptions,
}

impl ForcedToolCall {
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
            body_options: BodyOptions::default(),
        }
    }

    pub fn with_body_options(mut self, body_options: BodyOptions) -> Self {
        self.body_options = body_options;
        self
    }
}

#[async_trait]
impl JsonStrategy for ForcedToolCall {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let mut body = build_tool_call_body(&self.model, system, user, schema);
        self.body_options.apply(&mut body);

        let response = self
            .client
            .post_json(
                &join_chat_completions(&self.base_url),
                self.api_key.as_deref(),
                &[],
                &body,
            )
            .await?;

        extract_tool_call_arguments(&response)
    }

    fn name(&self) -> &'static str {
        "forced-tool-call"
    }
}

// --- ToolCallAndPrompt ---

pub struct ToolCallAndPrompt {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
    body_options: BodyOptions,
}

impl ToolCallAndPrompt {
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
            body_options: BodyOptions::default(),
        }
    }

    pub fn with_body_options(mut self, body_options: BodyOptions) -> Self {
        self.body_options = body_options;
        self
    }
}

#[async_trait]
impl JsonStrategy for ToolCallAndPrompt {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let augmented_system = format!("{system}{TOOL_NUDGE}");
        let mut body = build_tool_call_body(&self.model, &augmented_system, user, schema);
        self.body_options.apply(&mut body);

        let response = self
            .client
            .post_json(
                &join_chat_completions(&self.base_url),
                self.api_key.as_deref(),
                &[],
                &body,
            )
            .await?;

        extract_tool_call_arguments(&response)
    }

    fn name(&self) -> &'static str {
        "tool-call-and-prompt"
    }
}

// --- PromptToJson ---

pub struct PromptToJson {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
    body_options: BodyOptions,
}

impl PromptToJson {
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
            body_options: BodyOptions::default(),
        }
    }

    pub fn with_body_options(mut self, body_options: BodyOptions) -> Self {
        self.body_options = body_options;
        self
    }
}

#[async_trait]
impl JsonStrategy for PromptToJson {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let schema_text = serde_json::to_string(schema)
            .map_err(|e| StrategyError::Permanent(anyhow!("failed to serialize schema: {e}")))?;
        let augmented_system = format!("{system}{JSON_INSTRUCTION}{schema_text}");

        let mut body = build_prompt_body(&self.model, &augmented_system, user);
        self.body_options.apply(&mut body);

        let response = self
            .client
            .post_json(
                &join_chat_completions(&self.base_url),
                self.api_key.as_deref(),
                &[],
                &body,
            )
            .await?;

        extract_prompt_content_as_json(&response)
    }

    fn name(&self) -> &'static str {
        "prompt"
    }
}

// --- Private helpers ---

fn build_prompt_body(model: &str, system: &str, user: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user   }
        ]
    })
}

fn build_json_schema_body(model: &str, system: &str, user: &str, schema: &Value) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user   }
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": TOOL_NAME,
                "strict": true,
                "schema": schema
            }
        }
    })
}

fn build_tool_call_body(model: &str, system: &str, user: &str, schema: &Value) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user   }
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": TOOL_NAME,
                "description": TOOL_DESCRIPTION,
                "parameters": schema
            }
        }],
        "tool_choice": {
            "type": "function",
            "function": { "name": TOOL_NAME }
        }
    })
}

fn extract_content(response: &Value) -> Result<Value, StrategyError> {
    let choices = response
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("OpenAIJsonSchema: response had no choices"))
        })?;

    if choices.is_empty() {
        return Err(StrategyError::Permanent(anyhow!(
            "OpenAIJsonSchema: response had no choices"
        )));
    }

    let content = choices[0]
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!(
                "OpenAIJsonSchema: message.content was not a string"
            ))
        })?;

    // Strip BOM and leading whitespace before parsing.
    // Markdown fences → Permanent; PromptToJson handles fence stripping, not this strategy.
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();

    serde_json::from_str(trimmed).map_err(|e| {
        StrategyError::Permanent(anyhow!(
            "OpenAIJsonSchema: model returned non-JSON content: {e}"
        ))
    })
}

fn extract_tool_call_arguments(response: &Value) -> Result<Value, StrategyError> {
    let choices = response
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("tool-call strategy: response had no choices"))
        })?;

    if choices.is_empty() {
        return Err(StrategyError::Permanent(anyhow!(
            "tool-call strategy: response had no choices"
        )));
    }

    let message = choices[0]
        .get("message")
        .ok_or_else(|| StrategyError::Permanent(anyhow!("tool-call strategy: no message")))?;

    let tool_calls = message
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("tool-call strategy: no tool_calls in response"))
        })?;

    if tool_calls.is_empty() {
        return Err(StrategyError::Permanent(anyhow!(
            "tool-call strategy: tool_calls array was empty"
        )));
    }

    let function = tool_calls[0]
        .get("function")
        .ok_or_else(|| StrategyError::Permanent(anyhow!("tool-call strategy: no function")))?;

    // Verify the model called the right tool (not a hallucinated one).
    let fn_name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
    if fn_name != TOOL_NAME {
        return Err(StrategyError::Permanent(anyhow!(
            "tool-call strategy: model called unexpected function '{fn_name}'"
        )));
    }

    let arguments = function
        .get("arguments")
        .and_then(|a| a.as_str())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("tool-call strategy: arguments was not a string"))
        })?;

    let trimmed = arguments.trim_start_matches('\u{feff}').trim_start();

    serde_json::from_str(trimmed).map_err(|e| {
        StrategyError::Permanent(anyhow!("tool-call strategy: invalid JSON arguments: {e}"))
    })
}

fn extract_prompt_content_as_json(response: &Value) -> Result<Value, StrategyError> {
    let choices = response
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("PromptToJson: response had no choices"))
        })?;

    if choices.is_empty() {
        return Err(StrategyError::Permanent(anyhow!(
            "PromptToJson: response had no choices"
        )));
    }

    let content = choices[0]
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            StrategyError::Permanent(anyhow!("PromptToJson: message.content was not a string"))
        })?;

    // Strip BOM, then trim whitespace.
    let content = content.trim_start_matches('\u{feff}').trim();

    // One-shot markdown fence stripping. Only when content starts with ```.
    // Prose before a fence (e.g. "Here you go:\n```json...") → no strip → Permanent.
    let to_parse = if content.starts_with("```") {
        let after_first_fence = content.split_once('\n').map(|x| x.1).unwrap_or("");
        let stripped = after_first_fence
            .trim_end_matches("```")
            .trim_end_matches('\n')
            .trim();
        stripped
    } else {
        content
    };

    serde_json::from_str(to_parse).map_err(|e| {
        StrategyError::Permanent(anyhow!(
            "PromptToJson: model returned non-JSON content: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::llm::test_dsl::{extract_fails_with, extract_succeeds};

    // --- request bodies ---

    fn body_options() -> BodyOptions {
        BodyOptions {
            max_tokens: Some(64),
            extra: crate::llm::body::parse_extra_body(r#"{"thinking":{"type":"disabled"}}"#)
                .unwrap(),
        }
    }

    /// Every OpenAI-compatible strategy must honour --max-tokens/--extra-body;
    /// a rung that silently ignored them would leave a reasoning model thinking
    /// for minutes on the fallback path.
    fn assert_options_applied(mut body: Value) {
        body_options().apply(&mut body);
        assert_eq!(body["max_tokens"], json!(64));
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
        assert!(body["messages"].is_array(), "prompt survives the merge");
    }

    #[test]
    fn prompt_body_takes_options() {
        assert_options_applied(build_prompt_body("m", "sys", "usr"));
    }

    #[test]
    fn json_schema_body_takes_options() {
        assert_options_applied(build_json_schema_body("m", "sys", "usr", &json!({})));
    }

    #[test]
    fn tool_call_body_takes_options() {
        assert_options_applied(build_tool_call_body("m", "sys", "usr", &json!({})));
    }

    #[test]
    fn json_schema_body_keeps_schema_shape() {
        let body = build_json_schema_body("m", "sys", "usr", &json!({"type": "object"}));
        assert_eq!(body["response_format"]["type"], json!("json_schema"));
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            json!(TOOL_NAME)
        );
        assert_eq!(
            body["response_format"]["json_schema"]["strict"],
            json!(true)
        );
        assert_eq!(
            body["response_format"]["json_schema"]["schema"],
            json!({"type": "object"})
        );
    }

    // --- extract_content (OpenAIJsonSchema) ---

    #[test]
    fn happy_path() {
        extract_succeeds(
            extract_content(&json!({"choices":[{"message":{"content":"{\"x\":1}"}}]})),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn empty_choices_is_transient() {
        extract_fails_with(extract_content(&json!({"choices":[]})), "");
    }

    #[test]
    fn missing_choices_is_transient() {
        extract_fails_with(extract_content(&json!({})), "");
    }

    #[test]
    fn content_not_a_string() {
        extract_fails_with(
            extract_content(&json!({"choices":[{"message":{"content":null}}]})),
            "",
        );
    }

    #[test]
    fn content_invalid_json() {
        extract_fails_with(
            extract_content(&json!({"choices":[{"message":{"content":"not json"}}]})),
            "",
        );
    }

    #[test]
    fn content_with_bom_strips_cleanly() {
        extract_succeeds(
            extract_content(&json!({"choices":[{"message":{"content":"\u{feff}{\"x\":1}"}}]})),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn content_with_leading_whitespace() {
        extract_succeeds(
            extract_content(&json!({"choices":[{"message":{"content":"  {\"x\":1}"}}]})),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn content_with_markdown_fences() {
        extract_fails_with(
            extract_content(
                &json!({"choices":[{"message":{"content":"```json\n{\"x\":1}\n```"}}]}),
            ),
            "",
        );
    }

    // --- extract_tool_call_arguments ---

    fn tc(name: &str, arguments: &str) -> Value {
        json!({"choices":[{"message":{"tool_calls":[{"function":{"name":name,"arguments":arguments}}]}}]})
    }

    #[test]
    fn tc_happy_path() {
        extract_succeeds(
            extract_tool_call_arguments(&tc(TOOL_NAME, "{\"x\":1}")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn tc_tool_calls_empty() {
        extract_fails_with(
            extract_tool_call_arguments(&json!({"choices":[{"message":{"tool_calls":[]}}]})),
            "",
        );
    }

    #[test]
    fn tc_tool_calls_missing() {
        extract_fails_with(
            extract_tool_call_arguments(&json!({"choices":[{"message":{}}]})),
            "",
        );
    }

    #[test]
    fn tc_arguments_not_a_string() {
        extract_fails_with(
            extract_tool_call_arguments(
                &json!({"choices":[{"message":{"tool_calls":[{"function":{"name":TOOL_NAME,"arguments":{"x":1}}}]}}]}),
            ),
            "",
        );
    }

    #[test]
    fn tc_arguments_invalid_json() {
        extract_fails_with(extract_tool_call_arguments(&tc(TOOL_NAME, "not json")), "");
    }

    #[test]
    fn tc_arguments_with_bom() {
        extract_succeeds(
            extract_tool_call_arguments(&tc(TOOL_NAME, "\u{feff}{\"x\":1}")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn tc_wrong_function_name() {
        extract_fails_with(
            extract_tool_call_arguments(&tc("something_else", "{\"x\":1}")),
            "",
        );
    }

    #[test]
    fn tc_content_present_alongside_tool_calls() {
        let r = json!({"choices":[{"message":{"content":"hi","tool_calls":[{"function":{"name":TOOL_NAME,"arguments":"{\"x\":1}"}}]}}]});
        extract_succeeds(extract_tool_call_arguments(&r), &json!({"x": 1}));
    }

    // --- extract_prompt_content_as_json ---

    fn pt(content: &str) -> Value {
        json!({"choices":[{"message":{"content":content}}]})
    }

    #[test]
    fn pt_bare_json() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("{\"x\":1}")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_json_with_leading_whitespace() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("  \n{\"x\":1}")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_fenced_json() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("```json\n{\"x\":1}\n```")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_fenced_no_language() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("```\n{\"x\":1}\n```")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_fenced_with_trailing_newline() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("```json\n{\"x\":1}\n```\n")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_fenced_with_bom() {
        extract_succeeds(
            extract_prompt_content_as_json(&pt("\u{feff}```json\n{\"x\":1}\n```")),
            &json!({"x": 1}),
        );
    }

    #[test]
    fn pt_plain_text_no_json() {
        extract_fails_with(
            extract_prompt_content_as_json(&pt("Sure, here's the answer: 42")),
            "",
        );
    }

    #[test]
    fn pt_fenced_invalid_json_inside() {
        extract_fails_with(
            extract_prompt_content_as_json(&pt("```json\nnot json\n```")),
            "",
        );
    }

    #[test]
    fn pt_content_not_a_string() {
        extract_fails_with(
            extract_prompt_content_as_json(&json!({"choices":[{"message":{"content":null}}]})),
            "",
        );
    }

    #[test]
    fn pt_content_with_prose_then_fence() {
        extract_fails_with(
            extract_prompt_content_as_json(&pt("Here you go:\n```json\n{\"x\":1}\n```")),
            "",
        );
    }
}
