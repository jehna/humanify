use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::cache::{BudgetRenamer, CacheScope, CacheStats, CachingRenamer, DiskCache};
use crate::llm::renamer::RetryPolicy;
use crate::llm::{
    http::HttpClient, parse_extra_body, AnthropicNativeJsonSchema, AnthropicToolCallAndPrompt,
    BodyOptions, ForcedToolCall, JsonStrategy, Ladder, LlmRenamer, OpenAIJsonSchema, PromptToJson,
    ToolCallAndPrompt,
};
use crate::pipe;
use crate::rename::sentinel;
use crate::rename::{
    rename_all_identifiers_with_options, RenameError, RenameObserver, RenameOptions, Renamer,
    SelectionPolicy, SentinelReport, SentinelSpec,
};

const DEFAULT_CONTEXT_SIZE: usize = 500;
const DEFAULT_JSON_MODE: &str = "ladder";

pub struct PresetConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub json_mode: JsonMode,
    pub context_size: usize,
    /// Context used to build cache keys (`--cache-context-size`). Equal to
    /// `context_size` unless the flag or `HUMANIFY_CACHE_CONTEXT_SIZE` is set.
    pub cache_context_size: usize,
    pub verbose: bool,
    /// Extra request-body knobs (`--max-tokens`, `--extra-body`), applied to
    /// whichever strategy the ladder ends up using.
    pub body_options: BodyOptions,
}

#[derive(Clone, Copy)]
pub enum ProviderKind {
    OpenAICompat,
    Anthropic,
}

#[derive(Clone, Copy)]
pub struct PresetDefaults {
    pub name: &'static str,
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
    /// Literal source fragment (or `@file`) marking the start of the region to
    /// rename.
    pub start_sentinel: Option<String>,
    /// Literal source fragment (or `@file`) marking the end of that region.
    pub stop_sentinel: Option<String>,
    /// Policy A: only symbols *declared* inside the window.
    pub sentinel_strict: bool,
    /// Policy C: also rename inside the body of a helper pulled in by reference.
    pub sentinel_expand_helpers: bool,
    /// Resolve, filter, print the selection, and exit without making a single
    /// LLM call.
    pub dry_run: bool,
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
    let model_from_cli = args.model.is_some();
    let api_key_from_cli = args.api_key.is_some();
    let base_url_from_cli = args.base_url.is_some();
    let context_size_from_cli = args.context_size.is_some();
    let cache_context_size_from_cli = args.cache_context_size.is_some();
    let json_mode_from_cli = args.json_mode.is_some();
    let timeout_from_cli = args.timeout_seconds.is_some();
    let cache_dir_from_cli = args.cache_dir.is_some();
    let no_cache_from_cli = args.no_cache;
    let max_retries_from_cli = args.max_retries.is_some();
    let max_run_seconds_from_cli = args.max_run_seconds.is_some();
    let max_tokens_from_cli = args.max_tokens.is_some();

