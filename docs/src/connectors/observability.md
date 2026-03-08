# Connector Observability

The connector SDK provides structured events and logging for monitoring
sync executions.

## SyncEvent Stream

Pass a `tokio::sync::mpsc::Sender<SyncEvent>` to `run_connector` to receive
real-time events during the sync:

```rust
use tokio::sync::mpsc;
use parallax_connect::{run_connector, SyncEvent};

let (tx, mut rx) = mpsc::channel(100);

// Spawn event consumer
tokio::spawn(async move {
    while let Some(event) = rx.recv().await {
        match event {
            SyncEvent::Started { connector_id, sync_id } => {
                tracing::info!(%connector_id, %sync_id, "Sync started");
            }
            SyncEvent::StepStarted { step_id } => {
                tracing::info!(%step_id, "Step started");
            }
            SyncEvent::StepCompleted { step_id, entities, relationships, duration } => {
                tracing::info!(
                    %step_id,
                    entities_emitted = entities,
                    relationships_emitted = relationships,
                    duration_ms = duration.as_millis(),
                    "Step completed"
                );
            }
            SyncEvent::StepFailed { step_id, error } => {
                tracing::warn!(%step_id, %error, "Step failed");
            }
            SyncEvent::Completed { connector_id, sync_id } => {
                tracing::info!(%connector_id, %sync_id, "Sync completed");
            }
        }
    }
});

let output = run_connector(&connector, "account", "sync-001", Some(&tx)).await?;
```

## SyncEvent Variants

```rust
pub enum SyncEvent {
    Started {
        connector_id: String,
        sync_id: String,
    },
    StepStarted {
        step_id: String,
    },
    StepCompleted {
        step_id: String,
        entities: usize,
        relationships: usize,
        duration: Duration,
    },
    StepFailed {
        step_id: String,
        error: ConnectorError,
    },
    Completed {
        connector_id: String,
        sync_id: String,
    },
}
```

## Structured Logging

The scheduler uses `tracing` for structured logs. Enable them with any
`tracing` subscriber:

```rust
tracing_subscriber::fmt::init();
```

Relevant log fields:
- `connector_id` — identifies the connector
- `sync_id` — identifies this specific run
- `step_id` — identifies the current step
- `entities` — count of entities emitted in a step
- `relationships` — count of relationships emitted

## Metrics Integration

After a sync, the `SyncStats` from `commit_sync` / `commit_sync_exclusive`
provides counters suitable for Prometheus export:

```rust
let stats = result.stats;

// Export to your metrics system
metrics::counter!("parallax_entities_created_total")
    .increment(stats.entities_created);
metrics::counter!("parallax_entities_deleted_total")
    .increment(stats.entities_deleted);
metrics::counter!("parallax_relationships_created_total")
    .increment(stats.relationships_created);
```

Or use the built-in Prometheus endpoint when running `parallax-server`:
`GET /metrics` returns engine-wide counters.

## Tracing Context Propagation

When running connectors inside an existing trace context, the connector
steps inherit the current span:

```rust
let span = tracing::info_span!("sync_run", connector = "my-service");
let output = async move {
    run_connector(&connector, "account", "sync-001", None).await
}
.instrument(span)
.await?;
```

All log output from within `run_connector` will be nested under this span.
