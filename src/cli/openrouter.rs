use std::path::PathBuf;

use crate::cli::preset::{run_preset, PresetArgs, PresetDefaults, ProviderKind};

pub const DEFAULTS: PresetDefaults = PresetDefaults {
    name: "openrouter",
    base_url: "https://openrouter.ai/api/v1",
    model: "openai/gpt-oss-120b",
    api_key_env: "OPENROUTER_API_KEY",
    provider_kind: ProviderKind::OpenAICompat,
    timeout_seconds: 60,
};

pub struct Args {
    pub input: String,
    pub output: Option<PathBuf>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub context_size: Option<usize>,
    pub cache_context_size: Option<usize>,
    pub json_mode: Option<String>,
    pub verbose: bool,
    pub progress: bool,
    pub timeout_seconds: Option<u64>,
    pub cache_dir: Option<PathBuf>,
    pub no_cache: bool,
    pub refresh_cache: bool,
    pub max_retries: Option<u32>,
    pub max_run_seconds: Option<u64>,
    pub max_tokens: Option<u32>,
    pub extra_body: Option<String>,
}

impl From<Args> for PresetArgs {
    fn from(a: Args) -> Self {
        PresetArgs {
            input: a.input,
            output: a.output,
            model: a.model,
            api_key: a.api_key,
            base_url: a.base_url,
            context_size: a.context_size,
            cache_context_size: a.cache_context_size,
            json_mode: a.json_mode,
            verbose: a.verbose,
            progress: a.progress,
            timeout_seconds: a.timeout_seconds,
            cache_dir: a.cache_dir,
            no_cache: a.no_cache,
            refresh_cache: a.refresh_cache,
            max_retries: a.max_retries,
            max_run_seconds: a.max_run_seconds,
            max_tokens: a.max_tokens,
            extra_body: a.extra_body,
        }
    }
}

pub fn run(args: Args) -> i32 {
    run_preset(args.into(), DEFAULTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_defaults_constants() {
        assert_eq!(DEFAULTS.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(DEFAULTS.model, "openai/gpt-oss-120b");
        assert_eq!(DEFAULTS.api_key_env, "OPENROUTER_API_KEY");
        assert!(matches!(DEFAULTS.provider_kind, ProviderKind::OpenAICompat));
    }
}
