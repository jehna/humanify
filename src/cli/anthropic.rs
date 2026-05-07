use std::path::PathBuf;

use crate::cli::preset::{run_preset, PresetArgs, PresetDefaults, ProviderKind};

pub const DEFAULTS: PresetDefaults = PresetDefaults {
    base_url: "https://api.anthropic.com/v1",
    model: "claude-sonnet-4-6",
    api_key_env: "ANTHROPIC_API_KEY",
    provider_kind: ProviderKind::Anthropic,
};

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
        }
    }
}

/// Run the anthropic subcommand. Returns a process exit code (0 / 1 / 2 / 64).
pub fn run(args: Args) -> i32 {
    run_preset(args.into(), DEFAULTS)
}
