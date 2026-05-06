use std::collections::VecDeque;

use super::{rename_all_identifiers, Renamer};

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

pub struct RecordingRenamer {
    suffix: String,
    pub log: CallLog,
}

/// Captures `(original, surrounding)` pairs for each rename call.
#[derive(Default, Clone)]
pub struct CallLog(pub Vec<(String, String)>);

impl Renamer for RecordingRenamer {
    fn rename(&mut self, original: &str, surrounding: &str) -> String {
        self.log
            .0
            .push((original.to_string(), surrounding.to_string()));
        format!("{original}{}", self.suffix)
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

pub fn recording(sfx: &str) -> RecordingRenamer {
    RecordingRenamer {
        suffix: sfx.to_string(),
        log: CallLog::default(),
    }
}

// --- ScenarioBuilder ---

pub struct ScenarioBuilder {
    source: String,
    context_size: usize,
}

pub fn scenario(source: &str) -> ScenarioBuilder {
    ScenarioBuilder {
        source: source.to_string(),
        context_size: 200,
    }
}

impl ScenarioBuilder {
    pub fn with_context_size(mut self, n: usize) -> Self {
        self.context_size = n;
        self
    }

    pub fn renamed_with<R: Renamer>(self, mut renamer: R) -> RenamedScenario {
        let result = rename_all_identifiers(&self.source, &mut renamer, self.context_size);
        RenamedScenario {
            output: result.expect("rename_all_identifiers failed"),
        }
    }

    pub fn with_recording(self, mut renamer: RecordingRenamer) -> (RenamedScenario, CallLog) {
        let result = rename_all_identifiers(&self.source, &mut renamer, self.context_size);
        let log = renamer.log;
        (
            RenamedScenario {
                output: result.expect("rename_all_identifiers failed"),
            },
            log,
        )
    }

    pub fn parses_unchanged(self) {
        let output = rename_all_identifiers(&self.source, &mut IdentityRenamer, self.context_size)
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
