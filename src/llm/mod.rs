pub mod http;
pub mod openai_compat;

use async_trait::async_trait;
use serde_json::Value;

pub use http::{classify_error, HttpClient, StrategyError};
pub use openai_compat::{ForcedToolCall, OpenAIJsonSchema, PromptToJson, ToolCallAndPrompt};

#[async_trait]
pub trait JsonStrategy: Send + Sync {
    /// Send `system` + `user` messages to the LLM and request a JSON response
    /// matching `schema`. Returns parsed JSON on success.
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError>;

    /// Human-readable name for logging / `--json-mode` flag matching.
    fn name(&self) -> &'static str;
}