    let json_mode_name = args.json_mode.as_deref().unwrap_or(DEFAULT_JSON_MODE);
    let json_mode = match JsonMode::parse(json_mode_name) {
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

    if args.max_tokens == Some(0) {
        eprintln!("humanify: --max-tokens must be greater than 0");
        return 64;
    }

    let sentinels = match build_sentinel_spec(
        args.start_sentinel.as_deref(),
        args.stop_sentinel.as_deref(),
        args.sentinel_strict,
        args.sentinel_expand_helpers,
    ) {
        Ok(spec) => spec,
        Err(msg) => {
            eprintln!("humanify: {msg}");
            return 64;
        }
    };

    let extra = match args.extra_body.as_deref().map(parse_extra_body).transpose() {
        Ok(map) => map.unwrap_or_default(),
        Err(msg) => {
            eprintln!("humanify: {msg}");
            return 64;
        }
    };
    let body_options = BodyOptions {
        max_tokens: args.max_tokens,
        extra,
    };

    let env_key = if api_key_from_cli {
        None
    } else {
        env_api_key(defaults.api_key_env)
    };

    let context_size = args.context_size.unwrap_or(DEFAULT_CONTEXT_SIZE);

    // `--cache-context-size` decouples the window a cache key is built from the
    // window the model is shown, so runs at different `--context-size` values
    // still share entries. CLI > env > follow `--context-size`.
    let cache_context_size_env = match env::var_os("HUMANIFY_CACHE_CONTEXT_SIZE") {
        Some(raw) => {
            let text = raw.to_string_lossy().into_owned();
            match text.trim().parse::<usize>() {
                Ok(n) => Some(n),
                Err(_) => {
                    eprintln!(
                        "humanify: HUMANIFY_CACHE_CONTEXT_SIZE must be a positive integer, got {text:?}"
                    );
                    return 64;
                }
            }
        }
        None => None,
    };
    let cache_context_size_requested = args.cache_context_size.or(cache_context_size_env);

    // Deliberately *not* validated against `--context-size`. Keying on a wider
    // window than the model is shown is the intended setup for the cheap half of
    // a mixed-cost run: pin one key size, then vary `--context-size` freely above
    // and below it.
    if cache_context_size_requested == Some(0) {
        eprintln!("humanify: --cache-context-size must be greater than 0");
        return 64;
    }
    let cache_context_size = cache_context_size_requested.unwrap_or(context_size);

    let cfg = PresetConfig {
        base_url: args
            .base_url
            .unwrap_or_else(|| defaults.base_url.to_string()),
        model: args.model.unwrap_or_else(|| defaults.model.to_string()),
        api_key: args.api_key.or(env_key),
        json_mode,
        context_size,
        cache_context_size,
        verbose: args.verbose,
        body_options,
    };
    let output = args.output;
    let timeout_seconds = args.timeout_seconds.unwrap_or(defaults.timeout_seconds);

    let cache_dir = if args.no_cache {
        None
    } else {
        args.cache_dir
            .or_else(|| env::var_os("HUMANIFY_CACHE_DIR").map(PathBuf::from))
    };

    // `--refresh-cache` re-asks the model and overwrites stored answers, so it is
    // meaningless without a cache to overwrite. Both combinations below would
    // otherwise be silent no-ops, and the likely intent (re-run at a higher cost
    // setting) would be missed while still spending the money.
    if args.refresh_cache {
        if args.no_cache {
            eprintln!("humanify: --refresh-cache cannot be combined with --no-cache");
            return 64;
        }
        if cache_dir.is_none() {
            eprintln!(
                "humanify: --refresh-cache needs a cache to refresh; \
                 set --cache-dir <DIR> or HUMANIFY_CACHE_DIR"
            );
            return 64;
        }
    }

    // Same reasoning for `--cache-context-size`: it does nothing at all without a
    // cache, and silently accepting it would let a mixed-cost workflow run to
    // completion at full price under the illusion that keys were being shared.
    //
    // Only the *explicit* flag is an error. `HUMANIFY_CACHE_CONTEXT_SIZE` is meant
    // to be exported once and left alone, so a `--no-cache` run must be able to
    // ignore it rather than refuse to start.
    if cache_context_size_from_cli {
        if args.no_cache {
            eprintln!("humanify: --cache-context-size cannot be combined with --no-cache");
            return 64;
        }
        if cache_dir.is_none() {
            eprintln!(
                "humanify: --cache-context-size needs a cache to key; \
                 set --cache-dir <DIR> or HUMANIFY_CACHE_DIR"
            );
            return 64;
        }
    }

    let max_retries = args.max_retries.unwrap_or(3);

    if cfg.verbose {
        print_verbose_config(VerboseConfigOptions {
            input: &args.input,
            output: output.as_deref(),
            cfg: &cfg,
            defaults,
            sources: ConfigSources {
                model_from_cli,
                api_key_from_cli,
                base_url_from_cli,
                context_size_from_cli,
                cache_context_size_from_cli,
                json_mode_from_cli,
                timeout_from_cli,
                cache_dir_from_cli,
                no_cache_from_cli,
                max_retries_from_cli,
                max_run_seconds_from_cli,
                max_tokens_from_cli,
            },
            timeout_seconds,
            cache_dir: cache_dir.as_deref(),
            refresh_cache: args.refresh_cache,
            max_retries,
            max_run_seconds: args.max_run_seconds,
            sentinels: sentinels.as_ref(),
            dry_run: args.dry_run,
        });
    }

    let source = match pipe::read_input(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("humanify: failed to read input: {e}");
            return 1;
        }
    };

    let options = RenameOptions {
        context_size: cfg.context_size,
        cache_context_size: cfg.cache_context_size,
        sentinels,
    };

