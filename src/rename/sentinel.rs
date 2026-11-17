//! Sentinel-bounded renaming: restrict the work list to the symbols visible in
//! one region of the input.
//!
//! A sentinel is a **literal fragment of the input file**, resolved to a byte
//! offset by plain text search — not an identifier name, not a line number, not
//! a regex. The deciding property is user-verifiable uniqueness: you select a
//! distinctive run of characters, hit Ctrl+F, see "1 of 1", and *know* the
//! sentinel is stable before spending a cent. An identifier name cannot offer
//! that, because scope-local reuse of short names is the norm in minified code
//! and deciding whether a name is a unique *binding* needs the very scope
//! analysis you don't have in your editor.
//!
//! It also means the input file is never edited, so every existing cache entry
//! stays valid — pointing at code that is already there doesn't merely make the
//! cache problem tractable, it deletes it.
//!
//! Byte offsets are safe to filter on because nothing sits between parse and
//! print: no transform, no folding, no elimination pass. Every `Span` oxc hands
//! back is an offset into the exact string `pipe::read_input` returned.

use std::fmt;
use std::fs;

/// Which symbols a resolved window admits.
///
/// The governing principle for the default is: *rename every identifier that is
/// visible in the region I am reading*, where visible means declared there or
/// referenced there. That rule terminates on its own; anything beyond it needs
/// an arbitrary cutoff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionPolicy {
    /// Policy A: the declaration must fall inside the window. `--sentinel-strict`.
    Strict,
    /// Policy B (default): declaration **or** any reference inside the window.
    /// A helper declared hundreds of lines earlier but *called* in the window is
    /// renamed, because `parseColorCodes(e)` at the call site tells you far more
    /// than `xue(e)`. Its internals are not — you are not going to read the
    /// helper's body, and paying a model to name its loop counters is waste.
    #[default]
    DeclOrReference,
    /// Policy C: B, plus everything lexically inside the declaration of a symbol
    /// B pulled in *by reference only*. `--sentinel-expand-helpers`, for when the
    /// callee is your own code and reading it is the point. Deliberately one
    /// level — a fixed point over a bundler runtime is "the entire file".
    ExpandHelpers,
}

/// What the user asked for, before it is resolved against the source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SentinelSpec {
    /// Literal fragment marking the start of the window.
    pub start: Option<String>,
    /// Literal fragment marking the end of the window.
    pub stop: Option<String>,
    pub policy: SelectionPolicy,
}

impl SentinelSpec {
    /// True when at least one fragment was given. A spec with neither selects
    /// the whole file and is indistinguishable from no spec at all.
    pub fn is_bounded(&self) -> bool {
        self.start.is_some() || self.stop.is_some()
    }
}

/// A resolved byte range over the raw input, plus the spans of the fragments
/// that defined it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentinelWindow {
    /// Inclusive start: the *start* of the start-match (or 0).
    pub start: u32,
    /// Exclusive end: the *end* of the stop-match (or the end of the file).
    /// Both fragments are therefore inside the window, which is what a user
    /// pointing at "from here to there" expects.
    pub end: u32,
    /// Spans of the matched fragments themselves, used by
    /// [`SentinelWindow::is_inserted_marker`].
    pub marks: Vec<(u32, u32)>,
    pub policy: SelectionPolicy,
}

impl SentinelWindow {
    /// Whether a byte offset falls inside the window.
    pub fn contains(&self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }

    /// Whether the binding named `name`, declared at `[start, end)`, is a
    /// hand-inserted marker rather than code that was already there.
    ///
    /// A marker is an ordinary binding sitting on the window boundary, so a
    /// reference-aware policy would offer it to the model — wasting a call and
    /// destroying the marker for the next run of the same file.
    ///
    /// The test is deliberately narrow: the fragment must be *exactly* this
    /// identifier. Merely overlapping the declaration is not enough, because
    /// `--start-sentinel 'function xue('` is a supported way to point at
    /// existing code, and excluding `xue` from renaming there would silently
    /// drop the very identifier the user aimed at. A fragment that is nothing
    /// but one identifier is the shape a marker actually takes
    /// (`--start-sentinel 'START_SENTINEL'`).
    pub fn is_inserted_marker(&self, source: &str, start: u32, end: u32, name: &str) -> bool {
        self.marks.iter().any(|&(mark_start, mark_end)| {
            start < mark_end
                && mark_start < end
                && source
                    .get(mark_start as usize..mark_end as usize)
                    .map(str::trim)
                    == Some(name)
        })
    }
}

