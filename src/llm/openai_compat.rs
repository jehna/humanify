use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::llm::{
    http::{HttpClient, StrategyError},
    JsonStrategy,
};

pub struct OpenAIJsonSchema {
    client: HttpClient,
    base_url: String,
    api_key: Option<String>,
    model: String,
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
        }
    }
}

#[async_trait]
impl JsonStrategy for OpenAIJsonSchema {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        let base_url = self.base_url.trim_end_matches('/');
        let url = format!("{base_url}/chat/completions");

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user   }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "humanify_response",
                    "strict": true,
                    "schema": schema
                }
            }
        });

        let response = self
            .client
            .post_json(&url, self.api_key.as_deref(), &[], &body)
            .await?;

        extract_content(&response)
    }

    fn name(&self) -> &'static str {
        "openai-json-schema"
    }
}

fn extract_content(response: &Value) -> Result<Value, StrategyError> {
    let choices = response
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            StrategyError::Transient(anyhow!("OpenAIJsonSchema: response had no choices"))
        })?;

    if choices.is_empty() {
        return Err(StrategyError::Transient(anyhow!(
            "OpenAIJsonSchema: response had no choices"
        )));
    }

    let content = choices[0]
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            StrategyError::Transient(anyhow!(
                "OpenAIJsonSchema: message.content was not a string"
            ))
        })?;

    // Strip BOM and leading whitespace before parsing.
    // If a provider wraps JSON in markdown fences, parsing will fail → Transient.
    // That's correct: PromptToJson (task 6) handles fence stripping, not this strategy.
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();

    serde_json::from_str(trimmed).map_err(|e| {
        StrategyError::Transient(anyhow!(
            "OpenAIJsonSchema: model returned non-JSON content: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(response: Value) -> Result<Value, StrategyError> {
        extract_content(&response)
    }

    fn assert_transient(result: Result<Value, StrategyError>) {
        match result {
            Err(StrategyError::Transient(_)) => {}
            Ok(v) => panic!("expected Transient, got Ok({v})"),
            Err(StrategyError::NotSupported(r)) => {
                panic!("expected Transient, got NotSupported({r})")
            }
        }
    }

    #[test]
    fn happy_path() {
        let response = json!({"choices":[{"message":{"content":"{\"x\":1}"}}]});
        assert_eq!(run(response).unwrap(), json!({"x": 1}));
    }

    #[test]
    fn empty_choices_is_transient() {
        assert_transient(run(json!({"choices":[]})));
    }

    #[test]
    fn missing_choices_is_transient() {
        assert_transient(run(json!({})));
    }

    #[test]
    fn content_not_a_string() {
        assert_transient(run(json!({"choices":[{"message":{"content":null}}]})));
    }

    #[test]
    fn content_invalid_json() {
        assert_transient(run(json!({"choices":[{"message":{"content":"not json"}}]})));
    }

    #[test]
    fn content_with_bom_strips_cleanly() {
        let content = "\u{feff}{\"x\":1}";
        let response = json!({"choices":[{"message":{"content": content}}]});
        assert_eq!(run(response).unwrap(), json!({"x": 1}));
    }

    #[test]
    fn content_with_leading_whitespace() {
        let response = json!({"choices":[{"message":{"content":"  {\"x\":1}"}}]});
        assert_eq!(run(response).unwrap(), json!({"x": 1}));
    }

    #[test]
    fn content_with_markdown_fences() {
        let content = "```json\n{\"x\":1}\n```";
        let response = json!({"choices":[{"message":{"content": content}}]});
        assert_transient(run(response));
    }
}