    // `--dry-run` answers "did I select the right region?" for free. It returns
    // before any HTTP client, tokio runtime or cache exists, so a mis-aimed
    // window costs nothing to discover.
    if args.dry_run {
        return run_dry_run(&source, &options);
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("humanify: failed to create tokio runtime: {e}");
            return 1;
        }
    };

    let timeout = std::time::Duration::from_secs(timeout_seconds);
    let client = HttpClient::with_timeout(timeout);
    let ladder = Arc::new(build_ladder(client, &cfg, defaults.provider_kind));

    // `--max-run-seconds` is documented as having no upper bound, so a value
    // large enough to overflow the platform's `Instant` range must degrade to
    // "unlimited" rather than panic on the addition.
    let deadline = args
        .max_run_seconds
        .filter(|s| *s > 0)
        .and_then(|s| std::time::Instant::now().checked_add(std::time::Duration::from_secs(s)));

    let llm = LlmRenamer::new(Arc::clone(&ladder), rt.handle().clone())
        .with_retry(RetryPolicy {
            max_retries,
            ..Default::default()
        })
        .with_deadline(deadline);

    let mut renamer: Box<dyn Renamer + Send> = Box::new(llm);
    renamer = Box::new(BudgetRenamer::new(renamer, deadline));

    let mut cache_stats: Option<Arc<CacheStats>> = None;
    if let Some(dir) = &cache_dir {
        match DiskCache::open(dir) {
            Ok(cache) => {
                let scope = CacheScope {
                    provider: defaults.name,
                    model: cfg.model.clone(),
                    base_url: cfg.base_url.trim_end_matches('/').to_string(),
                    json_mode: cfg.json_mode.as_str().to_string(),
                };
                let c = CachingRenamer::new(renamer, cache, scope)
                    .with_refresh(args.refresh_cache)
                    .with_context_sizes(cfg.context_size, cfg.cache_context_size);
                cache_stats = Some(c.stats());
                renamer = Box::new(c);
            }
            Err(e) => eprintln!("humanify: cache disabled ({e})"),
        }
    }

    let mut observer =
        CliObserver::new(cfg.verbose, args.progress, Arc::clone(&ladder), cache_stats);

    let joined = rt.block_on(async move {
        tokio::task::spawn_blocking(move || {
            let res = rename_all_identifiers_with_options(
                &source,
                &mut *renamer,
                &options,
                &mut observer,
            );
            (res, observer)
        })
        .await
    });

    let (result, mut observer) = match joined {
        Ok(pair) => pair,
        Err(join_err) => {
            eprintln!("humanify: internal error: {join_err}");
            return 1;
        }
    };

    observer.finish();

    let renamed = match result {
        Ok(s) => s,
        Err(RenameError::Parse(msg)) => {
            eprintln!("humanify: parse error: {msg}");
            return 2;
        }
        Err(RenameError::Sentinel(msg)) => {
            eprintln!("humanify: {msg}");
            return 64;
        }
    };

    if let Err(e) = pipe::write_output(output.as_deref(), &renamed) {
        eprintln!("humanify: failed to write output: {e}");
        return 1;
    }

    0
}

/// Validate the sentinel flags and resolve any `@file` indirection.
///
/// Returns `None` when no window was requested — the whole file, i.e. the
/// behaviour humanify has always had.
fn build_sentinel_spec(
    start: Option<&str>,
    stop: Option<&str>,
    strict: bool,
    expand_helpers: bool,
) -> Result<Option<SentinelSpec>, String> {
    if strict && expand_helpers {
        return Err(
            "--sentinel-strict and --sentinel-expand-helpers select different \
                    selection policies and cannot be combined"
                .to_string(),
        );
    }

    if start.is_none() && stop.is_none() {
        // Without a window both policy flags are silent no-ops (the whole file is
        // selected either way), and the likely intent — renaming one region —
        // would be missed while still spending the money.
        if strict || expand_helpers {
            return Err(
                "--sentinel-strict and --sentinel-expand-helpers need a window; \
                        pass --start-sentinel and/or --stop-sentinel"
                    .to_string(),
            );
        }
        return Ok(None);
    }

    let policy = if strict {
        SelectionPolicy::Strict
    } else if expand_helpers {
        SelectionPolicy::ExpandHelpers
    } else {
        SelectionPolicy::DeclOrReference
    };

    Ok(Some(SentinelSpec {
        start: start
            .map(|arg| sentinel::parse_arg("--start-sentinel", arg))
            .transpose()?,
        stop: stop
            .map(|arg| sentinel::parse_arg("--stop-sentinel", arg))
            .transpose()?,
        policy,
    }))
}

/// Resolve the window, apply the filter, print what was selected, and stop.
///
/// Deliberately does not construct a `Renamer` that can talk to anything: the
/// identity renamer below is the only one the walker ever sees, so "zero LLM
/// calls" is a property of the wiring rather than a promise.
fn run_dry_run(source: &str, options: &RenameOptions) -> i32 {
    struct NoCallRenamer;
    impl Renamer for NoCallRenamer {
        fn rename(&mut self, original: &str, _surrounding: &str) -> String {
            original.to_string()
        }
    }

    #[derive(Default)]
    struct DryRunObserver {
        report: Option<SentinelReport>,
        names: Vec<String>,
    }
    impl RenameObserver for DryRunObserver {
        fn sentinel_window(&mut self, report: &SentinelReport) {
            self.report = Some(report.clone());
        }
        fn rename_started(&mut self, _current: usize, _total: usize, original: &str) {
            self.names.push(original.to_string());
        }
    }

    let mut renamer = NoCallRenamer;
    let mut observer = DryRunObserver::default();
    match rename_all_identifiers_with_options(source, &mut renamer, options, &mut observer) {
        Ok(_) => {}
        Err(RenameError::Parse(msg)) => {
            eprintln!("humanify: parse error: {msg}");
            return 2;
        }
        Err(RenameError::Sentinel(msg)) => {
            eprintln!("humanify: {msg}");
            return 64;
        }
    }

    eprintln!("humanify: dry run: no LLM calls made, no output written");
    match &observer.report {
        Some(report) => eprintln!("humanify: sentinel window: {report}"),
        None => eprintln!(
            "humanify: whole file: {} identifiers selected",
            observer.names.len()
        ),
    }
    if observer.names.is_empty() {
        eprintln!("humanify: selected identifiers: (none)");
    } else {
        // Sorted and deduped rather than shown in rename order: a minified
        // region binds `e` in a dozen scopes, and a reader checking "did I catch
        // `_D`, and did `xue` come along?" wants a scannable set, not the work
        // queue. The counts above are the authoritative per-binding numbers.
        let mut names = observer.names;
        names.sort_unstable();
        names.dedup();
        eprintln!("humanify: selected identifiers: {}", names.join(", "));
    }

    0
}

