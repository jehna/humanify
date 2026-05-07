use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::preset::{env_api_key, JsonMode};
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

    let input = args.input;
    let base_url = args
        .base_url
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    let model = args.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let api_key = args.api_key.or_else(|| env_api_key(API_KEY_ENV));
    let context_size = args.context_size;
    let verbose = args.verbose;
    let output = args.output;

    let source = match pipe::read_input(&input) {
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
    let ladder = Arc::new(build_ladder(json_mode, client, base_url, model, api_key));
    let mut renamer = LlmRenamer::new(Arc::clone(&ladder), rt.handle().clone());

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

    if verbose {
        let locked = ladder.locked_strategy_name().unwrap_or("none");
        eprintln!("humanify: locked strategy: {locked}");
    }

    if let Err(e) = pipe::write_output(output.as_deref(), &renamed) {
        eprintln!("humanify: failed to write output: {e}");
        return 1;
    }

    0
}

fn build_ladder(
    mode: JsonMode,
    client: HttpClient,
    base_url: String,
    model: String,
    api_key: Option<String>,
) -> Ladder {
    match mode {
        JsonMode::Ladder => build_default_ladder(client, base_url, model, api_key),
        JsonMode::OpenAIJsonSchema => Ladder::pinned(Arc::new(OpenAIJsonSchema::new(
            client, base_url, api_key, model,
        ))),
        JsonMode::ForcedToolCall => Ladder::pinned(Arc::new(ForcedToolCall::new(
            client, base_url, api_key, model,
        ))),
        JsonMode::ToolCallAndPrompt => Ladder::pinned(Arc::new(ToolCallAndPrompt::new(
            client, base_url, api_key, model,
        ))),
        JsonMode::Prompt => Ladder::pinned(Arc::new(PromptToJson::new(
            client, base_url, api_key, model,
        ))),
        JsonMode::AnthropicNative => unreachable!("rejected before build_ladder"),
    }
}

fn build_default_ladder(
    client: HttpClient,
    base_url: String,
    model: String,
    api_key: Option<String>,
) -> Ladder {
    let strategies: Vec<Arc<dyn JsonStrategy>> = vec![
        Arc::new(OpenAIJsonSchema::new(
            client.clone(),
            base_url.clone(),
            api_key.clone(),
            model.clone(),
        )),
        Arc::new(ForcedToolCall::new(
            client.clone(),
            base_url.clone(),
            api_key.clone(),
            model.clone(),
        )),
        Arc::new(PromptToJson::new(client, base_url, api_key, model)),
    ];
    Ladder::new(strategies)
}
