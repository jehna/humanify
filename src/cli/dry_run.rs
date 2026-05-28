use std::path::PathBuf;

use tiktoken_rs::{cl100k_base, o200k_base, CoreBPE};

use crate::llm::build_rename_prompt;
use crate::pipe;
use crate::rename::{rename_all_identifiers, RenameError, Renamer};

pub struct Args {
    pub input: String,
    pub output: Option<PathBuf>,
    pub context_size: usize,
    pub tokenizer: String,
}

/// Dry-run: walks every identifier the real pipeline would walk, builds the
/// exact prompt that would be sent, tokenises it, and prints per-call and
/// aggregate token totals. No LLM calls, no API keys required.
pub fn run(args: Args) -> i32 {
    let bpe = match args.tokenizer.as_str() {
        "cl100k_base" => match cl100k_base() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("humanify: failed to load cl100k_base: {e}");
                return 1;
            }
        },
        "o200k_base" => match o200k_base() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("humanify: failed to load o200k_base: {e}");
                return 1;
            }
        },
        other => {
            eprintln!(
                "humanify: unknown tokenizer '{other}'. Valid: cl100k_base, o200k_base"
            );
            return 64;
        }
    };

    let source = match pipe::read_input(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("humanify: failed to read input: {e}");
            return 1;
        }
    };

    let mut renamer = TokenCountingRenamer::new(bpe);

    match rename_all_identifiers(&source, &mut renamer, args.context_size) {
        Ok(_) => {}
        Err(RenameError::Parse(msg)) => {
            eprintln!("humanify: parse error: {msg}");
            return 2;
        }
    }

    let report = renamer.report(&args.tokenizer, args.context_size);

    if let Err(e) = pipe::write_output(args.output.as_deref(), &report) {
        eprintln!("humanify: failed to write output: {e}");
        return 1;
    }

    0
}

struct TokenCountingRenamer {
    bpe: CoreBPE,
    calls: usize,
    system_tokens: usize,
    user_tokens: usize,
    schema_tokens: usize,
}

impl TokenCountingRenamer {
    fn new(bpe: CoreBPE) -> Self {
        Self {
            bpe,
            calls: 0,
            system_tokens: 0,
            user_tokens: 0,
            schema_tokens: 0,
        }
    }

    fn count(&self, s: &str) -> usize {
        self.bpe.encode_with_special_tokens(s).len()
    }

    fn report(&self, tokenizer: &str, context_size: usize) -> String {
        let total = self.system_tokens + self.user_tokens + self.schema_tokens;
        let avg_total = if self.calls == 0 {
            0
        } else {
            total / self.calls
        };
        let avg_user = if self.calls == 0 {
            0
        } else {
            self.user_tokens / self.calls
        };
        format!(
            "humanify dry-run\n\
             tokenizer       {tokenizer}\n\
             context_size    {context_size}\n\
             calls           {calls}\n\
             system tokens   {sys}\n\
             user tokens     {usr}\n\
             schema tokens   {sch}\n\
             total tokens    {total}\n\
             avg total/call  {avg_total}\n\
             avg user/call   {avg_user}\n",
            calls = self.calls,
            sys = self.system_tokens,
            usr = self.user_tokens,
            sch = self.schema_tokens,
        )
    }
}

impl Renamer for TokenCountingRenamer {
    fn rename(&mut self, original: &str, surrounding_code: &str) -> String {
        if original.is_empty() {
            return String::new();
        }
        let (system, user, schema) = build_rename_prompt(original, surrounding_code);
        let schema_str = serde_json::to_string(&schema).unwrap_or_default();
        self.calls += 1;
        self.system_tokens += self.count(system);
        self.user_tokens += self.count(&user);
        self.schema_tokens += self.count(&schema_str);
        // Return the original name unchanged → walker short-circuits the
        // safe-name pipeline for this binding, no double counting.
        original.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(input: &str) -> Args {
        Args {
            input: input.to_string(),
            output: None,
            context_size: 500,
            tokenizer: "cl100k_base".to_string(),
        }
    }

    #[test]
    fn unknown_tokenizer_returns_64() {
        let mut a = args("irrelevant");
        a.tokenizer = "garbage".to_string();
        assert_eq!(run(a), 64);
    }

    #[test]
    fn counts_calls_per_binding() {
        let bpe = cl100k_base().unwrap();
        let mut r = TokenCountingRenamer::new(bpe);
        let src = "function a(b){var c=1;return b+c}";
        rename_all_identifiers(src, &mut r, 500).unwrap();
        // a, b, c → 3 bindings
        assert_eq!(r.calls, 3);
        assert!(r.system_tokens > 0);
        assert!(r.user_tokens > 0);
        assert!(r.schema_tokens > 0);
    }
}