#[derive(Clone, Copy)]
struct ConfigSources {
    model_from_cli: bool,
    api_key_from_cli: bool,
    base_url_from_cli: bool,
    context_size_from_cli: bool,
    cache_context_size_from_cli: bool,
    json_mode_from_cli: bool,
    timeout_from_cli: bool,
    cache_dir_from_cli: bool,
    no_cache_from_cli: bool,
    max_retries_from_cli: bool,
    max_run_seconds_from_cli: bool,
    max_tokens_from_cli: bool,
}

struct VerboseConfigOptions<'a> {
    input: &'a str,
    output: Option<&'a Path>,
    cfg: &'a PresetConfig,
    defaults: PresetDefaults,
    sources: ConfigSources,
    timeout_seconds: u64,
    cache_dir: Option<&'a Path>,
    refresh_cache: bool,
    max_retries: u32,
    max_run_seconds: Option<u64>,
    sentinels: Option<&'a SentinelSpec>,
    dry_run: bool,
}

fn print_verbose_config(opts: VerboseConfigOptions<'_>) {
    let sources = opts.sources;
    let cfg = opts.cfg;
    let defaults = opts.defaults;
    let timeout_seconds = opts.timeout_seconds;
    let cache_dir = opts.cache_dir;
    let refresh_cache = opts.refresh_cache;
    let max_retries = opts.max_retries;
    let max_run_seconds = opts.max_run_seconds;
    let input = opts.input;
    let output = opts.output;
    let source = |from_cli| {
        if from_cli {
            "command line"
        } else {
            "default"
        }
    };
    eprintln!("* provider: {}", defaults.name);
    eprintln!(
        "* model: {} ({})",
        cfg.model,
        source(sources.model_from_cli)
    );
    eprintln!(
        "* base URL: {} ({})",
        cfg.base_url,
        source(sources.base_url_from_cli)
    );
    match (&cfg.api_key, sources.api_key_from_cli) {
        (Some(_), true) => eprintln!("* API key: set (command line)"),
        (Some(_), false) => eprintln!("* API key: set ({})", defaults.api_key_env),
        (None, _) => eprintln!("* API key: not set"),
    }
    eprintln!(
        "* JSON mode: {} ({})",
        cfg.json_mode.as_str(),
        source(sources.json_mode_from_cli)
    );
    eprintln!(
        "* context size: {} ({})",
        cfg.context_size,
        source(sources.context_size_from_cli)
    );
    // Only worth a line when it actually differs. The wrapper always runs with
    // `-v` and logs stderr, so this is the one place a mis-pinned key window
    // becomes visible before a whole run is paid for at a zero hit rate.
    if cfg.cache_context_size != cfg.context_size {
        let src = if sources.cache_context_size_from_cli {
            "command line"
        } else {
            "HUMANIFY_CACHE_CONTEXT_SIZE"
        };
        eprintln!(
            "* cache key context: {} ({src}) [prompt context: {}]",
            cfg.cache_context_size, cfg.context_size
        );
    }
    eprintln!(
        "* timeout: {timeout_seconds}s ({})",
        source(sources.timeout_from_cli)
    );
    if let Some(dir) = cache_dir {
        let src = if sources.cache_dir_from_cli {
            "command line"
        } else {
            "HUMANIFY_CACHE_DIR"
        };
        eprintln!("* cache: {} ({src})", dir.display());
        if refresh_cache {
            eprintln!("* cache mode: refresh (re-asking the model, overwriting stored answers)");
        }
    } else if sources.no_cache_from_cli {
        eprintln!("* cache: disabled (command line)");
    } else {
        eprintln!("* cache: disabled");
    }
    match cfg.body_options.max_tokens {
        Some(n) => eprintln!(
            "* max tokens: {n} ({})",
            source(sources.max_tokens_from_cli)
        ),
        None => eprintln!("* max tokens: unset (provider default)"),
    }
    if !cfg.body_options.extra.is_empty() {
        eprintln!("* extra body: {}", cfg.body_options.extra_display());
    }
    eprintln!(
        "* retries: {max_retries} ({})",
        source(sources.max_retries_from_cli)
    );
    if let Some(s) = max_run_seconds.filter(|s| *s > 0) {
        eprintln!(
            "* max run: {s}s ({})",
            source(sources.max_run_seconds_from_cli)
        );
    } else {
        eprintln!("* max run: unlimited");
    }
    match opts.sentinels {
        Some(spec) => {
            match &spec.start {
                Some(fragment) => eprintln!("* start sentinel: {fragment:?}"),
                None => eprintln!("* start sentinel: unset (start of file)"),
            }
            match &spec.stop {
                Some(fragment) => eprintln!("* stop sentinel: {fragment:?}"),
                None => eprintln!("* stop sentinel: unset (end of file)"),
            }
            eprintln!("* selection policy: {}", policy_name(spec.policy));
        }
        None => eprintln!("* sentinels: unset (whole file)"),
    }
    if opts.dry_run {
        eprintln!("* dry run: selection only, no LLM calls");
    }
    eprintln!("* input: {input}");
    match output {
        Some(path) => eprintln!("* output: {}", path.display()),
        None => eprintln!("* output: stdout"),
    }
}

