use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::runtime::Handle;

use crate::llm::ladder::Ladder;
use crate::rename::{RenameOutcome, RenameRequest, Renamer, BUDGET_EXHAUSTED_REASON};

pub const SYSTEM_PROMPT: &str = "You are a senior software engineer reviewing minified or obfuscated JavaScript. Your job is to assign a single descriptive identifier name based on how the variable is used in the surrounding code. Return JSON only.";

pub const USER_PROMPT_TEMPLATE: &str = "Surrounding code:\n```javascript\n{surrounding_code}\n```\n\nThe identifier currently named `{original}` appears in this code. Suggest a single descriptive replacement name. Rules:\n- camelCase for variables and functions, PascalCase for classes/constructors\n- ASCII letters, digits, underscores only; first character must be a letter or underscore\n- Avoid JavaScript reserved words\n- If the current name is already meaningful, return it unchanged";

pub fn render_user_prompt(original: &str, surrounding_code: &str) -> String {
    USER_PROMPT_TEMPLATE
        .replace("{original}", original)
        .replace("{surrounding_code}", surrounding_code)
}

pub fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "description": "The replacement identifier name."
                }
            }
        })
    })
}

/// Manually maintained version of everything about a request that the fingerprint
/// below **cannot** see for itself: the JSON strategies' payload shapes, the
/// ladder's ordering, the response parsers, and any model parameters.
///
/// Bump it when provider wire behaviour changes. Prompt text and schema changes
/// are picked up automatically (they are hashed in below); nothing else is.
///
/// Note this is **provenance only** — see [`crate::cache::cache_key`], which no
/// longer includes the fingerprint. Changing the prompt or bumping this version
/// does not invalidate a warm cache on its own; `--refresh-cache` and a separate
/// `--cache-dir` are the ways to force fresh answers.
pub const REQUEST_SHAPE_VERSION: &[u8] = b"humanify-request-shape-v1";

/// Hex sha256 of everything that defines the request shape: the manually
/// maintained [`REQUEST_SHAPE_VERSION`] plus the prompts and schema. Recorded on
/// each cache entry so the request shape that produced a name stays inspectable.
pub fn prompt_fingerprint() -> &'static str {
    static FP: OnceLock<String> = OnceLock::new();
    FP.get_or_init(|| {
        crate::cache::sha256_hex_parts(&[
            REQUEST_SHAPE_VERSION,
            SYSTEM_PROMPT.as_bytes(),
            USER_PROMPT_TEMPLATE.as_bytes(),
            schema().to_string().as_bytes(),
        ])
    })
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
        }
    }
}

/// Bridges the synchronous `Renamer` trait to the async `Ladder` via
/// `tokio::runtime::Handle::block_on`.
///
/// # Panics
///
/// `rename` panics if called from within the same tokio runtime that `runtime`
/// belongs to. The walker is sync; call it from a non-async thread (e.g. wrap
/// the walker call in `spawn_blocking` if you are in an async context).
pub struct LlmRenamer {
    ladder: Arc<Ladder>,
    runtime: Handle,
    retry: RetryPolicy,
    deadline: Option<Instant>,
}

