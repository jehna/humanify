mod collision;
mod safe_name;
pub mod sentinel;
#[cfg(test)]
pub mod test_dsl;
mod walker;

pub use sentinel::{SelectionPolicy, SentinelReport, SentinelSpec, SentinelWindow};
pub use walker::{
    rename_all_identifiers, rename_all_identifiers_with_observer,
    rename_all_identifiers_with_options, RenameOptions,
};

/// The single reason string used for every outcome caused by the run budget,
/// wherever it is detected: before a call (`BudgetRenamer`), during one, or
/// during a retry backoff (`LlmRenamer`). Shared so the CLI can report all three
/// the same way.
pub const BUDGET_EXHAUSTED_REASON: &str = "run budget exhausted";

/// What a `Renamer` actually did for one identifier.
#[derive(Debug, Clone, PartialEq)]
pub enum RenameOutcome {
    /// The model answered. `String` is its raw suggestion, which may legitimately
    /// equal the original name (the prompt asks for already-meaningful names to be
    /// returned unchanged).
    Ok(String),
    /// The call failed (network, rate limit, malformed response, all strategies
    /// dead). The original name is kept. Never cached.
    Failed { reason: String },
    /// No call was attempted (run budget exhausted). The original name is kept.
    /// Never cached.
    Skipped { reason: String },
}

/// One identifier's rename request, carrying **two** views of the same
/// neighbourhood.
///
/// They exist as separate fields because `--cache-context-size` decouples them:
/// `surrounding` is what the model is shown, `cache_context` is what the cache
/// key and the entry's verification hash are built from. Both are produced by
/// the *same* extraction routine over the *same* original source, differing only
/// in the size passed to it — never by slicing one out of the other, which would
/// diverge at file boundaries (see `compute_context_window`).
///
/// When `--cache-context-size` is unset the two are identical, and every key is
/// exactly what it would have been without the flag.
pub struct RenameRequest<'a> {
    pub original: &'a str,
    pub surrounding: &'a str,
    pub cache_context: &'a str,
}

impl<'a> RenameRequest<'a> {
    /// The prompt window doubles as the key window — the default, and what every
    /// caller that has no opinion about caching should use.
    pub fn new(original: &'a str, surrounding: &'a str) -> Self {
        Self {
            original,
            surrounding,
            cache_context: surrounding,
        }
    }

    pub fn with_cache_context(
        original: &'a str,
        surrounding: &'a str,
        cache_context: &'a str,
    ) -> Self {
        Self {
            original,
            surrounding,
            cache_context,
        }
    }
}

pub trait Renamer {
    /// Returns the new name for the identifier. Returning the same string means "leave it alone".
    fn rename(&mut self, original: &str, surrounding_code: &str) -> String;

    /// Like `rename`, but distinguishes a real answer from a failed or skipped
    /// call. The default treats every result as an answer, so existing
    /// implementations (all of `rename::test_dsl`) keep working untouched.
    ///
    /// Note for decorators: this takes the whole [`RenameRequest`] rather than
    /// loose strings so that a link in the chain cannot quietly drop
    /// `cache_context` and send the cache layer a key window it did not ask for.
    /// Forward the request; do not rebuild it.
    fn try_rename(&mut self, req: &RenameRequest<'_>) -> RenameOutcome {
        RenameOutcome::Ok(self.rename(req.original, req.surrounding))
    }
}

/// Every identifier produces **exactly one** terminal event: `rename_finished`,
/// `rename_failed`, or `rename_skipped` — never two.
///
/// This matters to anything counting consecutive failures. `rename_finished` with
/// `original == renamed` is a legitimate success (the prompt tells the model to
/// return already-meaningful names unchanged), so a caller cannot infer failure
/// from name equality; conversely, a failure that also emitted `rename_finished`
/// would let every failure reset the caller's own failure counter.
pub trait RenameObserver {
    /// A sentinel window was resolved and applied. Emitted once, before
    /// `identifiers_found`, and only when `--start-sentinel`/`--stop-sentinel`
    /// were given.
    fn sentinel_window(&mut self, _report: &SentinelReport) {}

    fn identifiers_found(&mut self, _total: usize) {}

    fn rename_started(&mut self, _current: usize, _total: usize, _original: &str) {}

    /// Terminal: the renamer answered. `renamed` may equal `original`.
    fn rename_finished(&mut self, _current: usize, _total: usize, _original: &str, _renamed: &str) {
    }

    /// Terminal: the LLM call for `original` failed; the original name was kept.
    fn rename_failed(&mut self, _current: usize, _total: usize, _original: &str, _reason: &str) {}

    /// Terminal: no answer was obtained for `original` because the run budget ran
    /// out — before the call, during it, or during a retry backoff. The original
    /// name was kept and the identifier is worth re-running.
    fn rename_skipped(&mut self, _current: usize, _total: usize, _original: &str, _reason: &str) {}

    /// The rename for `original` was served from the on-disk cache.
    fn cache_hit(&mut self, _current: usize, _total: usize, _original: &str) {}
}

pub struct NoopRenameObserver;

impl RenameObserver for NoopRenameObserver {}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("failed to parse JavaScript: {0}")]
    Parse(String),
    /// A `--start-sentinel` / `--stop-sentinel` fragment did not resolve to
    /// exactly one usable window. A CLI usage error (exit 64), not a failure of
    /// the input: nothing was read from the model and nothing was written.
    #[error("{0}")]
    Sentinel(String),
}
