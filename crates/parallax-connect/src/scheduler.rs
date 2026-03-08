//! Step scheduler — executes connector steps in dependency waves.
//!
//! Steps within the same wave have no inter-dependencies and run concurrently
//! via `tokio::task::JoinSet`. Waves execute sequentially so downstream steps
//! always see completed upstream data.
//!
//! `run_connector` collects entity/relationship output from all steps.
//! The caller is responsible for committing via `parallax_ingest::commit_sync_exclusive`
//! or `SyncEngine::commit_sync`. This keeps `parallax-connect` free of a
//! direct `parallax-store` dependency (spec §7.2 — acyclic dependency graph).
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.2, §5.6
//!
//! INV-C06: A failed step doesn't prevent independent steps from running.

use std::sync::Arc;
use std::time::Instant;

use parallax_core::{entity::Entity, relationship::Relationship};
use tracing::{info, warn};

use crate::connector::{topological_waves, Connector, PriorStepData, StepContext};
use crate::error::{ConnectorError, SyncError};
use crate::event::SyncEvent;

type StepOutcome = (
    String,
    Result<(StepContext, std::time::Duration), ConnectorError>,
);

/// Output collected from a connector run, ready to be committed.
///
/// Pass this to `parallax_ingest::commit_sync_exclusive` (when you hold
/// `&mut StorageEngine`) or `SyncEngine::commit_sync` (when sharing the
/// engine behind a Mutex).
pub struct ConnectorOutput {
    pub connector_id: String,
    pub sync_id: String,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

/// Run all steps of a connector and return the collected output.
///
/// Steps within the same topological wave execute concurrently via
/// `tokio::task::JoinSet`. Waves run sequentially — downstream steps always
/// see data from all upstream waves via `prior_entities`.
///
/// Failed steps are logged but don't block independent steps (INV-C06).
/// Panicking steps surface as `SyncError::StepPanic`.
///
/// The caller must commit the output separately via the `parallax-ingest`
/// API — this function does **not** write to the storage engine.
pub async fn run_connector(
    connector: Arc<dyn Connector + Send + Sync>,
    account_id: &str,
    sync_id: &str,
    event_tx: Option<&tokio::sync::mpsc::Sender<SyncEvent>>,
) -> Result<ConnectorOutput, SyncError> {
    let connector_id = connector.name().to_owned();

    emit(
        &event_tx,
        SyncEvent::Started {
            connector_id: connector_id.clone(),
            sync_id: sync_id.to_owned(),
        },
    )
    .await;

    let steps = connector.steps();
    let waves = topological_waves(&steps);

    let mut prior = Arc::new(PriorStepData::default());
    let mut all_entities: Vec<Entity> = Vec::new();
    let mut all_relationships: Vec<Relationship> = Vec::new();

    for wave in waves {
        // Emit StepStarted for each step in the wave before spawning.
        for step_id in &wave {
            emit(
                &event_tx,
                SyncEvent::StepStarted {
                    step_id: step_id.clone(),
                },
            )
            .await;
        }

        // Spawn all steps in the wave concurrently.
        let mut join_set: tokio::task::JoinSet<StepOutcome> = tokio::task::JoinSet::new();

        for step_id in wave {
            let c = Arc::clone(&connector);
            let prior_clone = Arc::clone(&prior);
            let cid = connector_id.clone();
            let aid = account_id.to_owned();
            let sid = sync_id.to_owned();

            join_set.spawn(async move {
                let step_start = Instant::now();
                let mut ctx = StepContext::new(&cid, &aid, &sid, prior_clone);
                let result = c.execute_step(&step_id, &mut ctx).await;
                let elapsed = step_start.elapsed();
                (step_id, result.map(|_| (ctx, elapsed)))
            });
        }

        // Collect wave results; merge successful step data into next prior.
        let mut new_prior = PriorStepData::default();
        for (k, v) in prior.entities.iter() {
            new_prior.entities.insert(k.clone(), v.clone());
        }

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((step_id, Ok((ctx, elapsed)))) => {
                    let step_entities = ctx.metrics.entities_emitted;
                    let step_rels = ctx.metrics.relationships_emitted;

                    // Merge this step's entities into the next wave's prior data.
                    for b in &ctx.entities {
                        new_prior.insert(b);
                    }

                    emit(
                        &event_tx,
                        SyncEvent::StepCompleted {
                            step_id: step_id.clone(),
                            entities: step_entities,
                            relationships: step_rels,
                            duration: elapsed,
                        },
                    )
                    .await;

                    // Collect materialised entities and relationships.
                    let source = ctx.source_tag();
                    for b in ctx.entities {
                        all_entities.push(b.build(account_id, source.clone()));
                    }
                    for b in ctx.relationships {
                        if let Some(r) = b.build(account_id, source.clone()) {
                            all_relationships.push(r);
                        }
                    }
                }
                Ok((step_id, Err(e))) => {
                    warn!(step_id = %step_id, error = %e, "Step failed");
                    emit(
                        &event_tx,
                        SyncEvent::StepFailed {
                            step_id: step_id.clone(),
                            error: e,
                        },
                    )
                    .await;
                    // Continue with other steps in the wave (INV-C06).
                }
                Err(join_err) => {
                    return Err(SyncError::StepPanic(join_err.to_string()));
                }
            }
        }