impl LlmRenamer {
    pub fn new(ladder: Arc<Ladder>, runtime: Handle) -> Self {
        Self {
            ladder,
            runtime,
            retry: RetryPolicy::default(),
            deadline: None,
        }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_deadline(mut self, deadline: Option<Instant>) -> Self {
        self.deadline = deadline;
        self
    }

    fn backoff_delay(&self, attempt: u32) -> Duration {
        let mult = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
        let base_delay = self.retry.base_delay_ms.saturating_mul(mult);
        let capped = base_delay.min(self.retry.max_delay_ms);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let jitter_factor = (nanos % 250) as f64 / 1000.0;
        let jitter_ms = (capped as f64 * jitter_factor) as u64;
        Duration::from_millis(capped.saturating_add(jitter_ms))
    }

    /// Runs one ladder call, bounded by the run deadline when one is set.
    ///
    /// Returns `None` if the deadline passed while the request was still in
    /// flight. Without this the budget would only be checked *between*
    /// identifiers, so a request started one millisecond before the deadline
    /// could still run for the full per-request timeout — up to 1800s on Ollama.
    fn call_within_budget(
        &self,
        user: &str,
        schema: &Value,
    ) -> Option<Result<Value, crate::llm::http::StrategyError>> {
        match self.deadline {
            None => Some(
                self.runtime
                    .block_on(self.ladder.call(SYSTEM_PROMPT, user, schema)),
            ),
            Some(dl) => {
                let at = tokio::time::Instant::from_std(dl);
                let call = self.ladder.call(SYSTEM_PROMPT, user, schema);
                self.runtime
                    .block_on(async move { tokio::time::timeout_at(at, call).await.ok() })
            }
        }
    }

    fn budget_exhausted(&self) -> bool {
        self.deadline.is_some_and(|dl| Instant::now() >= dl)
    }
}

fn extract_name(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

impl Renamer for LlmRenamer {
    fn try_rename(&mut self, req: &RenameRequest<'_>) -> RenameOutcome {
        let original = req.original;
        if original.is_empty() {
            return RenameOutcome::Ok(String::new());
        }
        // The prompt window, never the cache window: `--cache-context-size`
        // changes only how an answer is filed, never what the model is asked.
        let user = render_user_prompt(original, req.surrounding);
        let schema = schema();

        let mut attempt = 0u32;
        loop {
            if self.budget_exhausted() {
                return RenameOutcome::Skipped {
                    reason: BUDGET_EXHAUSTED_REASON.to_string(),
                };
            }

            // `None` = the deadline cut the request short. That is a budget
            // outcome, not a failure: reporting it as `Skipped` is what makes the
            // run emit the resumable `PARTIAL:` line instead of counting it as an
            // error the user should investigate.
            let Some(result) = self.call_within_budget(&user, schema) else {
                return RenameOutcome::Skipped {
                    reason: BUDGET_EXHAUSTED_REASON.to_string(),
                };
            };

            match result {
                Ok(value) => {
                    return match extract_name(&value) {
                        Some(n) => RenameOutcome::Ok(n),
                        // A well-formed response with no usable `name` is a
                        // model-behaviour problem, not a transient one: the
                        // identical prompt would produce the same answer. Don't
                        // spend retries on it. The observer prints the failure —
                        // printing here too would double every failure line.
                        None => RenameOutcome::Failed {
                            reason: format!("response had no valid 'name' for `{original}`"),
                        },
                    };
                }
                Err(e) => {
                    if !e.is_transient() || attempt >= self.retry.max_retries {
                        return RenameOutcome::Failed {
                            reason: e.to_string(),
                        };
                    }
                    let delay = self.backoff_delay(attempt);
                    // Never sleep past the run budget.
                    if let Some(dl) = self.deadline {
                        let wakes_too_late = Instant::now()
                            .checked_add(delay)
                            .is_none_or(|wake| wake >= dl);
                        if wakes_too_late {
                            return RenameOutcome::Skipped {
                                reason: BUDGET_EXHAUSTED_REASON.to_string(),
                            };
                        }
                    }
                    eprintln!(
                        "humanify: retry {}/{} for `{original}` in {}ms: {e}",
                        attempt + 1,
                        self.retry.max_retries,
                        delay.as_millis()
                    );
                    self.runtime
                        .block_on(async move { tokio::time::sleep(delay).await });
                    attempt += 1;
                }
            }
        }
    }

    fn rename(&mut self, original: &str, surrounding_code: &str) -> String {
        match self.try_rename(&RenameRequest::new(original, surrounding_code)) {
            RenameOutcome::Ok(n) => n,
            _ => original.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    use serde_json::json;

    use crate::llm::test_dsl::{not_supported, ok, script, ScriptedResponse, ScriptedStrategy};

    fn make_renamer(strategy: Arc<ScriptedStrategy>) -> (LlmRenamer, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(strategy));
        let renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 0,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        (renamer, rt)
    }

    #[test]
    fn successful_rename() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"name":"splitString"})));
        assert_eq!(r.rename("a", "..."), "splitString");
    }

    #[test]
    fn ladder_transient_falls_back_to_original() {
        let (mut r, _rt) = make_renamer(script(
            "s",
            vec![ScriptedResponse::Transient("network error".into())],
        ));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_missing_name() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"other":"x"})));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_not_string() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"name":42})));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_empty_string() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"name":""})));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_whitespace() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"name":"  "})));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_with_extras() {
        let (mut r, _rt) = make_renamer(ok("s", json!({"name":"fooBar","extra":"ignored"})));
        assert_eq!(r.rename("foo", "..."), "fooBar");
    }

    #[test]
    fn ladder_response_top_level_array() {
        let (mut r, _rt) = make_renamer(ok("s", json!(["foo"])));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_top_level_string() {
        let (mut r, _rt) = make_renamer(ok("s", json!("foo")));
        assert_eq!(r.rename("foo", "..."), "foo");
    }

    #[test]
    fn all_strategies_dead_falls_back() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::new(vec![
            not_supported("s0", "no") as Arc<dyn crate::llm::JsonStrategy>,
            not_supported("s1", "no") as Arc<dyn crate::llm::JsonStrategy>,
        ]));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone());
        assert_eq!(renamer.rename("bar", "..."), "bar");
    }

    #[test]
    fn empty_original_no_call() {
        let s = ok("s", json!({}));
        let count = s.call_count.clone();
        let (mut r, _rt) = make_renamer(s);
        assert_eq!(r.rename("", "..."), "");
        assert_eq!(count.load(SeqCst), 0);
    }

    #[test]
    fn system_and_user_prompt_shape() {
        let s = ok("s", json!({"name":"result"}));
        let recorded = s.recorded.clone();
        let (mut r, _rt) = make_renamer(s);
        r.rename("foo", "const foo = 1;");
        let calls = recorded.lock().unwrap();
        let (system, user, schema) = &calls[0];
        assert!(
            system.contains("senior software engineer"),
            "system: {system}"
        );
        assert!(
            user.contains("const foo = 1;"),
            "user should contain surrounding code: {user}"
        );
        assert!(
            user.contains("`foo`"),
            "user should contain original name: {user}"
        );
        assert_eq!(
            schema["required"],
            json!(["name"]),
            "schema required: {schema}"
        );
    }

    #[test]
    fn multiple_renames_share_one_runtime() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Ok(json!({"name":"alpha"})),
                ScriptedResponse::Ok(json!({"name":"beta"})),
                ScriptedResponse::Ok(json!({"name":"gamma"})),
            ],
        );
        let count = s.call_count.clone();
        let (mut r, _rt) = make_renamer(s);
        assert_eq!(r.rename("a", "..."), "alpha");
        assert_eq!(r.rename("b", "..."), "beta");
        assert_eq!(r.rename("c", "..."), "gamma");
        assert_eq!(count.load(SeqCst), 3);
    }

    #[test]
    fn render_user_prompt_expands_correctly() {
        let prompt = render_user_prompt("foo", "const foo = 1;");
        assert!(prompt.contains("`foo`"));
        assert!(prompt.contains("const foo = 1;"));
        assert!(!prompt.contains("{original}"));
        assert!(!prompt.contains("{surrounding_code}"));
    }

    #[test]
    fn render_user_prompt_preserves_literal_original_in_code() {
        let prompt = render_user_prompt("foo", "const bar = '{original}';");
        assert!(prompt.contains("const bar = '{original}';"));
    }

    #[test]
    fn transient_then_success_retries_and_succeeds() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Transient("rate limited".into()),
                ScriptedResponse::Ok(json!({"name":"successVal"})),
            ],
        );
        let count = s.call_count.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(s));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 2,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        let res = renamer.try_rename(&RenameRequest::new("a", "const a = 1;"));
        assert_eq!(res, RenameOutcome::Ok("successVal".into()));
        assert_eq!(count.load(SeqCst), 2);
    }

    #[test]
    fn transient_exhausts_retries_then_fails() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Transient("err 1".into()),
                ScriptedResponse::Transient("err 2".into()),
                ScriptedResponse::Transient("err 3".into()),
            ],
        );
        let count = s.call_count.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(s));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 2,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        let res = renamer.try_rename(&RenameRequest::new("a", "const a = 1;"));
        assert!(matches!(res, RenameOutcome::Failed { .. }));
        assert_eq!(count.load(SeqCst), 3);
    }

    #[test]
    fn zero_retries_matches_old_behaviour() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Transient("err 1".into()),
                ScriptedResponse::Ok(json!({"name":"won't reach"})),
            ],
        );
        let count = s.call_count.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(s));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 0,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        let res = renamer.try_rename(&RenameRequest::new("a", "const a = 1;"));
        assert!(matches!(res, RenameOutcome::Failed { .. }));
        assert_eq!(count.load(SeqCst), 1);
    }

    #[test]
    fn missing_name_does_not_retry() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Ok(json!({"other":"x"})),
                ScriptedResponse::Ok(json!({"name":"retry"})),
            ],
        );
        let count = s.call_count.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(s));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 3,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        let res = renamer.try_rename(&RenameRequest::new("foo", "const foo = 1;"));
        assert!(matches!(res, RenameOutcome::Failed { .. }));
        assert_eq!(count.load(SeqCst), 1);
    }

    /// Builds a renamer over `ladder` with fast, deterministic backoff.
    fn renamer_with(
        ladder: Arc<Ladder>,
        max_retries: u32,
    ) -> (LlmRenamer, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        (r, rt)
    }

    // --- retry classification ---
    //
    // These pin the cost of a misconfigured run. Before `StrategyError::Permanent`
    // existed, `Ladder` handed `LlmRenamer` a `Transient` for a bad API key, an
    // invalid model, a pinned-but-unsupported strategy and an exhausted ladder
    // alike — so each of those cost `max_retries + 1` requests plus seconds of
    // backoff *per identifier*, for a run that could never succeed.

    #[test]
    fn permanent_error_does_not_retry() {
        let s = script(
            "s",
            vec![
                ScriptedResponse::Permanent("http 401: invalid api key".into()),
                ScriptedResponse::Ok(json!({"name":"never reached"})),
            ],
        );
        let count = s.call_count.clone();
        let (mut r, _rt) = renamer_with(Arc::new(Ladder::pinned(s)), 3);
        assert!(matches!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Failed { .. }
        ));
        assert_eq!(count.load(SeqCst), 1, "a bad key must cost exactly 1 call");
    }

    #[test]
    fn pinned_not_supported_does_not_retry() {
        let s = script("s", vec![ScriptedResponse::NotSupported("nope".into())]);
        let count = s.call_count.clone();
        let (mut r, _rt) = renamer_with(Arc::new(Ladder::pinned(s)), 3);
        assert!(matches!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Failed { .. }
        ));
        assert_eq!(count.load(SeqCst), 1);
    }

    #[test]
    fn all_strategies_dead_does_not_retry() {
        let s0 = not_supported("s0", "no");
        let s1 = not_supported("s1", "no");
        let (c0, c1) = (s0.call_count.clone(), s1.call_count.clone());
        let ladder = Arc::new(Ladder::new(vec![
            s0 as Arc<dyn crate::llm::JsonStrategy>,
            s1 as Arc<dyn crate::llm::JsonStrategy>,
        ]));
        let (mut r, _rt) = renamer_with(ladder, 3);
        assert!(matches!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Failed { .. }
        ));
        assert_eq!(c0.load(SeqCst), 1);
        assert_eq!(c1.load(SeqCst), 1);
    }

    // --- run budget ---

    #[test]
    fn deadline_in_the_past_skips_without_calling() {
        let s = ok("s", json!({"name":"never reached"}));
        let count = s.call_count.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut r = LlmRenamer::new(Arc::new(Ladder::pinned(s)), rt.handle().clone())
            .with_deadline(Some(Instant::now() - Duration::from_secs(1)));
        assert_eq!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Skipped {
                reason: BUDGET_EXHAUSTED_REASON.to_string()
            }
        );
        assert_eq!(count.load(SeqCst), 0);
    }

    /// A deadline reached during backoff is a *budget* outcome, not a failure —
    /// otherwise a run that exhausts its budget on its only identifier reports
    /// `failed` and never prints the resumable `PARTIAL:` line.
    #[test]
    fn budget_exhausted_during_backoff_is_a_skip_not_a_failure() {
        let s = script(
            "s",
            vec![ScriptedResponse::Transient("rate limited".into())],
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut r = LlmRenamer::new(Arc::new(Ladder::pinned(s)), rt.handle().clone())
            .with_retry(RetryPolicy {
                max_retries: 3,
                base_delay_ms: 60_000,
                max_delay_ms: 60_000,
            })
            .with_deadline(Some(Instant::now() + Duration::from_millis(50)));
        assert_eq!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Skipped {
                reason: BUDGET_EXHAUSTED_REASON.to_string()
            }
        );
    }

    #[test]
    fn a_generous_deadline_does_not_interfere() {
        let s = ok("s", json!({"name":"fine"}));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut r = LlmRenamer::new(Arc::new(Ladder::pinned(s)), rt.handle().clone())
            .with_deadline(Some(Instant::now() + Duration::from_secs(3600)));
        assert_eq!(
            r.try_rename(&RenameRequest::new("a", "const a = 1;")),
            RenameOutcome::Ok("fine".into())
        );
    }

    #[test]
    fn failure_returns_failed_not_ok_original() {
        let s = script(
            "s",
            vec![ScriptedResponse::Transient("network down".into())],
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(s));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone()).with_retry(RetryPolicy {
            max_retries: 0,
            base_delay_ms: 1,
            max_delay_ms: 5,
        });
        let res = renamer.try_rename(&RenameRequest::new("foo", "const foo = 1;"));
        assert_eq!(
            res,
            RenameOutcome::Failed {
                reason: "network down".into()
            }
        );
    }
}
