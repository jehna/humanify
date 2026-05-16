use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use crate::llm::{
    http::HttpClient, AnthropicNativeJsonSchema, AnthropicToolCallAndPrompt, ForcedToolCall,
    JsonStrategy, Ladder, LlmRenamer, OpenAIJsonSchema, PromptToJson, ToolCallAndPrompt,
};
use crate::pipe;
use crate::rename::{rename_all_identifiers, RenameError};

pub struct PresetConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub json_mode: JsonMode,
    pub context_size: usize,
    pub verbose: bool,
}

#[derive(Clone, Copy)]
pub enum ProviderKind {
    OpenAICompat,
    Anthropic,
}

#[derive(Clone, Copy)]
pub struct PresetDefaults {
    pub base_url: &'static str,
    pub model: &'static str,
    pub api_key_env: &'static str,
    pub provider_kind: ProviderKind,
    /// Per-request HTTP timeout. Set generously for local providers (Ollama on a
    /// CPU runner can take ~10–15 min for a single constrained completion) and
    /// tight for hosted APIs that answer in seconds.
    pub timeout_seconds: u64,
}

/// Generic args carrier for all presets.
pub struct PresetArgs {
    pub input: String,
    pub output: Option<PathBuf>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub context_size: usize,
    pub json_mode: String,
    pub verbose: bool,
    pub timeout_seconds: Option<u64>,
}

/// Returns Err with a user-facing message if `mode` is not valid for `kind`.
pub fn validate_json_mode_for_provider(mode: &JsonMode, kind: ProviderKind) -> Result<(), String> {
    match (mode, kind) {
        (JsonMode::AnthropicNative, ProviderKind::OpenAICompat) => Err(
            "--json-mode anthropic-native is only valid for the `anthropic` subcommand".to_string(),
        ),
        (
            JsonMode::OpenAIJsonSchema | JsonMode::ForcedToolCall | JsonMode::ToolCallAndPrompt | JsonMode::Prompt,
            ProviderKind::Anthropic,
        ) => Err(format!(
            "--json-mode {} is not valid for the `anthropic` subcommand; use anthropic-native or ladder",
            mode.as_str()
        )),
        _ => Ok(()),
    }
}

/// Drives the full pipeline for any preset. Returns process exit code (0 / 1 / 2 / 64).
pub fn run_preset(args: PresetArgs, defaults: PresetDefaults) -> i32 {
    let json_mode = match JsonMode::parse(&args.json_mode) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("humanify: {e}");
            return 64;
        }
    };

    if let Err(msg) = validate_json_mode_for_provider(&json_mode, defaults.provider_kind) {
        eprintln!("humanify: {msg}");
        return 64;
    }

    let cfg = PresetConfig {
        base_url: args
            .base_url
            .unwrap_or_else(|| defaults.base_url.to_string()),
        model: args.model.unwrap_or_else(|| defaults.model.to_string()),
        api_key: args.api_key.or_else(|| env_api_key(defaults.api_key_env)),
        json_mode,
        context_size: args.context_size,
        verbose: args.verbose,
    };
    let output = args.output;

    let source = match pipe::read_input(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("humanify: failed to read input: {e}");
            return 1;
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("humanify: failed to create tokio runtime: {e}");
            return 1;
        }
    };

    let timeout =
        std::time::Duration::from_secs(args.timeout_seconds.unwrap_or(defaults.timeout_seconds));
    let client = HttpClient::with_timeout(timeout);
    let ladder = Arc::new(build_ladder(client, &cfg, defaults.provider_kind));
    let mut renamer = LlmRenamer::new(Arc::clone(&ladder), rt.handle().clone());
    let context_size = cfg.context_size;

    let result = rt.block_on(async move {
        tokio::task::spawn_blocking(move || {
            rename_all_identifiers(&source, &mut renamer, context_size)
        })
        .await
    });

    let renamed = match result {
        Ok(Ok(s)) => s,
        Ok(Err(RenameError::Parse(msg))) => {
            eprintln!("humanify: parse error: {msg}");
            return 2;
        }
        Err(join_err) => {
            eprintln!("humanify: internal error: {join_err}");
            return 1;
        }
    };

    if cfg.verbose {
        let locked = ladder.locked_strategy_name().unwrap_or("none");
        eprintln!("humanify: locked strategy: {locked}");
    }

    if let Err(e) = pipe::write_output(output.as_deref(), &renamed) {
        eprintln!("humanify: failed to write output: {e}");
        return 1;
    }

    0
}

