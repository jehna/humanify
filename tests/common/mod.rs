#![allow(dead_code)]
//! E2e DSL shared by all `tests/e2e_*.rs` files.
//!
//! Backbone:
//!   given("fixtures/...")
//!       .judge_says_minified().await
//!       .when(humanify().ollama().model("qwen3.5:4b"))
//!       .await
//!       .then_judge_says_one_of(&["EXCELLENT", "GOOD"])
//!       .await;
//!
//! The judge is a local Ollama instance with qwen3.5:4b. Set
//! HUMANIFY_E2E_JUDGE_URL to override (default: http://localhost:11434/v1).

use humanify::{HttpClient, JsonStrategy, OpenAIJsonSchema};
use serde_json::json;

const JUDGE_SYSTEM: &str = "You are a code-readability judge. You receive JavaScript source code and respond with exactly one verdict. Respond with JSON only.

Verdicts:
- EXCELLENT: every identifier reads like normal, descriptive code.
- GOOD: most identifiers are descriptive; a few are still vague but plausible.
- GIBBERISH: most identifiers are sensible, but at least one is clearly wrong or nonsensical for what it represents (e.g. a function that adds numbers named `parseHTMLTree`, a counter named `purpleElephant`). Use this when the code looks renamed but contains an obvious hallucination.
- MINIFIED: identifiers are single letters, random-looking, or otherwise minified/obfuscated.

Examples:

Code:
```javascript
function addNumbers(a, b) { return a + b; }
const total = addNumbers(3, 4);
```
Verdict: EXCELLENT

Code:
```javascript
function process(data, opts) { return data.map(x => x * opts.factor); }
const result = process([1,2,3], { factor: 2 });
```
Verdict: GOOD

Code:
```javascript
function parseHTMLTree(a, b) { return a + b; }
const purpleElephant = parseHTMLTree(3, 4);
```
Verdict: GIBBERISH

Code:
```javascript
function a(b,c){return b+c}var d=a(3,4);
```
Verdict: MINIFIED";

fn judge_user(code: &str) -> String {
    format!("Source:\n```javascript\n{code}\n```\n\nVerdict:")
}

fn judge_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdict"],
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["EXCELLENT", "GOOD", "GIBBERISH", "MINIFIED"]
            }
        }
    })
}

fn judge_url() -> String {
    std::env::var("HUMANIFY_E2E_JUDGE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string())
}

pub async fn judge(code: &str) -> anyhow::Result<String> {
    // The judge talks to a local Ollama running qwen3.5:4b. On a CPU-only CI
    // runner one structured-output completion can take 10–15 min, so match the
    // Ollama subcommand's 1800s per-request budget instead of the global default.
    let strategy = OpenAIJsonSchema::new(
        HttpClient::with_timeout(std::time::Duration::from_secs(1800)),
        judge_url(),
        None,
        "qwen3.5:4b".to_string(),
    );
    let result = strategy
        .call(JUDGE_SYSTEM, &judge_user(code), &judge_schema())
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;
    let verdict = result["verdict"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("judge returned no verdict field"))?
        .to_string();
    Ok(verdict)
}

pub fn given(input_path: &str) -> Scenario {
    Scenario {
        input_path: input_path.to_string(),
    }
}

pub fn humanify() -> HumanifyCmdBuilder {
    HumanifyCmdBuilder {
        subcommand: String::new(),
        model: None,
        json_mode: None,
        base_url: None,
    }
}

pub struct Scenario {
    input_path: String,
}

impl Scenario {
    pub async fn judge_says_minified(self) -> Self {
        let source = std::fs::read_to_string(&self.input_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", self.input_path));
        let verdict = judge(&source)
            .await
            .unwrap_or_else(|e| panic!("judge failed for pre-assertion: {e}"));
        assert_eq!(
            verdict, "MINIFIED",
            "pre-assertion failed: expected fixture to be MINIFIED but got {verdict}"
        );
        self
    }

    pub async fn when(self, builder: HumanifyCmdBuilder) -> Outcome {
        let input_path = self.input_path.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut cmd = assert_cmd::Command::cargo_bin("humanify").unwrap();
            cmd.arg(&builder.subcommand);
            if let Some(m) = &builder.model {
                cmd.arg("--model").arg(m);
            }
            if let Some(j) = &builder.json_mode {
                cmd.arg("--json-mode").arg(j);
            }
            if let Some(u) = &builder.base_url {
                cmd.arg("--base-url").arg(u);
            }
            cmd.arg(&input_path);
            cmd.output().expect("failed to run humanify binary")
        })
        .await
        .expect("spawn_blocking panicked");

        Outcome {
            renamed: String::from_utf8(output.stdout).unwrap_or_default(),
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8(output.stderr).unwrap_or_default(),
        }
    }
}

pub struct HumanifyCmdBuilder {
    subcommand: String,
    model: Option<String>,
    json_mode: Option<String>,
    base_url: Option<String>,
}

impl HumanifyCmdBuilder {
    pub fn openai(mut self) -> Self {
        self.subcommand = "openai".to_string();
        self
    }

    pub fn gemini(mut self) -> Self {
        self.subcommand = "gemini".to_string();
        self
    }

    pub fn anthropic(mut self) -> Self {
        self.subcommand = "anthropic".to_string();
        self
    }

    pub fn ollama(mut self) -> Self {
        self.subcommand = "ollama".to_string();
        self
    }

    pub fn openrouter(mut self) -> Self {
        self.subcommand = "openrouter".to_string();
        self
    }

    pub fn model(mut self, name: &str) -> Self {
        self.model = Some(name.to_string());
        self
    }

    pub fn json_mode(mut self, mode: &str) -> Self {
        self.json_mode = Some(mode.to_string());
        self
    }

    pub fn base_url(mut self, url: &str) -> Self {
        self.base_url = Some(url.to_string());
        self
    }
}

pub struct Outcome {
    renamed: String,
    exit_code: i32,
    stderr: String,
}

impl Outcome {
    pub fn then_exit_code_is(self, code: i32) -> Self {
        assert_eq!(
            self.exit_code, code,
            "expected exit code {code} but got {}. stderr:\n{}",
            self.exit_code, self.stderr
        );
        self
    }

    pub async fn then_judge_says_one_of(self, verdicts: &[&str]) {
        let verdict = judge(&self.renamed)
            .await
            .unwrap_or_else(|e| panic!("judge failed: {e}\nstderr:\n{}", self.stderr));
        assert!(
            verdicts.contains(&verdict.as_str()),
            "expected verdict in {verdicts:?} but got {verdict:?}\nrenamed output:\n{}\nstderr:\n{}",
            self.renamed,
            self.stderr
        );
    }
}
