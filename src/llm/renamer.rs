use std::sync::Arc;

use serde_json::json;
use tokio::runtime::Handle;

use crate::llm::ladder::Ladder;
use crate::rename::Renamer;

const SYSTEM_PROMPT: &str = "You are a senior software engineer reviewing minified or obfuscated JavaScript. Your job is to assign a single descriptive identifier name based on how the variable is used in the surrounding code. Return JSON only.";

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
}

impl LlmRenamer {
    pub fn new(ladder: Arc<Ladder>, runtime: Handle) -> Self {
        Self { ladder, runtime }
    }
}

impl Renamer for LlmRenamer {
    fn rename(&mut self, original: &str, surrounding_code: &str) -> String {
        if original.is_empty() {
            return String::new();
        }

        let user = format!(
            "Surrounding code:\n```javascript\n{surrounding_code}\n```\n\nThe identifier currently named `{original}` appears in this code. Suggest a single descriptive replacement name. Rules:\n- camelCase for variables and functions, PascalCase for classes/constructors\n- ASCII letters, digits, underscores only; first character must be a letter or underscore\n- Avoid JavaScript reserved words\n- If the current name is already meaningful, return it unchanged"
        );

        let schema = json!({
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
        });

        let result = self
            .runtime
            .block_on(self.ladder.call(SYSTEM_PROMPT, &user, &schema));

        match result {
            Ok(value) => {
                let name = value
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());

                match name {
                    Some(n) => n.to_string(),
                    None => {
                        eprintln!("humanify: LLM response had no valid 'name' for `{original}`");
                        original.to_string()
                    }
                }
            }
            Err(e) => {
                eprintln!("humanify: LLM call failed for `{original}`: {e}");
                original.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Mutex;

    use anyhow::anyhow;
    use async_trait::async_trait;
    use serde_json::Value;

    use crate::llm::{http::StrategyError, JsonStrategy};

    // Minimal MockStrategy — same shape as in ladder.rs tests.
    enum MockResponse {
        Ok(Value),
        NotSupported(String),
        Transient(String),
    }

    struct MockStrategy {
        strategy_name: &'static str,
        responses: Arc<Mutex<VecDeque<MockResponse>>>,
        pub call_count: Arc<AtomicUsize>,
        // Records (system, user, schema) for each call.
        pub recorded: Arc<Mutex<Vec<(String, String, Value)>>>,
    }

    impl MockStrategy {
        fn new(name: &'static str, responses: Vec<MockResponse>) -> Arc<Self> {
            Arc::new(Self {
                strategy_name: name,
                responses: Arc::new(Mutex::new(VecDeque::from(responses))),
                call_count: Arc::new(AtomicUsize::new(0)),
                recorded: Arc::new(Mutex::new(Vec::new())),
            })
        }
    }

    #[async_trait]
    impl JsonStrategy for MockStrategy {
        async fn call(
            &self,
            system: &str,
            user: &str,
            schema: &Value,
        ) -> Result<Value, StrategyError> {
            self.call_count.fetch_add(1, SeqCst);
            self.recorded.lock().unwrap().push((
                system.to_string(),
                user.to_string(),
                schema.clone(),
            ));
            let next = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    panic!(
                        "MockStrategy '{}' called more times than configured",
                        self.strategy_name
                    )
                });
            match next {
                MockResponse::Ok(v) => Ok(v),
                MockResponse::NotSupported(r) => Err(StrategyError::NotSupported(r)),
                MockResponse::Transient(r) => Err(StrategyError::Transient(anyhow!("{}", r))),
            }
        }

        fn name(&self) -> &'static str {
            self.strategy_name
        }
    }

    fn make_renamer(strategy: Arc<MockStrategy>) -> (LlmRenamer, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::pinned(strategy));
        let renamer = LlmRenamer::new(ladder, rt.handle().clone());
        (renamer, rt)
    }

    fn ok_name(name: &str) -> MockResponse {
        MockResponse::Ok(json!({"name": name}))
    }

    #[test]
    fn successful_rename() {
        let s = MockStrategy::new("s", vec![ok_name("splitString")]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("a", "..."), "splitString");
    }

    #[test]
    fn ladder_transient_falls_back_to_original() {
        let s = MockStrategy::new("s", vec![MockResponse::Transient("network error".into())]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_missing_name() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!({"other": "x"}))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_not_string() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!({"name": 42}))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_empty_string() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!({"name": ""}))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_whitespace() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!({"name": "  "}))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_name_with_extras() {
        let s = MockStrategy::new(
            "s",
            vec![MockResponse::Ok(
                json!({"name": "fooBar", "extra": "ignored"}),
            )],
        );
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "fooBar");
    }

    #[test]
    fn ladder_response_top_level_array() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!(["foo"]))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn ladder_response_top_level_string() {
        let s = MockStrategy::new("s", vec![MockResponse::Ok(json!("foo"))]);
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("foo", "..."), "foo");
    }

    #[test]
    fn all_strategies_dead_falls_back() {
        // Two NotSupported → Ladder returns Transient "all strategies failed"
        let s0 = MockStrategy::new("s0", vec![MockResponse::NotSupported("no".into())]);
        let s1 = MockStrategy::new("s1", vec![MockResponse::NotSupported("no".into())]);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ladder = Arc::new(Ladder::new(vec![s0, s1]));
        let mut renamer = LlmRenamer::new(ladder, rt.handle().clone());
        assert_eq!(renamer.rename("bar", "..."), "bar");
    }

    #[test]
    fn empty_original_no_call() {
        let s = MockStrategy::new("s", vec![]);
        let count = s.call_count.clone();
        let (mut renamer, _rt) = make_renamer(s);
        assert_eq!(renamer.rename("", "..."), "");
        assert_eq!(count.load(SeqCst), 0);
    }

    #[test]
    fn system_and_user_prompt_shape() {
        let s = MockStrategy::new("s", vec![ok_name("result")]);
        let recorded = s.recorded.clone();
        let (mut renamer, _rt) = make_renamer(s);
        renamer.rename("foo", "const foo = 1;");

        let calls = recorded.lock().unwrap();
        assert_eq!(calls.len(), 1);
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
        let s = MockStrategy::new(
            "s",
            vec![ok_name("alpha"), ok_name("beta"), ok_name("gamma")],
        );
        let count = s.call_count.clone();
        let (mut renamer, _rt) = make_renamer(s);

        assert_eq!(renamer.rename("a", "..."), "alpha");
        assert_eq!(renamer.rename("b", "..."), "beta");
        assert_eq!(renamer.rename("c", "..."), "gamma");
        assert_eq!(count.load(SeqCst), 3);
    }
}
