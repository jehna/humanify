use clap::{error::ErrorKind, Parser, Subcommand};
use humanify::cli::{anthropic, gemini, ollama, openai, openrouter, requesty};
use std::path::PathBuf;

const EXIT_CLI_USAGE: i32 = 64;

#[derive(Parser)]
#[command(
    name = "humanify",
    version,
    about = "Un-minify JavaScript with LLM help"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Openai(SubArgs),
    Gemini(SubArgs),
    Anthropic(SubArgs),
    Ollama(SubArgs),
    Openrouter(SubArgs),
    Requesty(SubArgs),
}

#[derive(Parser)]
struct SubArgs {
    /// Filename, or `-` for stdin
    input: String,

    /// Output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Override preset's default model
    #[arg(short, long)]
    model: Option<String>,

    /// Override env-var-based API key
    #[arg(short = 'k', long)]
    api_key: Option<String>,

    /// Override preset's base URL
    #[arg(long)]
    base_url: Option<String>,

    /// Surrounding code chars per identifier (default: 500)
    #[arg(long)]
    context_size: Option<usize>,
    /// Context chars used for the cache key only (default: --context-size).
    /// Pin one value so runs at different --context-size still share entries.
    /// Also settable via HUMANIFY_CACHE_CONTEXT_SIZE.
    #[arg(long)]
    cache_context_size: Option<usize>,

    /// JSON strategy mode (default: ladder)
    #[arg(long)]
    json_mode: Option<String>,

    /// Per-request HTTP timeout in seconds. Overrides the preset default
    /// (60s for hosted APIs, 1800s for Ollama).
    #[arg(long)]
    timeout_seconds: Option<u64>,

    /// Directory for the per-call LLM response cache. Enables caching.
    /// Also settable via HUMANIFY_CACHE_DIR.
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Disable the cache even if --cache-dir or HUMANIFY_CACHE_DIR is set.
    #[arg(long)]
    no_cache: bool,

    /// Ignore stored answers but keep writing fresh ones, re-asking the model and
    /// overwriting what is cached. Use it to re-run a region with a better model.
    /// Unlike --no-cache, which disables reads *and* writes.
    #[arg(long)]
    refresh_cache: bool,

    /// Retries per identifier on transient errors (default: 3, 0 disables).
    #[arg(long)]
    max_retries: Option<u32>,

    /// Wall-clock budget in seconds. Past it, remaining identifiers are left
    /// unchanged and the (partial) output is still written. 0 or omitted = unlimited.
    #[arg(long)]
    max_run_seconds: Option<u64>,

    /// Cap the response length (`max_tokens`). Unset by default on hosted APIs.
    /// Reasoning tokens count against this budget, so only cap a thinking model
    /// once thinking is off — otherwise the reply is truncated to nothing.
    #[arg(long)]
    max_tokens: Option<u32>,

    /// JSON object merged into every request body, or `@file.json` to read it
    /// from a file. Top-level keys win over humanify's, `messages`/`system`/
    /// `stream` are rejected. Use it for provider-specific knobs, e.g.
    /// `--extra-body '{"thinking":{"type":"disabled"}}'` to stop a GLM model
    /// from spending minutes of chain of thought on a one-word answer.
    #[arg(long)]
    extra_body: Option<String>,

    /// Literal fragment of the input marking where renaming starts, or
    /// `@file.txt` to read the fragment from a file. Copy it from the input
    /// itself and check in your editor that it matches exactly once — 0 or 2+
    /// matches are hard errors. Combine with --stop-sentinel to bound a region.
    #[arg(long)]
    start_sentinel: Option<String>,

    /// Literal fragment of the input marking where renaming stops (inclusive),
    /// or `@file.txt`. Same matching rules as --start-sentinel.
    #[arg(long)]
    stop_sentinel: Option<String>,

    /// Only rename identifiers *declared* inside the sentinel window. By default
    /// a helper declared elsewhere but referenced inside the window is renamed
    /// too, so its call sites in the region read meaningfully.
    #[arg(long)]
    sentinel_strict: bool,

    /// Also rename inside the body of a helper the window pulled in by
    /// reference. Costs 10-50x more per helper; worth it when the callee is your
    /// own code rather than a third-party bundle.
    #[arg(long, conflicts_with = "sentinel_strict")]
    sentinel_expand_helpers: bool,

    /// Resolve the sentinels, print the window and the identifiers it selects,
    /// and exit without making a single LLM call.
    #[arg(long)]
    dry_run: bool,

    /// Show resolved configuration and rename steps on stderr
    #[arg(short, long)]
    verbose: bool,

    /// Show an identifier progress bar on stderr
    #[arg(long)]
    progress: bool,
}

fn into_openai_args(a: SubArgs) -> openai::Args {
    openai::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn into_gemini_args(a: SubArgs) -> gemini::Args {
    gemini::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn into_anthropic_args(a: SubArgs) -> anthropic::Args {
    anthropic::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn into_ollama_args(a: SubArgs) -> ollama::Args {
    ollama::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn into_openrouter_args(a: SubArgs) -> openrouter::Args {
    openrouter::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn into_requesty_args(a: SubArgs) -> requesty::Args {
    requesty::Args {
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
        start_sentinel: a.start_sentinel,
        stop_sentinel: a.stop_sentinel,
        sentinel_strict: a.sentinel_strict,
        sentinel_expand_helpers: a.sentinel_expand_helpers,
        dry_run: a.dry_run,
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                e.exit();
            }
            _ => {
                let _ = e.print();
                std::process::exit(EXIT_CLI_USAGE);
            }
        },
    };

    let exit_code = match cli.command {
        Commands::Openai(args) => openai::run(into_openai_args(args)),
        Commands::Gemini(args) => gemini::run(into_gemini_args(args)),
        Commands::Anthropic(args) => anthropic::run(into_anthropic_args(args)),
        Commands::Ollama(args) => ollama::run(into_ollama_args(args)),
        Commands::Openrouter(args) => openrouter::run(into_openrouter_args(args)),
        Commands::Requesty(args) => requesty::run(into_requesty_args(args)),
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
