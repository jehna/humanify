use std::collections::VecDeque;

use super::sentinel::{SelectionPolicy, SentinelReport, SentinelSpec};
use super::{
    rename_all_identifiers_with_options, NoopRenameObserver, RenameError, RenameObserver,
    RenameOptions, RenameOutcome, RenameRequest, Renamer,
};

// --- Renamer constructors ---

pub struct FixedRenamer(String);
impl Renamer for FixedRenamer {
    fn rename(&mut self, _: &str, _: &str) -> String {
        self.0.clone()
    }
}

pub struct QueueRenamer(VecDeque<String>);
impl Renamer for QueueRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        self.0.pop_front().unwrap_or_else(|| original.to_string())
    }
}

pub struct SuffixRenamer(String);
impl Renamer for SuffixRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        format!("{original}{}", self.0)
    }
}

pub struct IdentityRenamer;
impl Renamer for IdentityRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        original.to_string()
    }
}

/// Always fails. `rename` (the fallback path) returns the original.
pub struct FailingRenamer(String);
impl Renamer for FailingRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        original.to_string()
    }
    fn try_rename(&mut self, _req: &RenameRequest<'_>) -> RenameOutcome {
        RenameOutcome::Failed {
            reason: self.0.clone(),
        }
    }
}

/// Always skips. `rename` (the fallback path) returns the original.
pub struct SkippingRenamer(String);
impl Renamer for SkippingRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        original.to_string()
    }
    fn try_rename(&mut self, _req: &RenameRequest<'_>) -> RenameOutcome {
        RenameOutcome::Skipped {
            reason: self.0.clone(),
        }
    }
}

/// Renames based on the original name via a lookup table; unmapped names are
/// left unchanged. Order-independent, unlike `queue`, which makes it convenient
/// for testing scope-aware collision behaviour regardless of traversal order.
pub struct MapRenamer(std::collections::HashMap<String, String>);
impl Renamer for MapRenamer {
    fn rename(&mut self, original: &str, _: &str) -> String {
        self.0
            .get(original)
            .cloned()
            .unwrap_or_else(|| original.to_string())
    }
}

pub struct RecordingRenamer {
    suffix: String,
    pub log: CallLog,
}

/// Captures `(original, surrounding, cache_context)` triples for each rename
/// call. The third element is what the cache would key on, which is only
/// observable here — the cache layer itself hashes it away.
#[derive(Default, Clone)]
pub struct CallLog(pub Vec<(String, String, String)>);

impl CallLog {
    pub fn call_names(&self) -> Vec<&str> {
        self.0.iter().map(|(n, _, _)| n.as_str()).collect()
    }

    pub fn scope_for(&self, name: &str) -> &str {
        self.0
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, s, _)| s.as_str())
            .unwrap_or_else(|| panic!("no call recorded for '{name}'"))
    }

    /// The window the cache key would be built from for `name`.
    pub fn cache_context_for(&self, name: &str) -> &str {
        self.0
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, _, c)| c.as_str())
            .unwrap_or_else(|| panic!("no call recorded for '{name}'"))
    }
}

impl Renamer for RecordingRenamer {
    fn rename(&mut self, original: &str, surrounding: &str) -> String {
        match self.try_rename(&RenameRequest::new(original, surrounding)) {
            RenameOutcome::Ok(n) => n,
            _ => original.to_string(),
        }
    }

    fn try_rename(&mut self, req: &RenameRequest<'_>) -> RenameOutcome {
        self.log.0.push((
            req.original.to_string(),
            req.surrounding.to_string(),
            req.cache_context.to_string(),
        ));
        RenameOutcome::Ok(format!("{}{}", req.original, self.suffix))
    }
}

pub fn fixed(name: &str) -> FixedRenamer {
    FixedRenamer(name.to_string())
}

pub fn queue(names: &[&str]) -> QueueRenamer {
    QueueRenamer(names.iter().map(|s| s.to_string()).collect())
}

pub fn suffix(sfx: &str) -> SuffixRenamer {
    SuffixRenamer(sfx.to_string())
}

pub fn identity() -> IdentityRenamer {
    IdentityRenamer
}

pub fn failing(reason: &str) -> FailingRenamer {
    FailingRenamer(reason.to_string())
}

pub fn skipping(reason: &str) -> SkippingRenamer {
    SkippingRenamer(reason.to_string())
}

pub fn recording(sfx: &str) -> RecordingRenamer {
    RecordingRenamer {
        suffix: sfx.to_string(),
        log: CallLog::default(),
    }
}

pub fn mapping(pairs: &[(&str, &str)]) -> MapRenamer {
    MapRenamer(
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    )
}

// --- ScenarioBuilder ---

pub struct ScenarioBuilder {
    source: String,
    context_size: usize,
    cache_context_size: Option<usize>,
    sentinels: Option<SentinelSpec>,
}

pub fn scenario(source: &str) -> ScenarioBuilder {
    ScenarioBuilder {
        source: source.to_string(),
        context_size: 200,
        cache_context_size: None,
        sentinels: None,
    }
}

