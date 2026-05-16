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
                eprintln!("humanify: LLM call failed for `{original}`: {e:?}");
                original.to_string()
            }
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
        let renamer = LlmRenamer::new(ladder, rt.handle().clone());
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
}
