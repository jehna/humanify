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

    #[cfg(test)]
    pub(crate) fn strategy_count(&self) -> usize {
        self.strategies.len()
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
    use serde_json::json;

    use crate::llm::test_dsl::{
        ladder_with, not_supported, ok, ok_n, pinned_with, script, ScriptedResponse,
    };

    #[tokio::test]
    async fn first_strategy_succeeds_locks() {
        ladder_with(vec![ok("s0", json!({"ok":true})), ok_n("s1", json!({}), 0)])
            .called_n_times(1)
            .await
            .locks_to("s0");
    }

    #[tokio::test]
    async fn first_unsupported_second_succeeds() {
        let outcome = ladder_with(vec![
            not_supported("s0", "no support"),
            ok("s1", json!({"ok":true})),
        ])
        .called_n_times(1)
        .await;
        outcome.locks_to("s1");
    }

    #[tokio::test]
    async fn locked_strategy_used_on_subsequent_calls() {
        ladder_with(vec![
            ok_n("s0", json!({"ok":true}), 2),
            ok_n("s1", json!({}), 0),
        ])
        .called_n_times(2)
        .await
        .locks_to("s0");
    }

    #[tokio::test]
    async fn dead_strategy_skipped_on_subsequent_calls() {
        let s0 = not_supported("s0", "nope");
        let s1 = ok_n("s1", json!({"ok":true}), 2);
        let outcome = ladder_with(vec![s0.clone(), s1.clone()])
            .called_n_times(2)
            .await;
        outcome.each_called(&[("s0", 1), ("s1", 2)]);
    }

    #[tokio::test]
    async fn transient_propagates_no_lock_no_dead() {
        let s0 = script(
            "s0",
            vec![
                ScriptedResponse::Transient("network error".into()),
                ScriptedResponse::Ok(json!({"ok":true})),
            ],
        );
        let outcome = ladder_with(vec![s0.clone(), not_supported("s1", "unused")])
            .called_n_times(1)
            .await;
        outcome.errors_with("network error");
    }

    #[tokio::test]
    async fn all_strategies_unsupported() {
        let outcome = ladder_with(vec![
            script(
                "alpha",
                vec![
                    ScriptedResponse::NotSupported("foo".into()),
                    ScriptedResponse::NotSupported("foo".into()),
                ],
            ),
            script(
                "beta",
                vec![
                    ScriptedResponse::NotSupported("bar".into()),
                    ScriptedResponse::NotSupported("bar".into()),
                ],
            ),
        ])
        .called_n_times(1)
        .await;
        outcome.errors_with("all");
    }

    #[tokio::test]
    async fn locked_then_unsupported_reprobes() {
        let s0 = script(
            "s0",
            vec![
                ScriptedResponse::Ok(json!({"ok":true})),
                ScriptedResponse::NotSupported("suddenly broken".into()),
            ],
        );
        let s1 = ok("s1", json!({"ok":true}));
        ladder_with(vec![s0, s1])
            .called_n_times(2)
            .await
            .locks_to("s1");
    }

    #[tokio::test]
    async fn pinned_single_strategy_succeeds() {
        pinned_with(ok_n("s0", json!({"ok":true}), 2))
            .called_n_times(2)
            .await
            .locks_to("s0");
    }

    #[tokio::test]
    async fn pinned_unsupported_becomes_transient() {
        let outcome = pinned_with(script(
            "s0",
            vec![ScriptedResponse::NotSupported("nope".into())],
        ))
        .called_n_times(1)
        .await;
        outcome.errors_with("nope");
    }

    #[tokio::test]
    async fn pinned_transient_propagates() {
        let outcome = pinned_with(script(
            "s0",
            vec![ScriptedResponse::Transient("network".into())],
        ))
        .called_n_times(1)
        .await;
        outcome.errors_with("network");
    }

    #[test]
    #[should_panic(expected = "must not be empty")]
    fn ladder_new_empty_panics() {
        Ladder::new(vec![]);
    }

    #[test]
    #[should_panic(expected = "at most 64")]
    fn ladder_new_too_many_panics() {
        use crate::llm::test_dsl::ok_n;
        let strats: Vec<Arc<dyn JsonStrategy>> = (0..65)
            .map(|_| ok_n("s", json!({}), 0) as Arc<dyn JsonStrategy>)
            .collect();
        Ladder::new(strats);
    }

    #[tokio::test]
    async fn locked_strategy_name_unlocked() {
        ladder_with(vec![ok_n("s0", json!({}), 0)])
            .called_n_times(0)
            .await
            .is_unlocked();
    }

    #[tokio::test]
    async fn locked_strategy_name_after_lock() {
        ladder_with(vec![ok("my-strategy", json!({"ok":true}))])
            .called_n_times(1)
            .await
            .locks_to("my-strategy");
    }

    // kept inline: concurrency setup (8 spawns + Barrier) doesn't compress into DSL without obscuring the race structure
    #[tokio::test]
    async fn concurrent_calls_during_probe() {
        use std::sync::Arc;
        use tokio::sync::Barrier;

        use crate::llm::test_dsl::ScriptedStrategy;

        let s0 = ScriptedStrategy::new_inner_pub(
            "s0",
            (0..16)
                .map(|_| ScriptedResponse::NotSupported("not supported".into()))
                .collect(),
        );
        let s1 = ScriptedStrategy::new_inner_pub(
            "s1",
            (0..16)
                .map(|_| ScriptedResponse::Ok(json!({"ok":true})))
                .collect(),
        );

        let dyn_strats: Vec<Arc<dyn JsonStrategy>> = vec![
            Arc::clone(&s0) as Arc<dyn JsonStrategy>,
            Arc::clone(&s1) as Arc<dyn JsonStrategy>,
        ];
        let ladder = Arc::new(Ladder::new(dyn_strats));
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
            assert!(handle.await.unwrap().is_ok());
        }

        assert_eq!(ladder.locked_strategy_name(), Some("s1"));
        assert!(s1.count() >= 1, "s1 should have been called at least once");
    }
}