/// Captures the resolved-window report so a test can assert on the counts the
/// CLI would print.
#[derive(Default)]
struct ReportObserver(Option<SentinelReport>);

impl RenameObserver for ReportObserver {
    fn sentinel_window(&mut self, report: &SentinelReport) {
        self.0 = Some(report.clone());
    }
}

impl ScenarioBuilder {
    pub fn with_context_size(mut self, n: usize) -> Self {
        self.context_size = n;
        self
    }

    /// `--cache-context-size`. Unset means it follows `context_size`.
    pub fn with_cache_context_size(mut self, n: usize) -> Self {
        self.cache_context_size = Some(n);
        self
    }

    /// Bound the run by a start fragment.
    pub fn from(mut self, fragment: &str) -> Self {
        self.sentinels
            .get_or_insert_with(SentinelSpec::default)
            .start = Some(fragment.to_string());
        self
    }

    /// Bound the run by a stop fragment.
    pub fn until(mut self, fragment: &str) -> Self {
        self.sentinels
            .get_or_insert_with(SentinelSpec::default)
            .stop = Some(fragment.to_string());
        self
    }

    pub fn between(self, start: &str, stop: &str) -> Self {
        self.from(start).until(stop)
    }

    pub fn with_policy(mut self, policy: SelectionPolicy) -> Self {
        self.sentinels
            .get_or_insert_with(SentinelSpec::default)
            .policy = policy;
        self
    }

    fn options(&self) -> RenameOptions {
        RenameOptions {
            context_size: self.context_size,
            cache_context_size: self.cache_context_size.unwrap_or(self.context_size),
            sentinels: self.sentinels.clone(),
        }
    }
    fn run(&self, renamer: &mut dyn Renamer) -> String {
        rename_all_identifiers_with_options(
            &self.source,
            renamer,
            &self.options(),
            &mut NoopRenameObserver,
        )
        .expect("rename_all_identifiers failed")
    }

    pub fn renamed_with<R: Renamer>(self, mut renamer: R) -> RenamedScenario {
        RenamedScenario {
            output: self.run(&mut renamer),
        }
    }

    pub fn with_recording(self, mut renamer: RecordingRenamer) -> (RenamedScenario, CallLog) {
        let output = self.run(&mut renamer);
        (RenamedScenario { output }, renamer.log)
    }

    /// Like `with_recording`, but also hands back the window report the CLI
    /// would print.
    pub fn with_recorded_report(
        self,
        mut renamer: RecordingRenamer,
    ) -> (CallLog, Option<SentinelReport>) {
        let mut observer = ReportObserver::default();
        rename_all_identifiers_with_options(
            &self.source,
            &mut renamer,
            &self.options(),
            &mut observer,
        )
        .expect("rename_all_identifiers failed");
        (renamer.log, observer.0)
    }

    /// Asserts the run fails sentinel resolution, and returns the message.
    pub fn sentinel_error(self) -> String {
        let result = rename_all_identifiers_with_options(
            &self.source,
            &mut IdentityRenamer,
            &self.options(),
            &mut super::NoopRenameObserver,
        );
        match result {
            Err(RenameError::Sentinel(msg)) => msg,
            Err(other) => panic!("expected a sentinel error, got {other:?}"),
            Ok(_) => panic!("expected a sentinel error, but the run succeeded"),
        }
    }

    pub fn parses_unchanged(self) {
        let output = rename_all_identifiers_with_options(
            &self.source,
            &mut IdentityRenamer,
            &self.options(),
            &mut super::NoopRenameObserver,
        )
        .expect("rename_all_identifiers failed");
        let got = output.trim_end_matches('\n');
        assert_eq!(
            got,
            self.source.trim_end_matches('\n'),
            "source did not round-trip"
        );
    }
}

pub struct RenamedScenario {
    output: String,
}

impl RenamedScenario {
    pub fn yields(self, expected: &str) {
        let got = self.output.trim_end_matches('\n');
        assert_eq!(got, expected, "renamed output mismatch");
    }

    pub fn output(&self) -> &str {
        self.output.trim_end_matches('\n')
    }
}

// --- IdentifierAssertion (for safe_name tests) ---

pub struct IdentifierAssertion {
    raw: String,
    result: String,
}

pub fn to_identifier_of(raw: &str) -> IdentifierAssertion {
    IdentifierAssertion {
        raw: raw.to_string(),
        result: super::safe_name::to_identifier(raw),
    }
}

impl IdentifierAssertion {
    pub fn is(self, expected: &str) {
        assert_eq!(
            self.result, expected,
            "to_identifier({:?}) should be {expected:?}",
            self.raw
        );
    }

    pub fn is_reserved(self) {
        assert!(
            super::safe_name::is_reserved_word(&self.raw),
            "{:?} should be reserved",
            self.raw
        );
    }

    pub fn is_not_reserved(self) {
        assert!(
            !super::safe_name::is_reserved_word(&self.raw),
            "{:?} should not be reserved",
            self.raw
        );
    }
}
