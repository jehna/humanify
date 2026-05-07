use std::env;

pub struct PresetConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub json_mode: JsonMode,
    pub context_size: usize,
    pub verbose: bool,
}

/// Selects which JSON strategy (or ladder) to use for a run.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonMode {
    Ladder,
    OpenAIJsonSchema,
    AnthropicNative,
    ForcedToolCall,
    ToolCallAndPrompt,
    Prompt,
}

impl JsonMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "ladder" => Ok(JsonMode::Ladder),
            "openai-json-schema" => Ok(JsonMode::OpenAIJsonSchema),
            "anthropic-native" => Ok(JsonMode::AnthropicNative),
            "forced-tool-call" => Ok(JsonMode::ForcedToolCall),
            "tool-call-and-prompt" => Ok(JsonMode::ToolCallAndPrompt),
            "prompt" => Ok(JsonMode::Prompt),
            other => Err(format!(
                "unknown json-mode '{}'. Valid values: ladder, openai-json-schema, \
                 anthropic-native, forced-tool-call, tool-call-and-prompt, prompt",
                other
            )),
        }
    }
}

/// Read an API key from the given env var. Returns `None` if unset or empty.
pub fn env_api_key(var_name: &str) -> Option<String> {
    env::var(var_name).ok().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_parses() {
        assert_eq!(JsonMode::parse("ladder"), Ok(JsonMode::Ladder));
    }

    #[test]
    fn openai_json_schema_parses() {
        assert_eq!(
            JsonMode::parse("openai-json-schema"),
            Ok(JsonMode::OpenAIJsonSchema)
        );
    }

    #[test]
    fn anthropic_native_parses() {
        assert_eq!(
            JsonMode::parse("anthropic-native"),
            Ok(JsonMode::AnthropicNative)
        );
    }

    #[test]
    fn forced_tool_call_parses() {
        assert_eq!(
            JsonMode::parse("forced-tool-call"),
            Ok(JsonMode::ForcedToolCall)
        );
    }

    #[test]
    fn tool_call_and_prompt_parses() {
        assert_eq!(
            JsonMode::parse("tool-call-and-prompt"),
            Ok(JsonMode::ToolCallAndPrompt)
        );
    }

    #[test]
    fn prompt_parses() {
        assert_eq!(JsonMode::parse("prompt"), Ok(JsonMode::Prompt));
    }

    #[test]
    fn unknown_returns_err_with_valid_values() {
        let err = JsonMode::parse("garbage").unwrap_err();
        assert!(err.contains("garbage"), "err: {err}");
        assert!(err.contains("ladder"), "err: {err}");
    }

    #[test]
    fn empty_string_returns_err() {
        assert!(JsonMode::parse("").is_err());
    }

    #[test]
    fn case_sensitive_rejects_uppercase() {
        assert!(JsonMode::parse("Ladder").is_err());
    }

    #[test]
    fn env_api_key_returns_value_when_set() {
        std::env::set_var("_TEST_KEY_SET", "mykey");
        assert_eq!(env_api_key("_TEST_KEY_SET"), Some("mykey".to_string()));
        std::env::remove_var("_TEST_KEY_SET");
    }

    #[test]
    fn env_api_key_returns_none_when_unset() {
        std::env::remove_var("_TEST_KEY_UNSET");
        assert_eq!(env_api_key("_TEST_KEY_UNSET"), None);
    }

    #[test]
    fn env_api_key_returns_none_when_empty() {
        std::env::set_var("_TEST_KEY_EMPTY", "");
        assert_eq!(env_api_key("_TEST_KEY_EMPTY"), None);
        std::env::remove_var("_TEST_KEY_EMPTY");
    }
}