        prior = Arc::new(new_prior);
    }

    info!(
        connector_id = %connector_id,
        sync_id,
        entities = all_entities.len(),
        relationships = all_relationships.len(),
        "connector run complete — awaiting commit"
    );

    Ok(ConnectorOutput {
        connector_id,
        sync_id: sync_id.to_owned(),
        entities: all_entities,
        relationships: all_relationships,
    })
}

async fn emit(tx: &Option<&tokio::sync::mpsc::Sender<SyncEvent>>, event: SyncEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::entity;
    use crate::connector::{step, StepDefinition};
    use crate::error::ConnectorError;
    use async_trait::async_trait;

    struct MockConnector {
        steps: Vec<StepDefinition>,
    }

    #[async_trait]
    impl Connector for MockConnector {
        fn name(&self) -> &str {
            "mock"
        }

        fn steps(&self) -> Vec<StepDefinition> {
            self.steps.clone()
        }

        async fn execute_step(
            &self,
            step_id: &str,
            ctx: &mut StepContext,
        ) -> Result<(), ConnectorError> {
            ctx.emit_entity(entity(step_id, step_id).display_name(step_id))?;
            Ok(())
        }
    }

    struct FailingConnector;

    #[async_trait]
    impl Connector for FailingConnector {
        fn name(&self) -> &str {
            "failing"
        }

        fn steps(&self) -> Vec<StepDefinition> {
            vec![
                step("ok", "ok step").build(),
                step("fail", "always fails").build(),
            ]
        }

        async fn execute_step(
            &self,
            step_id: &str,
            ctx: &mut StepContext,
        ) -> Result<(), ConnectorError> {
            if step_id == "fail" {
                return Err(ConnectorError::UnknownStep("fail".into()));
            }
            ctx.emit_entity(entity("host", step_id))?;
            Ok(())
        }
    }

    fn mock(deps: &[(&str, &[&str])]) -> Arc<MockConnector> {
        Arc::new(MockConnector {
            steps: deps
                .iter()
                .map(|(id, ds)| StepDefinition {
                    id: id.to_string(),
                    description: id.to_string(),
                    depends_on: ds.iter().map(|d| d.to_string()).collect(),
                })
                .collect(),
        })
    }

    #[tokio::test]
    async fn run_connector_no_steps_returns_empty() {
        let c = Arc::new(MockConnector { steps: vec![] });
        let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
        assert!(out.entities.is_empty());
        assert!(out.relationships.is_empty());
    }

    #[tokio::test]
    async fn run_connector_single_step_emits_entity() {
        let c = mock(&[("hosts", &[])]);
        let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
        assert_eq!(out.entities.len(), 1);
    }

    #[tokio::test]
    async fn run_connector_parallel_wave_all_entities_collected() {
        // a, b, c have no deps → all in wave 0, run concurrently.
        let c = mock(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
        assert_eq!(out.entities.len(), 3);
    }

    #[tokio::test]
    async fn run_connector_chain_respects_order() {
        // a → b → c; each emits one entity.
        let c = mock(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
        assert_eq!(out.entities.len(), 3);
    }

    #[tokio::test]
    async fn run_connector_failed_step_does_not_block_others() {
        // Both "ok" and "fail" are in wave 0. "fail" errors; "ok" should still emit.
        let c = Arc::new(FailingConnector);
        let out = run_connector(c, "acct", "sync-1", None).await.unwrap();
        // Only the "ok" step emits an entity.
        assert_eq!(out.entities.len(), 1);
    }

    #[tokio::test]
    async fn run_connector_sends_sync_events() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let c = mock(&[("hosts", &[])]);
        run_connector(c, "acct", "sync-1", Some(&tx)).await.unwrap();
        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        // Expect: Started, StepStarted, StepCompleted.
        assert!(events.len() >= 3);
    }
}
