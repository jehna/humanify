use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;

use anyhow::anyhow;
use serde_json::Value;

use crate::llm::{http::StrategyError, JsonStrategy};

const UNLOCKED: usize = usize::MAX;

pub struct Ladder {
    strategies: Vec<Arc<dyn JsonStrategy>>,
    locked: AtomicUsize,
    dead: AtomicU64,
}

impl Ladder {
    pub fn new(strategies: Vec<Arc<dyn JsonStrategy>>) -> Self {
        assert!(
            !strategies.is_empty(),
            "Ladder::new: strategies must not be empty"
        );
        assert!(
            strategies.len() <= 64,
            "Ladder::new: at most 64 strategies supported (got {})",
            strategies.len()
        );
        Self {
            strategies,
            locked: AtomicUsize::new(UNLOCKED),
            dead: AtomicU64::new(0),
        }
    }

    /// Build a single-strategy ladder that is immediately locked.
    /// NotSupported from this strategy converts to Transient (no fallback exists).
    pub fn pinned(strategy: Arc<dyn JsonStrategy>) -> Self {
        Self {
            strategies: vec![strategy],
            locked: AtomicUsize::new(0),
            dead: AtomicU64::new(0),
        }
    }

    pub async fn call(
        &self,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> Result<Value, StrategyError> {
        let locked_idx = self.locked.load(SeqCst);

        if locked_idx != UNLOCKED {
            // Fast path: use the locked strategy.
            let strategy = &self.strategies[locked_idx];
            return match strategy.call(system, user, schema).await {
                Ok(v) => Ok(v),
                Err(StrategyError::Transient(e)) => Err(StrategyError::Transient(e)),
                Err(StrategyError::NotSupported(reason)) => {
                    if self.strategies.len() == 1 {
                        // Pinned: no fallback. Convert to Transient without marking dead
                        // so subsequent calls can retry.
                        Err(StrategyError::Transient(anyhow!(
                            "strategy '{}' returned NotSupported: {}",
                            strategy.name(),
                            reason
                        )))
                    } else {
                        // Safety net: locked strategy suddenly rejected. Clear lock,
                        // mark dead, re-probe from index 0.
                        self.locked.store(UNLOCKED, SeqCst);
                        self.dead.fetch_or(1u64 << locked_idx, SeqCst);
                        self.probe(system, user, schema).await
                    }
                }
            };
        }

        self.probe(system, user, schema).await
    }

    /// Probe strategies in order, skipping dead ones. Re-probe from index 0
    /// (dead set ensures we skip already-rejected strategies).
    async fn probe(
        &self,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> Result<Value, StrategyError> {
        let mut not_supported_reasons: Vec<String> = Vec::new();

        for (i, strategy) in self.strategies.iter().enumerate() {
            if self.dead.load(SeqCst) & (1u64 << i) != 0 {
                // Already known-dead; collect a placeholder reason for "all dead" message.
                not_supported_reasons.push(format!("{} (previously rejected)", strategy.name()));
                continue;
            }

            match strategy.call(system, user, schema).await {
                Ok(v) => {
                    self.locked.store(i, SeqCst);
                    return Ok(v);
                }
                Err(StrategyError::NotSupported(reason)) => {
                    self.dead.fetch_or(1u64 << i, SeqCst);
                    not_supported_reasons.push(format!("{} ({})", strategy.name(), reason));
                    // Continue to next strategy.
                }
                Err(StrategyError::Transient(e)) => {
                    // Don't lock, don't mark dead. Caller retries the whole operation.
                    return Err(StrategyError::Transient(e));
                }
            }
        }

        // All strategies exhausted (all dead or all returned NotSupported this call).
        Err(StrategyError::Transient(anyhow!(
            "all JSON-mode strategies failed: {}",
            not_supported_reasons.join(", ")
        )))
    }

    pub fn locked_strategy_name(&self) -> Option<&'static str> {
        let idx = self.locked.load(SeqCst);
        if idx == UNLOCKED {
            None
        } else {
            Some(self.strategies[idx].name())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;

    enum MockResponse {
        Ok(Value),
        NotSupported(String),
        Transient(String),
    }

    struct MockStrategy {
        strategy_name: &'static str,
        responses: Arc<Mutex<VecDeque<MockResponse>>>,
        call_count: Arc<AtomicUsize>,
    }

    impl MockStrategy {
        fn new(name: &'static str, responses: Vec<MockResponse>) -> Arc<Self> {
            Arc::new(Self {
                strategy_name: name,
                responses: Arc::new(Mutex::new(VecDeque::from(responses))),
                call_count: Arc::new(AtomicUsize::new(0)),
            })
        }

        fn count(&self) -> usize {
            self.call_count.load(SeqCst)
        }
    }

    #[async_trait]
    impl JsonStrategy for MockStrategy {
        async fn call(&self, _: &str, _: &str, _: &Value) -> Result<Value, StrategyError> {
            self.call_count.fetch_add(1, SeqCst);
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

    fn ok() -> MockResponse {
        MockResponse::Ok(json!({"ok": true}))
    }
    fn ns(reason: &str) -> MockResponse {
        MockResponse::NotSupported(reason.to_string())
    }
    fn tr(msg: &str) -> MockResponse {
        MockResponse::Transient(msg.to_string())
    }

    fn assert_ok(r: Result<Value, StrategyError>) -> Value {
        match r {
            Ok(v) => v,
            Err(e) => panic!("expected Ok, got Err: {e}"),
        }
    }

    fn assert_transient(r: Result<Value, StrategyError>) -> String {
        match r {
            Err(StrategyError::Transient(e)) => e.to_string(),
            Ok(v) => panic!("expected Transient, got Ok({v})"),
            Err(StrategyError::NotSupported(r)) => {
                panic!("expected Transient, got NotSupported({r})")
            }
        }
    }

    #[tokio::test]
    async fn first_strategy_succeeds_locks() {
        let s0 = MockStrategy::new("s0", vec![ok()]);
        let s1 = MockStrategy::new("s1", vec![]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s0"));
        assert_eq!(s0.count(), 1);
        assert_eq!(s1.count(), 0);
    }

    #[tokio::test]
    async fn first_unsupported_second_succeeds() {
        let s0 = MockStrategy::new("s0", vec![ns("no support")]);
        let s1 = MockStrategy::new("s1", vec![ok()]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s1"));
        assert_eq!(s0.count(), 1);
        assert_eq!(s1.count(), 1);
        // s0 is dead
        assert_ne!(ladder.dead.load(SeqCst) & 1, 0);
    }

    #[tokio::test]
    async fn locked_strategy_used_on_subsequent_calls() {
        let s0 = MockStrategy::new("s0", vec![ok(), ok()]);
        let s1 = MockStrategy::new("s1", vec![]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_ok(ladder.call("", "", &json!({})).await);

        assert_eq!(ladder.locked_strategy_name(), Some("s0"));
        assert_eq!(s0.count(), 2);
        assert_eq!(s1.count(), 0);
    }

    #[tokio::test]
    async fn dead_strategy_skipped_on_subsequent_calls() {
        // First call: s0 NotSupported → s1 Ok (locked=1)
        // Second call: locked=1 → s1 Ok. s0 never called again.
        let s0 = MockStrategy::new("s0", vec![ns("nope")]);
        let s1 = MockStrategy::new("s1", vec![ok(), ok()]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_ok(ladder.call("", "", &json!({})).await);

        assert_eq!(s0.count(), 1); // only probed once
        assert_eq!(s1.count(), 2); // locked and used twice
    }

    #[tokio::test]
    async fn transient_propagates_no_lock_no_dead() {
        let s0 = MockStrategy::new("s0", vec![tr("network error"), ok()]);
        let s1 = MockStrategy::new("s1", vec![]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        let msg = assert_transient(ladder.call("", "", &json!({})).await);
        assert!(msg.contains("network error"), "msg: {msg}");
        assert_eq!(ladder.locked_strategy_name(), None);
        assert_eq!(ladder.dead.load(SeqCst), 0);
        assert_eq!(s0.count(), 1);
        assert_eq!(s1.count(), 0);

        // Next call: s0 returns Ok → locks
        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s0"));
    }

    #[tokio::test]
    async fn all_strategies_unsupported() {
        let s0 = MockStrategy::new("alpha", vec![ns("foo"), ns("foo")]);
        let s1 = MockStrategy::new("beta", vec![ns("bar"), ns("bar")]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        let msg = assert_transient(ladder.call("", "", &json!({})).await);
        assert!(msg.contains("all"), "msg: {msg}");
        assert!(msg.contains("alpha"), "msg: {msg}");
        assert!(msg.contains("beta"), "msg: {msg}");
        assert!(msg.contains("foo"), "msg: {msg}");
        assert!(msg.contains("bar"), "msg: {msg}");

        // Both dead — subsequent call returns immediately (counts unchanged).
        let s0_count = s0.count();
        let s1_count = s1.count();
        let msg2 = assert_transient(ladder.call("", "", &json!({})).await);
        assert!(msg2.contains("all"), "msg2: {msg2}");
        assert_eq!(s0.count(), s0_count, "s0 should not be called again");
        assert_eq!(s1.count(), s1_count, "s1 should not be called again");
    }

    #[tokio::test]
    async fn locked_then_unsupported_reprobes() {
        // First call: s0 Ok → locked=0
        // Second call: s0 NotSupported → clear lock, mark dead, re-probe → s1 Ok → locked=1
        let s0 = MockStrategy::new("s0", vec![ok(), ns("suddenly broken")]);
        let s1 = MockStrategy::new("s1", vec![ok()]);
        let ladder = Ladder::new(vec![s0.clone(), s1.clone()]);

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s0"));

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s1"));
        assert_eq!(s0.count(), 2);
        assert_eq!(s1.count(), 1);
        assert_ne!(ladder.dead.load(SeqCst) & 1, 0); // s0 is dead
    }

    #[tokio::test]
    async fn pinned_single_strategy_succeeds() {
        let s0 = MockStrategy::new("s0", vec![ok(), ok()]);
        let ladder = Ladder::pinned(s0.clone());

        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("s0"));
        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(s0.count(), 2);
    }

    #[tokio::test]
    async fn pinned_unsupported_becomes_transient() {
        let s0 = MockStrategy::new("s0", vec![ns("nope"), ns("nope")]);
        let ladder = Ladder::pinned(s0.clone());

        let msg = assert_transient(ladder.call("", "", &json!({})).await);
        assert!(msg.contains("nope"), "msg: {msg}");
        // Strategy NOT marked dead — subsequent call retries.
        assert_eq!(ladder.dead.load(SeqCst), 0);
        assert_eq!(s0.count(), 1);

        // Second call also retries.
        let msg2 = assert_transient(ladder.call("", "", &json!({})).await);
        assert!(msg2.contains("nope"), "msg2: {msg2}");
        assert_eq!(s0.count(), 2);
    }

    #[tokio::test]
    async fn pinned_transient_propagates() {
        let s0 = MockStrategy::new("s0", vec![tr("network"), ok()]);
        let ladder = Ladder::pinned(s0.clone());

        assert_transient(ladder.call("", "", &json!({})).await);
        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(s0.count(), 2);
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn ladder_new_empty_panics() {
        Ladder::new(vec![]);
    }

    #[test]
    #[should_panic(expected = "at most 64")]
    fn ladder_new_too_many_panics() {
        let strats: Vec<Arc<dyn JsonStrategy>> = (0..65)
            .map(|_| MockStrategy::new("s", vec![]) as Arc<dyn JsonStrategy>)
            .collect();
        Ladder::new(strats);
    }

    #[tokio::test]
    async fn locked_strategy_name_unlocked() {
        let s0 = MockStrategy::new("s0", vec![]);
        let ladder = Ladder::new(vec![s0]);
        assert_eq!(ladder.locked_strategy_name(), None);
    }

    #[tokio::test]
    async fn locked_strategy_name_after_lock() {
        let s0 = MockStrategy::new("my-strategy", vec![ok()]);
        let ladder = Ladder::new(vec![s0]);
        assert_ok(ladder.call("", "", &json!({})).await);
        assert_eq!(ladder.locked_strategy_name(), Some("my-strategy"));
    }

    #[tokio::test]
    async fn concurrent_calls_during_probe() {
        use std::sync::Arc;
        use tokio::sync::Barrier;

        // s0 returns NotSupported (many times — concurrent callers may all hit it)
        // s1 returns Ok (many times)
        let s0_responses: Vec<MockResponse> = (0..16).map(|_| ns("not supported")).collect();
        let s1_responses: Vec<MockResponse> = (0..16).map(|_| ok()).collect();
        let s0 = MockStrategy::new("s0", s0_responses);
        let s1 = MockStrategy::new("s1", s1_responses);
        let s1_count = s1.call_count.clone();

        let ladder = Arc::new(Ladder::new(vec![s0.clone(), s1.clone()]));
        let barrier = Arc::new(Barrier::new(8));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let ladder = ladder.clone();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    ladder.call("", "", &json!({})).await
                })
            })
            .collect();

        for handle in handles {
            assert_ok(handle.await.unwrap());
        }

        assert_eq!(ladder.locked_strategy_name(), Some("s1"));
        assert!(
            s1_count.load(SeqCst) >= 1,
            "s1 should have been called at least once"
        );
    }
}