fn build_ladder(client: HttpClient, cfg: &PresetConfig, kind: ProviderKind) -> Ladder {
    match cfg.json_mode {
        JsonMode::Ladder => build_default_ladder(client, cfg, kind),
        JsonMode::OpenAIJsonSchema => Ladder::pinned(Arc::new(OpenAIJsonSchema::new(
            client,
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            cfg.model.clone(),
        ))),
        JsonMode::ForcedToolCall => Ladder::pinned(Arc::new(ForcedToolCall::new(
            client,
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            cfg.model.clone(),
        ))),
        JsonMode::ToolCallAndPrompt => Ladder::pinned(Arc::new(ToolCallAndPrompt::new(
            client,
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            cfg.model.clone(),
        ))),
        JsonMode::Prompt => Ladder::pinned(Arc::new(PromptToJson::new(
            client,
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            cfg.model.clone(),
        ))),
        JsonMode::AnthropicNative => Ladder::pinned(Arc::new(AnthropicNativeJsonSchema::new(
            client,
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            cfg.model.clone(),
        ))),
    }
}

pub(crate) fn build_default_ladder(
    client: HttpClient,
    cfg: &PresetConfig,
    kind: ProviderKind,
) -> Ladder {
    let strategies: Vec<Arc<dyn JsonStrategy>> = match kind {
        ProviderKind::OpenAICompat => vec![
            Arc::new(OpenAIJsonSchema::new(
                client.clone(),
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )),
            Arc::new(ForcedToolCall::new(
                client.clone(),
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )),
            Arc::new(PromptToJson::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )),
        ],
        ProviderKind::Anthropic => vec![
            Arc::new(AnthropicNativeJsonSchema::new(
                client.clone(),
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )),
            Arc::new(AnthropicToolCallAndPrompt::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )),
        ],
    };
    Ladder::new(strategies)
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

    pub fn as_str(&self) -> &'static str {
        match self {
            JsonMode::Ladder => "ladder",
            JsonMode::OpenAIJsonSchema => "openai-json-schema",
            JsonMode::AnthropicNative => "anthropic-native",
            JsonMode::ForcedToolCall => "forced-tool-call",
            JsonMode::ToolCallAndPrompt => "tool-call-and-prompt",
            JsonMode::Prompt => "prompt",
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

    // --- JsonMode::parse ---

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

    // --- env_api_key ---

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

    // --- validate_json_mode_for_provider ---

    #[test]
    fn validate_anthropic_native_on_openai_compat_returns_err() {
        assert!(validate_json_mode_for_provider(
            &JsonMode::AnthropicNative,
            ProviderKind::OpenAICompat
        )
        .is_err());
    }

    #[test]
    fn validate_anthropic_native_on_anthropic_returns_ok() {
        assert!(validate_json_mode_for_provider(
            &JsonMode::AnthropicNative,
            ProviderKind::Anthropic
        )
        .is_ok());
    }

    #[test]
    fn validate_valid_mode_on_openai_compat_returns_ok() {
        assert!(
            validate_json_mode_for_provider(&JsonMode::Ladder, ProviderKind::OpenAICompat).is_ok()
        );
    }

    #[test]
    fn validate_openai_json_schema_on_anthropic_returns_err() {
        assert!(validate_json_mode_for_provider(
            &JsonMode::OpenAIJsonSchema,
            ProviderKind::Anthropic
        )
        .is_err());
    }

    #[test]
    fn validate_forced_tool_call_on_anthropic_returns_err() {
        assert!(validate_json_mode_for_provider(
            &JsonMode::ForcedToolCall,
            ProviderKind::Anthropic
        )
        .is_err());
    }

    #[test]
    fn validate_tool_call_and_prompt_on_anthropic_returns_err() {
        assert!(validate_json_mode_for_provider(
            &JsonMode::ToolCallAndPrompt,
            ProviderKind::Anthropic
        )
        .is_err());
    }

    #[test]
    fn validate_prompt_on_anthropic_returns_err() {
        assert!(
            validate_json_mode_for_provider(&JsonMode::Prompt, ProviderKind::Anthropic).is_err()
        );
    }

    #[test]
    fn validate_ladder_on_anthropic_returns_ok() {
        assert!(
            validate_json_mode_for_provider(&JsonMode::Ladder, ProviderKind::Anthropic).is_ok()
        );
    }

    #[test]
    fn validate_prompt_on_openai_compat_returns_ok() {
        assert!(
            validate_json_mode_for_provider(&JsonMode::Prompt, ProviderKind::OpenAICompat).is_ok()
        );
    }

    // --- PresetDefaults sanity ---

    #[test]
    fn openai_defaults_constants() {
        assert_eq!(
            crate::cli::openai::DEFAULTS.base_url,
            "https://api.openai.com/v1"
        );
        assert_eq!(crate::cli::openai::DEFAULTS.model, "gpt-5.4-mini");
        assert_eq!(crate::cli::openai::DEFAULTS.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn gemini_defaults_constants() {
        assert_eq!(
            crate::cli::gemini::DEFAULTS.base_url,
            "https://generativelanguage.googleapis.com/v1beta/openai/"
        );
        assert_eq!(crate::cli::gemini::DEFAULTS.model, "gemini-3.1-flash-lite");
        assert_eq!(crate::cli::gemini::DEFAULTS.api_key_env, "GEMINI_API_KEY");
    }

    #[test]
    fn anthropic_defaults_constants() {
        assert_eq!(
            crate::cli::anthropic::DEFAULTS.base_url,
            "https://api.anthropic.com/v1"
        );
        assert_eq!(crate::cli::anthropic::DEFAULTS.model, "claude-sonnet-4-6");
        assert_eq!(
            crate::cli::anthropic::DEFAULTS.api_key_env,
            "ANTHROPIC_API_KEY"
        );
    }

    #[test]
    fn hosted_providers_share_short_timeout() {
        // Hosted APIs answer in seconds; a tight per-request budget surfaces
        // upstream stalls quickly instead of letting the run hang.
        assert_eq!(crate::cli::openai::DEFAULTS.timeout_seconds, 60);
        assert_eq!(crate::cli::gemini::DEFAULTS.timeout_seconds, 60);
        assert_eq!(crate::cli::anthropic::DEFAULTS.timeout_seconds, 60);
        assert_eq!(crate::cli::openrouter::DEFAULTS.timeout_seconds, 60);
    }

    #[test]
    fn ollama_gets_generous_timeout_for_local_inference() {
        assert_eq!(crate::cli::ollama::DEFAULTS.timeout_seconds, 1800);
    }

    // --- run_preset early-exit paths (no I/O reached) ---

    fn preset_args_no_io(json_mode: &str) -> PresetArgs {
        PresetArgs {
            input: "irrelevant".to_string(),
            output: None,
            model: None,
            api_key: None,
            base_url: None,
            context_size: 500,
            json_mode: json_mode.to_string(),
            verbose: false,
            timeout_seconds: None,
        }
    }

    #[test]
    fn anthropic_native_on_openai_compat_returns_64() {
        let code = run_preset(
            preset_args_no_io("anthropic-native"),
            crate::cli::openai::DEFAULTS,
        );
        assert_eq!(code, 64);
    }

    #[test]
    fn unknown_json_mode_returns_64() {
        let code = run_preset(preset_args_no_io("garbage"), crate::cli::openai::DEFAULTS);
        assert_eq!(code, 64);
    }

    // --- Anthropic default ladder shape ---

    fn anthropic_preset_cfg() -> PresetConfig {
        PresetConfig {
            base_url: crate::cli::anthropic::DEFAULTS.base_url.to_string(),
            model: crate::cli::anthropic::DEFAULTS.model.to_string(),
            api_key: None,
            json_mode: JsonMode::Ladder,
            context_size: 500,
            verbose: false,
        }
    }

    #[test]
    fn anthropic_default_ladder_has_two_strategies() {
        let ladder = build_default_ladder(
            HttpClient::new(),
            &anthropic_preset_cfg(),
            ProviderKind::Anthropic,
        );
        assert_eq!(ladder.strategy_count(), 2);
    }
}