fn policy_name(policy: SelectionPolicy) -> &'static str {
    match policy {
        SelectionPolicy::Strict => "declaration in window (--sentinel-strict)",
        SelectionPolicy::DeclOrReference => "declaration or reference in window (default)",
        SelectionPolicy::ExpandHelpers => {
            "declaration or reference in window, plus pulled-in helper bodies \
             (--sentinel-expand-helpers)"
        }
    }
}

const PROGRESS_BAR_WIDTH: usize = 30;

struct CliObserver {
    verbose: bool,
    progress: bool,
    progress_is_terminal: bool,
    completed: usize,
    total: usize,
    last_logged_percent: Option<usize>,
    displayed_width: usize,
    ladder: Arc<Ladder>,
    reported_strategy: Option<&'static str>,
    cache_stats: Option<Arc<CacheStats>>,
    changed: usize,
    unchanged: usize,
    failed: usize,
    skipped: usize,
}

impl CliObserver {
    fn new(
        verbose: bool,
        progress: bool,
        ladder: Arc<Ladder>,
        cache_stats: Option<Arc<CacheStats>>,
    ) -> Self {
        Self {
            verbose,
            progress,
            progress_is_terminal: io::stderr().is_terminal(),
            completed: 0,
            total: 0,
            last_logged_percent: None,
            displayed_width: 0,
            ladder,
            reported_strategy: None,
            cache_stats,
            changed: 0,
            unchanged: 0,
            failed: 0,
            skipped: 0,
        }
    }

    /// End-of-run accounting. `changed + unchanged + failed + skipped == total`,
    /// so the four categories are exhaustive and disjoint. Cache hits are
    /// reported separately because they are a *source* for an answer, not an
    /// outcome: a hit still lands in `changed` or `unchanged`.
    fn finish(&mut self) {
        self.clear_terminal_progress();
        let notable = self.failed > 0 || self.skipped > 0 || self.verbose;
        if notable {
            eprintln!(
                "humanify: {} identifiers: {} changed, {} unchanged, {} failed, {} skipped",
                self.total, self.changed, self.unchanged, self.failed, self.skipped
            );
        }
        if let Some(stats) = &self.cache_stats {
            let hits = stats.hits.load(Ordering::Relaxed);
            let misses = stats.misses.load(Ordering::Relaxed);
            let bypassed = stats.bypassed.load(Ordering::Relaxed);
            if notable || hits > 0 || misses > 0 || bypassed > 0 {
                let refreshed = if bypassed > 0 {
                    format!(", {bypassed} refreshed")
                } else {
                    String::new()
                };
                eprintln!(
                    "humanify: cache: {} hits, {} misses{}, {} writes, {} write errors",
                    hits,
                    misses,
                    refreshed,
                    stats.writes.load(Ordering::Relaxed),
                    stats.write_errors.load(Ordering::Relaxed),
                );
            }
        }
        if self.skipped > 0 {
            eprintln!(
                "humanify: PARTIAL: {} of {} identifiers were left unrenamed (run budget exhausted). Re-run with the same --cache-dir to continue from here.",
                self.skipped, self.total
            );
        }
    }

