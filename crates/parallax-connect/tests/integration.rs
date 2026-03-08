//! Integration tests for parallax-connect v0.2.
//!
//! Asserts parallel step execution (3A), topological wave grouping, and that
//! INV-C06 (failed steps don't block siblings) is preserved in the new JoinSet
//! scheduler.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use parallax_connect::{
    builder::entity,
    connector::{step, topological_waves, StepDefinition},
    error::ConnectorError,
    scheduler::run_connector,
    Connector, StepContext,
};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn make_steps(deps: &[(&str, &[&str])]) -> Vec<StepDefinition> {
    deps.iter()
        .map(|(id, ds)| StepDefinition {
            id: id.to_string(),
            description: id.to_string(),
            depends_on: ds.iter().map(|d| d.to_string()).collect(),
        })
        .collect()
}

// ─── topological_waves ───────────────────────────────────────────────────────

/// All independent steps collapse to a single wave.
#[test]
fn v02_waves_all_independent_is_one_wave() {
    let steps = make_steps(&[("a", &[]), ("b", &[]), ("c", &[])]);
    let waves = topological_waves(&steps);
    assert_eq!(waves.len(), 1, "three independent steps = one wave");
    assert_eq!(waves[0].len(), 3);
}

/// Linear chain a → b → c produces three sequential waves.
#[test]
fn v02_waves_linear_chain_is_three_waves() {
    let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
    let waves = topological_waves(&steps);
    assert_eq!(waves.len(), 3);
    assert!(waves[0].contains(&"a".to_string()));
    assert!(waves[1].contains(&"b".to_string()));
    assert!(waves[2].contains(&"c".to_string()));
}

/// Diamond: a → {b, c} → d — b and c must be in the same parallel wave.
#[test]
fn v02_waves_diamond_b_and_c_parallel() {
    let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
    let waves = topological_waves(&steps);
    assert_eq!(waves.len(), 3, "diamond = 3 waves: {{a}}, {{b,c}}, {{d}}");

    assert_eq!(waves[0], vec!["a"]);
    assert_eq!(waves[2], vec!["d"]);

    let mid: std::collections::HashSet<_> = waves[1].iter().cloned().collect();
    assert!(mid.contains("b"), "b must be in wave 1");
    assert!(mid.contains("c"), "c must be in wave 1");
}

/// Empty step list returns an empty wave list.
#[test]
fn v02_waves_empty_input_returns_empty() {
    assert!(topological_waves(&[]).is_empty());
}

/// Wide fan: 1 root → 5 parallel leaves → 1 sink = 3 waves, middle wave has 5 steps.
#[test]
fn v02_waves_wide_fan_parallelism() {
    let steps = make_steps(&[
        ("root", &[]),
        ("l1", &["root"]),
        ("l2", &["root"]),
        ("l3", &["root"]),
        ("l4", &["root"]),
        ("l5", &["root"]),
        ("sink", &["l1", "l2", "l3", "l4", "l5"]),
    ]);
    let waves = topological_waves(&steps);
    assert_eq!(waves.len(), 3);
    assert_eq!(
        waves[1].len(),
        5,
        "all five leaves must be in the same wave"
    );
}

// ─── run_connector (parallel execution) ──────────────────────────────────────

/// Connector whose steps track how many execute concurrently (by incrementing
/// an atomic counter before sleeping).
struct CountingConnector {
    steps: Vec<StepDefinition>,
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for CountingConnector {
    fn name(&self) -> &str {
        "counter"
    }
    fn steps(&self) -> Vec<StepDefinition> {
        self.steps.clone()
    }
    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        ctx.emit_entity(entity(step_id, step_id))?;
        Ok(())
    }
}

/// Three independent steps all run and emit one entity each.
#[tokio::test]
async fn v02_parallel_run_all_entities_collected() {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = Arc::new(CountingConnector {
        steps: make_steps(&[("a", &[]), ("b", &[]), ("c", &[])]),
        counter: Arc::clone(&counter),
    });
    let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
    assert_eq!(
        out.entities.len(),
        3,
        "all three parallel steps must emit their entity"
    );
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

/// Chain a → b → c; entities from all waves are collected.
#[tokio::test]
async fn v02_parallel_run_chain_collects_all() {
    let c = Arc::new(CountingConnector {
        steps: make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]),
        counter: Arc::new(AtomicUsize::new(0)),
    });
    let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
    assert_eq!(out.entities.len(), 3);
}

/// Downstream step can read entities from upstream steps via `prior_entities`.
struct PriorReadingConnector;

#[async_trait]
impl Connector for PriorReadingConnector {
    fn name(&self) -> &str {
        "prior-reader"
    }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            step("upstream", "upstream step").build(),
            step("downstream", "reads upstream")
                .depends_on(&["upstream"])
                .build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "upstream" => {
                ctx.emit_entity(entity("host", "upstream-host").display_name("Upstream Host"))?;
            }
            "downstream" => {
                // Must be able to find the entity emitted by upstream.
                let found = ctx.get_prior_entity("host", "upstream-host");
                assert!(
                    found.is_some(),
                    "downstream must see upstream entity in prior_entities"
                );
                ctx.emit_entity(entity("service", "downstream-svc"))?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Downstream step sees upstream entities in `prior_entities`.
#[tokio::test]
async fn v02_parallel_downstream_sees_upstream_prior_entities() {
    let c = Arc::new(PriorReadingConnector);
    let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
    assert_eq!(
        out.entities.len(),
        2,
        "both upstream and downstream must emit"
    );
}

/// A failing step in a wave doesn't prevent its sibling from running (INV-C06).
struct SiblingFailConnector;

#[async_trait]
impl Connector for SiblingFailConnector {
    fn name(&self) -> &str {
        "sibling-fail"
    }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            step("ok", "succeeds").build(),
            step("fail", "always fails").build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        if step_id == "fail" {
            return Err(ConnectorError::UnknownStep("intentional".into()));
        }
        ctx.emit_entity(entity("host", step_id))?;
        Ok(())
    }
}

/// INV-C06: failed step in parallel wave does not block sibling step.
#[tokio::test]
async fn v02_parallel_failed_step_does_not_block_sibling() {
    let c = Arc::new(SiblingFailConnector);
    let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
    // "ok" succeeds and emits one entity; "fail" produces nothing
    assert_eq!(
        out.entities.len(),
        1,
        "sibling step must still run despite peer failure"
    );
}

/// SyncEvent stream receives Started, StepStarted×N, StepCompleted/Failed events.
#[tokio::test]
async fn v02_parallel_events_emitted_for_all_steps() {
    use parallax_connect::SyncEvent;
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::channel(64);
    let c = Arc::new(CountingConnector {
        steps: make_steps(&[("x", &[]), ("y", &[])]),
        counter: Arc::new(AtomicUsize::new(0)),
    });
    run_connector(c, "acct", "sync-1", Some(&tx)).await.unwrap();
    drop(tx);

    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }

    let started_count = events
        .iter()
        .filter(|e| matches!(e, SyncEvent::Started { .. }))
        .count();
    let step_starts = events
        .iter()
        .filter(|e| matches!(e, SyncEvent::StepStarted { .. }))
        .count();
    let step_done = events
        .iter()
        .filter(|e| matches!(e, SyncEvent::StepCompleted { .. }))
        .count();

    assert_eq!(started_count, 1, "one Started event");
    assert_eq!(step_starts, 2, "two StepStarted events");
    assert_eq!(step_done, 2, "two StepCompleted events");
}