/// The one-line summary printed at the top of a sentinel run, so a mis-aimed
/// window is obvious in the first second rather than after the bill arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentinelReport {
    pub start: u32,
    pub end: u32,
    pub start_line: usize,
    pub end_line: usize,
    /// Identifiers that survived the filter.
    pub selected: usize,
    /// Identifiers in the whole file.
    pub total: usize,
}

impl SentinelReport {
    pub fn new(source: &str, window: &SentinelWindow, selected: usize, total: usize) -> Self {
        let (start_line, _) = line_col(source, window.start as usize);
        // The window end is exclusive, so report the line of its last byte.
        let last = (window.end as usize).saturating_sub(1);
        let (end_line, _) = line_col(source, last);
        SentinelReport {
            start: window.start,
            end: window.end,
            start_line,
            end_line,
            selected,
            total,
        }
    }
}

impl fmt::Display for SentinelReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bytes {}..{} (lines {}..{}), {} of {} identifiers selected",
            self.start, self.end, self.start_line, self.end_line, self.selected, self.total
        )
    }
}

/// Resolve a spec against the raw input bytes.
///
/// Zero matches and multiple matches are both hard errors: silently picking the
/// first match, or silently degrading to the whole file, spends real money on
/// the wrong region. Because the user can pre-verify uniqueness in an editor,
/// a hard error costs them nothing and should essentially never fire.
pub fn resolve(source: &str, spec: &SentinelSpec) -> Result<SentinelWindow, String> {
    let start_match = match &spec.start {
        Some(fragment) => Some(find_unique(source, fragment, "--start-sentinel")?),
        None => None,
    };
    let stop_match = match &spec.stop {
        Some(fragment) => Some(find_unique(source, fragment, "--stop-sentinel")?),
        None => None,
    };

    if let (Some(s), Some(e)) = (start_match, stop_match) {
        if e.0 < s.0 {
            let (sl, sc) = line_col(source, s.0 as usize);
            let (el, ec) = line_col(source, e.0 as usize);
            return Err(format!(
                "--stop-sentinel matches at {el}:{ec}, before --start-sentinel at {sl}:{sc}; \
                 the window would be empty"
            ));
        }
    }

    let start = start_match.map_or(0, |(s, _)| s);
    let end = stop_match.map_or(source.len() as u32, |(_, e)| e);
    let marks = [start_match, stop_match].into_iter().flatten().collect();

    Ok(SentinelWindow {
        start,
        end,
        marks,
        policy: spec.policy,
    })
}

/// Locate the single occurrence of `fragment`, or explain why there isn't one.
fn find_unique(source: &str, fragment: &str, flag: &str) -> Result<(u32, u32), String> {
    let hits: Vec<usize> = source.match_indices(fragment).map(|(i, _)| i).collect();
    match hits.len() {
        0 => Err(format!(
            "{flag}: fragment not found in the input: {fragment:?}\n  \
             the fragment is matched against the raw input bytes, so copy it from the input \
             file itself — not from a pretty-printed view of it, and not from a previous \
             run's output"
        )),
        1 => Ok((hits[0] as u32, (hits[0] + fragment.len()) as u32)),
        n => {
            let locations = hits
                .iter()
                .map(|&offset| {
                    let (line, col) = line_col(source, offset);
                    format!("{line}:{col}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "{flag}: fragment matches {n} times ({locations}): {fragment:?}\n  \
                 lengthen the fragment until it is unique — your editor's search will agree"
            ))
        }
    }
}

/// Read a sentinel argument: the fragment itself, or `@path` to read it verbatim
/// from a file.
///
/// Code fragments contain quotes, parens, backslashes and `$`, which are
/// unpleasant to quote on a command line — especially in PowerShell, where
/// backtick is the escape character and `$` interpolates. This mirrors the
/// `@file.json` convention `--extra-body` already uses.
///
/// A single trailing newline is trimmed from a file, since editors add one.
pub fn parse_arg(flag: &str, arg: &str) -> Result<String, String> {
    let text = match arg.strip_prefix('@') {
        Some(path) => {
            let contents = fs::read_to_string(path)
                .map_err(|e| format!("{flag}: cannot read '{path}': {e}"))?;
            trim_one_trailing_newline(&contents).to_string()
        }
        None => arg.to_string(),
    };

    if text.is_empty() {
        return Err(format!("{flag}: fragment must not be empty"));
    }

    Ok(text)
}

fn trim_one_trailing_newline(text: &str) -> &str {
    match text.strip_suffix('\n') {
        Some(rest) => rest.strip_suffix('\r').unwrap_or(rest),
        None => text,
    }
}

/// 1-based line and column (in characters) of a byte offset.
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..floor_char_boundary(source, offset)];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before[line_start..].chars().count() + 1;
    (line, col)
}