    fn take_changed_strategy(&mut self) -> Option<&'static str> {
        let strategy = self.ladder.locked_strategy_name();
        if strategy.is_some() && strategy != self.reported_strategy {
            self.reported_strategy = strategy;
            strategy
        } else {
            None
        }
    }

    fn clear_terminal_progress(&mut self) {
        if !self.progress || !self.progress_is_terminal || self.displayed_width == 0 {
            return;
        }

        eprint!(
            "\r{blank:width$}\r",
            blank = "",
            width = self.displayed_width
        );
        let _ = io::stderr().flush();
        self.displayed_width = 0;
    }

    fn draw_terminal_progress(&mut self) {
        if !self.progress || !self.progress_is_terminal {
            return;
        }

        let line = render_progress(self.completed, self.total);
        eprint!("\r{line}");
        let _ = io::stderr().flush();
        self.displayed_width = line.len();
        if self.completed == self.total {
            eprintln!();
            self.displayed_width = 0;
        }
    }

    fn log_progress_snapshot(&mut self) {
        if !self.progress || self.progress_is_terminal {
            return;
        }

        let percent = self
            .completed
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(100);
        let should_log = self.last_logged_percent.is_none()
            || self.completed == self.total
            || percent >= self.last_logged_percent.unwrap_or(0) + 10;
        if should_log {
            eprintln!("{}", render_progress(self.completed, self.total));
            self.last_logged_percent = Some(percent);
        }
    }
}

impl RenameObserver for CliObserver {
    /// Printed unconditionally, not just under `--verbose`: a mis-aimed window
    /// spends real money, and this makes it obvious in the first second of a run
    /// rather than after it finishes.
    fn sentinel_window(&mut self, report: &SentinelReport) {
        eprintln!("humanify: sentinel window: {report}");
        if report.selected == 0 {
            eprintln!(
                "humanify: nothing to do — the window contains no renameable identifiers. \
                 Re-check the fragments with --dry-run."
            );
        }
    }

    fn identifiers_found(&mut self, total: usize) {
        self.total = total;
        if self.verbose {
            eprintln!("* found {total} identifiers");
        }
        self.draw_terminal_progress();
        self.log_progress_snapshot();
    }

    fn rename_started(&mut self, current: usize, total: usize, original: &str) {
        if self.verbose {
            self.clear_terminal_progress();
            eprintln!("* [{current}/{total}] renaming `{original}`");
            self.draw_terminal_progress();
        }
    }

    fn rename_finished(&mut self, current: usize, total: usize, original: &str, renamed: &str) {
        if original == renamed {
            self.unchanged += 1;
        } else {
            self.changed += 1;
        }
        if self.verbose {
            self.clear_terminal_progress();
            if let Some(strategy) = self.take_changed_strategy() {
                eprintln!("* selected JSON strategy: {strategy}");
            }
            eprintln!("* [{current}/{total}] `{original}` -> `{renamed}`");
        }
        self.completed = current;
        self.total = total;
        self.draw_terminal_progress();
        self.log_progress_snapshot();
    }

    fn rename_failed(&mut self, current: usize, total: usize, original: &str, reason: &str) {
        self.failed += 1;
        self.completed = current;
        self.total = total;
        // Printed unconditionally, unlike the `->` lines: a silently swallowed
        // failure is exactly what made a failed call indistinguishable from a
        // deliberate "this name is already good". This is the only per-identifier
        // line a non-verbose run emits, so it stays greppable.
        self.clear_terminal_progress();
        eprintln!("* [{current}/{total}] `{original}` FAILED: {reason}");
        self.draw_terminal_progress();
        self.log_progress_snapshot();
    }

    fn rename_skipped(&mut self, current: usize, total: usize, original: &str, reason: &str) {
        self.skipped += 1;
        self.completed = current;
        self.total = total;
        // Counters first, then one clear/draw pass — a budget-exhausted run can
        // skip thousands of identifiers, and redrawing twice per skip flickers.
        // Only verbose prints the line; the run-level `PARTIAL:` line is the
        // summary everyone else needs.
        if self.verbose {
            self.clear_terminal_progress();
            eprintln!("* [{current}/{total}] `{original}` SKIPPED: {reason}");
        }
        self.draw_terminal_progress();
        self.log_progress_snapshot();
    }

    fn cache_hit(&mut self, _current: usize, _total: usize, _original: &str) {}
}

fn render_progress(completed: usize, total: usize) -> String {
    let completed = completed.min(total);
    let filled = completed
        .saturating_mul(PROGRESS_BAR_WIDTH)
        .checked_div(total)
        .unwrap_or(PROGRESS_BAR_WIDTH);
    let bar = if completed == total {
        "=".repeat(PROGRESS_BAR_WIDTH)
    } else {
        format!(
            "{}>{}",
            "=".repeat(filled),
            "-".repeat(PROGRESS_BAR_WIDTH - filled - 1)
        )
    };
    format!("[{bar}] {completed}/{total} identifiers")
}

fn build_ladder(client: HttpClient, cfg: &PresetConfig, kind: ProviderKind) -> Ladder {
    match cfg.json_mode {
        JsonMode::Ladder => build_default_ladder(client, cfg, kind),
        JsonMode::OpenAIJsonSchema => Ladder::pinned(Arc::new(
            OpenAIJsonSchema::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )),
        JsonMode::ForcedToolCall => Ladder::pinned(Arc::new(
            ForcedToolCall::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )),
        JsonMode::ToolCallAndPrompt => Ladder::pinned(Arc::new(
            ToolCallAndPrompt::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )),
        JsonMode::Prompt => Ladder::pinned(Arc::new(
            PromptToJson::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )),
        JsonMode::AnthropicNative => Ladder::pinned(Arc::new(
            AnthropicNativeJsonSchema::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )),
    }
}

