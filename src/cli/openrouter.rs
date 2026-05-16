use std::path::PathBuf;

use crate::cli::preset::{run_preset, PresetArgs, PresetDefaults, ProviderKind};

pub const DEFAULTS: PresetDefaults = PresetDefaults {
    base_url: "https://openrouter.ai/api/v1",
    model: "x-ai/grok-code-fast-1",
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
    pub context_size: usize,
    pub json_mode: String,
    pub verbose: bool,
    pub timeout_seconds: Option<u64>,
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
            json_mode: a.json_mode,
            verbose: a.verbose,
            timeout_seconds: a.timeout_seconds,
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
        assert_eq!(DEFAULTS.model, "x-ai/grok-code-fast-1");
        assert_eq!(DEFAULTS.api_key_env, "OPENROUTER_API_KEY");
        assert!(matches!(DEFAULTS.provider_kind, ProviderKind::OpenAICompat));
    }
}