fn floor_char_boundary(source: &str, index: usize) -> usize {
    let mut idx = index.min(source.len());
    while !source.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "const a = 1;\nfunction f() {\n  return a;\n}\n";

    fn spec(start: Option<&str>, stop: Option<&str>) -> SentinelSpec {
        SentinelSpec {
            start: start.map(str::to_string),
            stop: stop.map(str::to_string),
            policy: SelectionPolicy::default(),
        }
    }

    fn resolve_ok(source: &str, start: Option<&str>, stop: Option<&str>) -> SentinelWindow {
        resolve(source, &spec(start, stop)).expect("expected the window to resolve")
    }

    fn resolve_err(source: &str, start: Option<&str>, stop: Option<&str>) -> String {
        resolve(source, &spec(start, stop)).expect_err("expected a resolution error")
    }

    // --- window arithmetic ---

    #[test]
    fn both_fragments_are_inside_the_window() {
        let w = resolve_ok(SRC, Some("function f"), Some("return a"));
        assert_eq!(
            &SRC[w.start as usize..w.end as usize],
            "function f() {\n  return a"
        );
    }

    #[test]
    fn start_only_runs_to_end_of_file() {
        let w = resolve_ok(SRC, Some("function f"), None);
        assert_eq!(w.end as usize, SRC.len());
        assert!(SRC[w.start as usize..].starts_with("function f"));
    }

    #[test]
    fn stop_only_runs_from_start_of_file() {
        let w = resolve_ok(SRC, None, Some("= 1;"));
        assert_eq!(w.start, 0);
        assert_eq!(&SRC[..w.end as usize], "const a = 1;");
    }

    #[test]
    fn neither_fragment_is_the_whole_file() {
        let w = resolve_ok(SRC, None, None);
        assert_eq!((w.start as usize, w.end as usize), (0, SRC.len()));
    }

    #[test]
    fn contains_excludes_the_end_offset() {
        let w = resolve_ok(SRC, Some("const"), Some("= 1;"));
        assert!(w.contains(w.start));
        assert!(w.contains(w.end - 1));
        assert!(!w.contains(w.end));
    }

    // --- a fragment need not align with anything ---

    #[test]
    fn fragment_may_span_a_token_boundary() {
        let w = resolve_ok(SRC, Some("t a = 1"), None);
        assert_eq!(w.start as usize, SRC.find("t a = 1").unwrap());
    }

    #[test]
    fn fragment_may_sit_inside_a_string_literal() {
        let src = r#"const a = "function f() {"; function f() {}"#;
        let w = resolve_ok(src, Some(r#""function f() {""#), None);
        assert_eq!(w.start as usize, src.find('"').unwrap());
    }

    // --- hard errors ---

    #[test]
    fn zero_matches_is_an_error_quoting_the_fragment() {
        let msg = resolve_err(SRC, Some("no such text"), None);
        assert!(msg.contains("not found"), "{msg}");
        assert!(msg.contains("no such text"), "{msg}");
    }

    #[test]
    fn multiple_matches_lists_every_occurrence_as_line_col() {
        let src = "var e = 1;\nvar t = 2;\nvar e2 = 3;\n";
        let msg = resolve_err(src, Some("var "), None);
        assert!(msg.contains("matches 3 times"), "{msg}");
        assert!(msg.contains("1:1"), "{msg}");
        assert!(msg.contains("2:1"), "{msg}");
        assert!(msg.contains("3:1"), "{msg}");
    }

    #[test]
    fn stop_before_start_is_an_error() {
        let msg = resolve_err(SRC, Some("return a"), Some("const a"));
        assert!(msg.contains("before --start-sentinel"), "{msg}");
    }

    #[test]
    fn stop_fragment_reports_its_own_flag_name() {
        let msg = resolve_err(SRC, None, Some("nowhere"));
        assert!(msg.contains("--stop-sentinel"), "{msg}");
    }

    // --- the inserted-marker guard ---

    const MARKED: &str = "const a=1; const START_SENTINEL=0; const b=2;";

    #[test]
    fn marks_cover_both_matched_fragments() {
        let w = resolve_ok(SRC, Some("function"), Some("return"));
        assert_eq!(w.marks.len(), 2);
    }

    #[test]
    fn a_fragment_that_is_exactly_the_identifier_is_a_marker() {
        let w = resolve_ok(MARKED, Some("START_SENTINEL"), None);
        let (start, end) = w.marks[0];
        assert!(w.is_inserted_marker(MARKED, start, end, "START_SENTINEL"));
    }

    #[test]
    fn a_longer_fragment_overlapping_a_declaration_is_not_a_marker() {
        // `--start-sentinel 'const START_SENTINEL'` is a user pointing at code,
        // not declaring a marker: the binding must still be renamed.
        let w = resolve_ok(MARKED, Some("const START_SENTINEL"), None);
        let name_start = MARKED.find("START_SENTINEL").unwrap() as u32;
        assert!(!w.is_inserted_marker(
            MARKED,
            name_start,
            name_start + "START_SENTINEL".len() as u32,
            "START_SENTINEL"
        ));
    }

    #[test]
    fn a_marker_fragment_does_not_exclude_a_different_binding() {
        let w = resolve_ok(MARKED, Some("START_SENTINEL"), None);
        let a_start = MARKED.find('a').unwrap() as u32;
        assert!(!w.is_inserted_marker(MARKED, a_start, a_start + 1, "a"));
    }

    #[test]
    fn surrounding_whitespace_in_a_marker_fragment_is_tolerated() {
        let w = resolve_ok(MARKED, Some(" START_SENTINEL"), None);
        let name_start = MARKED.find("START_SENTINEL").unwrap() as u32;
        assert!(w.is_inserted_marker(
            MARKED,
            name_start,
            name_start + "START_SENTINEL".len() as u32,
            "START_SENTINEL"
        ));
    }

    // --- parse_arg ---

    #[test]
    fn inline_fragment_is_taken_verbatim() {
        assert_eq!(parse_arg("--start-sentinel", "a$b`c").unwrap(), "a$b`c");
    }

    #[test]
    fn empty_fragment_is_an_error() {
        assert!(parse_arg("--start-sentinel", "").is_err());
    }

    #[test]
    fn at_path_reads_the_fragment_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("start.txt");
        fs::write(&path, "or(var t=e.split(/\\n+/g\n").unwrap();
        let got = parse_arg("--start-sentinel", &format!("@{}", path.display())).unwrap();
        assert_eq!(
            got, "or(var t=e.split(/\\n+/g",
            "exactly one trailing newline is trimmed"
        );
    }

    #[test]
    fn at_path_trims_only_one_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("start.txt");
        fs::write(&path, "fragment\n\n").unwrap();
        let got = parse_arg("--start-sentinel", &format!("@{}", path.display())).unwrap();
        assert_eq!(got, "fragment\n");
    }

    #[test]
    fn at_path_trims_a_windows_line_ending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("start.txt");
        fs::write(&path, "fragment\r\n").unwrap();
        let got = parse_arg("--start-sentinel", &format!("@{}", path.display())).unwrap();
        assert_eq!(got, "fragment");
    }

    #[test]
    fn missing_at_path_is_an_error() {
        let msg = parse_arg("--start-sentinel", "@no/such/file.txt").unwrap_err();
        assert!(msg.contains("cannot read"), "{msg}");
    }

    #[test]
    fn empty_at_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "\n").unwrap();
        assert!(parse_arg("--stop-sentinel", &format!("@{}", path.display())).is_err());
    }

    // --- line_col ---

    #[test]
    fn line_col_is_one_based() {
        assert_eq!(line_col("abc\ndef", 0), (1, 1));
        assert_eq!(line_col("abc\ndef", 4), (2, 1));
        assert_eq!(line_col("abc\ndef", 6), (2, 3));
    }

    #[test]
    fn line_col_counts_columns_in_characters() {
        // Four 2-byte Cyrillic chars, then `x` at byte 8 / column 5.
        assert_eq!(line_col("оооиx", 8), (1, 5));
    }

    #[test]
    fn line_col_clamps_past_the_end() {
        assert_eq!(line_col("ab", 99), (1, 3));
    }

    // --- report rendering ---

    #[test]
    fn report_renders_bytes_lines_and_counts() {
        let w = resolve_ok(SRC, Some("function f"), Some("return a"));
        let report = SentinelReport::new(SRC, &w, 3, 41);
        assert_eq!(
            report.to_string(),
            format!(
                "bytes {}..{} (lines 2..3), 3 of 41 identifiers selected",
                w.start, w.end
            )
        );
    }

    #[test]
    fn is_bounded_tracks_both_fragments() {
        assert!(!spec(None, None).is_bounded());
        assert!(spec(Some("a"), None).is_bounded());
        assert!(spec(None, Some("a")).is_bounded());
    }
}
