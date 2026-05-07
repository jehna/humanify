use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::preset::{env_api_key, JsonMode, PresetConfig};
use crate::llm::{
    http::HttpClient, ForcedToolCall, JsonStrategy, Ladder, LlmRenamer, OpenAIJsonSchema,
    PromptToJson, ToolCallAndPrompt,
};
use crate::pipe;
use crate::rename::{rename_all_identifiers, RenameError};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";
pub const API_KEY_ENV: &str = "OPENAI_API_KEY";

/// Plain args carrier — not a clap struct so it can be constructed without clap.
pub struct Args {
    pub input: String,
    pub output: Option<PathBuf>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub context_size: usize,
    pub json_mode: String,
    pub verbose: bool,
}

/// Run the openai subcommand. Returns a process exit code (0 / 1 / 2 / 64).
pub fn run(args: Args) -> i32 {
    let json_mode = match JsonMode::parse(&args.json_mode) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("humanify: {e}");
            return 64;
        }
    };

    if json_mode == JsonMode::AnthropicNative {
        eprintln!(
            "humanify: --json-mode anthropic-native is only valid for the `anthropic` subcommand"
        );
        return 64;
    }

    let cfg = PresetConfig {
        base_url: args
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        model: args.model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        api_key: args.api_key.or_else(|| env_api_key(API_KEY_ENV)),
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

    let client = HttpClient::new();
    let ladder = Arc::new(build_ladder(client, &cfg));
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

fn build_ladder(client: HttpClient, cfg: &PresetConfig) -> Ladder {
    match cfg.json_mode {
        JsonMode::Ladder => build_default_ladder(client, cfg),
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
        JsonMode::AnthropicNative => unreachable!("rejected before build_ladder"),
    }
}

fn build_default_ladder(client: HttpClient, cfg: &PresetConfig) -> Ladder {
    let strategies: Vec<Arc<dyn JsonStrategy>> = vec![
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
    ];
    Ladder::new(strategies)
}