pub(crate) fn build_default_ladder(
    client: HttpClient,
    cfg: &PresetConfig,
    kind: ProviderKind,
) -> Ladder {
    let strategies: Vec<Arc<dyn JsonStrategy>> = match kind {
        ProviderKind::OpenAICompat => vec![
            Arc::new(
                OpenAIJsonSchema::new(
                    client.clone(),
                    cfg.base_url.clone(),
                    cfg.api_key.clone(),
                    cfg.model.clone(),
                )
                .with_body_options(cfg.body_options.clone()),
            ),
            Arc::new(
                ForcedToolCall::new(
                    client.clone(),
                    cfg.base_url.clone(),
                    cfg.api_key.clone(),
                    cfg.model.clone(),
                )
                .with_body_options(cfg.body_options.clone()),
            ),
            Arc::new(
                PromptToJson::new(
                    client,
                    cfg.base_url.clone(),
                    cfg.api_key.clone(),
                    cfg.model.clone(),
                )
                .with_body_options(cfg.body_options.clone()),
            ),
        ],
        // AnthropicNativeJsonSchema uses a beta API whose response shape we
        // haven't validated against a live call — its parser frequently rejects
        // real responses as "no JSON block in content", and the ladder cannot
        // fall back from a Transient error. Default to the tool-call strategy,
        // which is well-tested. AnthropicNativeJsonSchema is still reachable
        // via `--json-mode anthropic-native` for anyone wanting to opt in.
        ProviderKind::Anthropic => vec![Arc::new(
            AnthropicToolCallAndPrompt::new(
                client,
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                cfg.model.clone(),
            )
            .with_body_options(cfg.body_options.clone()),
        )],
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
                "unknown json-mode '{other}'. Valid values: ladder, openai-json-schema, \
                 anthropic-native, forced-tool-call, tool-call-and-prompt, prompt"
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
        assert_eq!(crate::cli::openai::DEFAULTS.model, "gpt-5-mini");
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
            context_size: None,
            cache_context_size: None,
            json_mode: Some(json_mode.to_string()),
            verbose: false,
            progress: false,
            timeout_seconds: None,
            cache_dir: None,
            no_cache: false,
            refresh_cache: false,
            max_retries: None,
            max_run_seconds: None,
            max_tokens: None,
            extra_body: None,
            start_sentinel: None,
            stop_sentinel: None,
            sentinel_strict: false,
            sentinel_expand_helpers: false,
            dry_run: false,
        }
    }

    /// Both knobs are validated before any input is read, so a typo costs
    /// nothing and never half-runs a file.
    fn body_args(max_tokens: Option<u32>, extra_body: Option<&str>) -> PresetArgs {
        PresetArgs {
            max_tokens,
            extra_body: extra_body.map(str::to_string),
            ..preset_args_no_io("ladder")
        }
    }

    #[test]
    fn zero_max_tokens_returns_64() {
        let code = run_preset(body_args(Some(0), None), crate::cli::openai::DEFAULTS);
        assert_eq!(code, 64);
    }

    #[test]
    fn malformed_extra_body_returns_64() {
        let code = run_preset(
            body_args(None, Some("{not json")),
            crate::cli::openai::DEFAULTS,
        );
        assert_eq!(code, 64);
    }

    #[test]
    fn reserved_key_in_extra_body_returns_64() {
        let code = run_preset(
            body_args(None, Some(r#"{"messages":[]}"#)),
            crate::cli::openai::DEFAULTS,
        );
        assert_eq!(code, 64);
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
            cache_context_size: 500,
            verbose: false,
            body_options: BodyOptions::default(),
        }
    }

    #[test]
    fn anthropic_default_ladder_uses_tool_call_only() {
        // The native json-schema strategy lives behind `--json-mode anthropic-native`;
        // the ladder ships tool-call exclusively because the native path's response
        // shape isn't validated end-to-end yet.
        let ladder = build_default_ladder(
            HttpClient::new(),
            &anthropic_preset_cfg(),
            ProviderKind::Anthropic,
        );
        assert_eq!(ladder.strategy_count(), 1);
    }

    // --- build_sentinel_spec ---

    fn spec_of(start: Option<&str>, stop: Option<&str>) -> Option<SentinelSpec> {
        build_sentinel_spec(start, stop, false, false).expect("expected a valid spec")
    }

    #[test]
    fn no_sentinel_flags_yields_no_spec() {
        assert_eq!(spec_of(None, None), None);
    }

    #[test]
    fn start_only_defaults_to_policy_b() {
        let spec = spec_of(Some("frag"), None).expect("a spec");
        assert_eq!(spec.start.as_deref(), Some("frag"));
        assert_eq!(spec.stop, None);
        assert_eq!(spec.policy, SelectionPolicy::DeclOrReference);
    }

    #[test]
    fn strict_flag_selects_policy_a() {
        let spec = build_sentinel_spec(Some("frag"), None, true, false)
            .unwrap()
            .unwrap();
        assert_eq!(spec.policy, SelectionPolicy::Strict);
    }

    #[test]
    fn expand_helpers_flag_selects_policy_c() {
        let spec = build_sentinel_spec(Some("frag"), None, false, true)
            .unwrap()
            .unwrap();
        assert_eq!(spec.policy, SelectionPolicy::ExpandHelpers);
    }

    #[test]
    fn strict_and_expand_helpers_conflict() {
        let msg = build_sentinel_spec(Some("frag"), None, true, true).unwrap_err();
        assert!(msg.contains("cannot be combined"), "{msg}");
    }

    #[test]
    fn a_policy_flag_without_a_window_is_an_error() {
        // Silently selecting the whole file would miss the likely intent while
        // still spending the money.
        let msg = build_sentinel_spec(None, None, true, false).unwrap_err();
        assert!(msg.contains("--start-sentinel"), "{msg}");
    }

    #[test]
    fn an_empty_fragment_is_an_error() {
        assert!(build_sentinel_spec(Some(""), None, false, false).is_err());
    }

    #[test]
    fn a_fragment_reads_from_an_at_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stop.txt");
        std::fs::write(&path, "return n.filter(Boolean)\n").unwrap();
        let spec = spec_of(None, Some(&format!("@{}", path.display()))).expect("a spec");
        assert_eq!(spec.stop.as_deref(), Some("return n.filter(Boolean)"));
    }

    // --- run_preset sentinel paths (no network reached) ---

    /// Points at an unroutable port, so any run that *did* try to call a
    /// provider would fail loudly rather than pass by accident.
    fn sentinel_args(input: &str, start: Option<&str>, dry_run: bool) -> PresetArgs {
        PresetArgs {
            input: input.to_string(),
            base_url: Some("http://127.0.0.1:1/v1".to_string()),
            api_key: Some("test".to_string()),
            start_sentinel: start.map(str::to_string),
            dry_run,
            ..preset_args_no_io("ladder")
        }
    }

    fn temp_js(contents: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{contents}").unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn conflicting_policy_flags_return_64_before_any_io() {
        let args = PresetArgs {
            start_sentinel: Some("frag".to_string()),
            sentinel_strict: true,
            sentinel_expand_helpers: true,
            ..preset_args_no_io("ladder")
        };
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 64);
    }

    #[test]
    fn an_unmatched_fragment_returns_64() {
        let file = temp_js("const a = 1; const b = 2;");
        let path = file.path().to_str().unwrap().to_string();
        let args = sentinel_args(&path, Some("no such fragment"), false);
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 64);
    }

    #[test]
    fn an_ambiguous_fragment_returns_64() {
        let file = temp_js("const a = 1; const b = 2;");
        let path = file.path().to_str().unwrap().to_string();
        let args = sentinel_args(&path, Some("const "), false);
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 64);
    }

    #[test]
    fn dry_run_succeeds_without_reaching_a_provider() {
        // The base URL is unroutable, so a zero exit code is only possible if no
        // client was ever built.
        let file = temp_js("const a = 1; const b = 2; const c = 3;");
        let path = file.path().to_str().unwrap().to_string();
        let args = sentinel_args(&path, Some("const b"), true);
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 0);
    }

    #[test]
    fn dry_run_writes_no_output_file() {
        let file = temp_js("const a = 1; const b = 2;");
        let path = file.path().to_str().unwrap().to_string();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.js");
        let args = PresetArgs {
            output: Some(out.clone()),
            ..sentinel_args(&path, Some("const b"), true)
        };
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 0);
        assert!(!out.exists(), "a dry run must not write output");
    }

    #[test]
    fn dry_run_without_sentinels_still_reports_and_makes_no_calls() {
        let file = temp_js("const a = 1;");
        let path = file.path().to_str().unwrap().to_string();
        let args = sentinel_args(&path, None, true);
        assert_eq!(run_preset(args, crate::cli::openai::DEFAULTS), 0);
    }

    #[test]
    fn progress_bar_starts_empty() {
        assert_eq!(
            render_progress(0, 4),
            "[>-----------------------------] 0/4 identifiers"
        );
    }

    #[test]
    fn progress_bar_shows_partial_completion() {
        assert_eq!(
            render_progress(2, 4),
            "[===============>--------------] 2/4 identifiers"
        );
    }

    #[test]
    fn progress_bar_finishes_full() {
        assert_eq!(
            render_progress(4, 4),
            "[==============================] 4/4 identifiers"
        );
    }
}
