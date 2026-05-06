use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::Value;

use crate::llm::{http::StrategyError, JsonStrategy, Ladder};

// --- ScriptedResponse ---

pub enum ScriptedResponse {
    Ok(Value),
    NotSupported(String),
    Transient(String),
}

// --- ScriptedStrategy ---

pub struct ScriptedStrategy {
    pub strategy_name: &'static str,
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    pub call_count: Arc<AtomicUsize>,
    /// Records (system, user, schema) for each call when non-None.
    pub recorded: Arc<Mutex<Vec<(String, String, Value)>>>,
}

impl ScriptedStrategy {
    pub fn new_inner_pub(name: &'static str, responses: Vec<ScriptedResponse>) -> Arc<Self> {
        Self::new_inner(name, responses)
    }

    fn new_inner(name: &'static str, responses: Vec<ScriptedResponse>) -> Arc<Self> {
        Arc::new(Self {
            strategy_name: name,
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            call_count: Arc::new(AtomicUsize::new(0)),
            recorded: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn count(&self) -> usize {
        self.call_count.load(SeqCst)
    }
}

#[async_trait]
impl JsonStrategy for ScriptedStrategy {
    async fn call(&self, system: &str, user: &str, schema: &Value) -> Result<Value, StrategyError> {
        self.call_count.fetch_add(1, SeqCst);
        self.recorded
            .lock()
            .unwrap()
            .push((system.to_string(), user.to_string(), schema.clone()));
        let next = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                panic!(
                    "ScriptedStrategy '{}' called more times than configured",
                    self.strategy_name
                )
            });
        match next {
            ScriptedResponse::Ok(v) => Ok(v),
            ScriptedResponse::NotSupported(r) => Err(StrategyError::NotSupported(r)),
            ScriptedResponse::Transient(r) => Err(StrategyError::Transient(anyhow!("{}", r))),
        }
    }

    fn name(&self) -> &'static str {
        self.strategy_name
    }
}

// --- ScriptedStrategy constructors ---

pub fn ok(name: &'static str, value: Value) -> Arc<ScriptedStrategy> {
    ScriptedStrategy::new_inner(name, vec![ScriptedResponse::Ok(value)])
}

pub fn ok_n(name: &'static str, value: Value, n: usize) -> Arc<ScriptedStrategy> {
    let responses = (0..n)
        .map(|_| ScriptedResponse::Ok(value.clone()))
        .collect();
    ScriptedStrategy::new_inner(name, responses)
}

pub fn not_supported(name: &'static str, reason: &str) -> Arc<ScriptedStrategy> {
    ScriptedStrategy::new_inner(
        name,
        vec![ScriptedResponse::NotSupported(reason.to_string())],
    )
}

pub fn transient(name: &'static str, reason: &str) -> Arc<ScriptedStrategy> {
    ScriptedStrategy::new_inner(name, vec![ScriptedResponse::Transient(reason.to_string())])
}

pub fn script(name: &'static str, responses: Vec<ScriptedResponse>) -> Arc<ScriptedStrategy> {
    ScriptedStrategy::new_inner(name, responses)
}

// --- LadderBuilder ---

pub struct LadderBuilder {
    ladder: Arc<Ladder>,
    strategies: Vec<Arc<ScriptedStrategy>>,
}

pub fn ladder_with(strategies: Vec<Arc<ScriptedStrategy>>) -> LadderBuilder {
    let dyn_strategies: Vec<Arc<dyn JsonStrategy>> = strategies
        .iter()
        .map(|s| Arc::clone(s) as Arc<dyn JsonStrategy>)
        .collect();
    LadderBuilder {
        ladder: Arc::new(Ladder::new(dyn_strategies)),
        strategies,
    }
}

pub fn pinned_with(strategy: Arc<ScriptedStrategy>) -> LadderBuilder {
    let dyn_strat: Arc<dyn JsonStrategy> = Arc::clone(&strategy) as Arc<dyn JsonStrategy>;
    LadderBuilder {
        ladder: Arc::new(Ladder::pinned(dyn_strat)),
        strategies: vec![strategy],
    }
}

impl LadderBuilder {
    pub async fn called_n_times(self, n: usize) -> LadderOutcome {
        use serde_json::json;
        let mut results = Vec::with_capacity(n);
        for _ in 0..n {
            results.push(self.ladder.call("", "", &json!({})).await);
        }
        LadderOutcome {
            ladder: self.ladder,
            strategies: self.strategies,
            results,
        }
    }
}

// --- LadderOutcome ---

pub struct LadderOutcome {
    ladder: Arc<Ladder>,
    strategies: Vec<Arc<ScriptedStrategy>>,
    results: Vec<Result<Value, StrategyError>>,
}

impl LadderOutcome {
    fn last(&self) -> &Result<Value, StrategyError> {
        self.results.last().expect("no results")
    }

    pub fn locks_to(self, name: &str) {
        assert_eq!(
            self.ladder.locked_strategy_name(),
            Some(name),
            "expected ladder locked to '{name}'"
        );
    }

    pub fn is_unlocked(self) {
        assert_eq!(
            self.ladder.locked_strategy_name(),
            None,
            "expected ladder to be unlocked"
        );
    }

    pub fn each_called(self, expected: &[(&str, usize)]) {
        for (name, count) in expected {
            let s = self
                .strategies
                .iter()
                .find(|s| s.strategy_name == *name)
                .unwrap_or_else(|| panic!("no strategy named '{name}'"));
            assert_eq!(s.count(), *count, "strategy '{name}' call count mismatch");
        }
    }

    pub fn errors_with(self, substring: &str) {
        match self.last() {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains(substring),
                    "error should contain {substring:?}, got: {msg}"
                );
            }
            Ok(v) => panic!("expected error containing {substring:?}, got Ok({v})"),
        }
    }

    pub fn succeeds_with(self, expected: &Value) {
        match self.last() {
            Ok(v) => assert_eq!(v, expected),
            Err(e) => panic!("expected Ok({expected}), got Err({e})"),
        }
    }

    pub fn last_is_ok(self) -> Value {
        match self.results.into_iter().last().expect("no results") {
            Ok(v) => v,
            Err(e) => panic!("expected Ok, got Err: {e}"),
        }
    }
}

// --- Helper extraction DSL for openai_compat / anthropic tests ---

/// Assert that a helper extraction function returns Ok and equals expected.
pub fn extract_succeeds(result: Result<Value, StrategyError>, expected: &Value) {
    match result {
        Ok(v) => assert_eq!(&v, expected),
        Err(e) => panic!("expected Ok({expected}), got Err({e})"),
    }
}

/// Assert that a helper extraction function returns a Transient error containing substring.
pub fn extract_fails_with(result: Result<Value, StrategyError>, substring: &str) {
    match result {
        Err(StrategyError::Transient(e)) => {
            let msg = e.to_string();
            assert!(
                substring.is_empty() || msg.contains(substring),
                "error should contain {substring:?}, got: {msg}"
            );
        }
        Ok(v) => panic!("expected Transient error, got Ok({v})"),
        Err(StrategyError::NotSupported(r)) => {
            panic!("expected Transient, got NotSupported({r})")
        }
    }
}
