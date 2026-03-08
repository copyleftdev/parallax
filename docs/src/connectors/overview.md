# Connector SDK Overview

The `parallax-connect` crate is the extension surface of Parallax — the part
third-party developers touch most. It defines the `Connector` trait and
provides all the infrastructure for collecting and publishing data.

## What a Connector Does

A connector bridges an external data source (AWS, Okta, GitHub, a scanner, etc.)
and the Parallax graph. It:

1. Authenticates with the external API
2. Collects entities and relationships (in parallel steps)
3. Emits them via the SDK
4. The SDK handles diffing, validation, and atomic commit

The connector author never writes to the storage engine directly. They
implement the `Connector` trait and call `ctx.emit_entity()` /
`ctx.emit_relationship()` — the framework handles the rest.

## The Lifecycle

```
┌──────────┐   ┌───────────┐   ┌──────────┐   ┌───────────┐   ┌──────────┐
│ Configure │──►│ Discover  │──►│ Collect  │──►│  Publish  │──►│  Commit  │
│(auth,     │   │(validate  │   │(fetch    │   │ (SDK diff │   │ (atomic  │
│ settings) │   │ creds)    │   │ assets)  │   │  + queue) │   │  write)  │
└──────────┘   └───────────┘   └──────────┘   └───────────┘   └──────────┘
```

Steps 1–4 are in the connector. Step 5 (commit) is handled by the scheduler
or the caller. The separation is deliberate: `parallax-connect` has no
dependency on `parallax-store`, keeping the dependency graph acyclic.

## Quick Example

```rust
use parallax_connect::prelude::*;

pub struct MyConnector;

#[async_trait]
impl Connector for MyConnector {
    fn name(&self) -> &str { "my-connector" }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            step("hosts", "Collect hosts").build(),
            step("services", "Collect services")
                .depends_on(&["hosts"])
                .build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "hosts" => {
                ctx.emit_entity(
                    entity("host", "web-01")
                        .class("Host")
                        .display_name("Web Server 01")
                        .property("state", "running")
                )?;
                Ok(())
            }
            "services" => {
                ctx.emit_entity(
                    entity("service", "nginx")
                        .class("Service")
                        .display_name("Nginx")
                )?;
                ctx.emit_relationship(
                    relationship("host", "web-01", "RUNS", "service", "nginx")
                )?;
                Ok(())
            }
            _ => Err(ConnectorError::UnknownStep(step_id.to_string())),
        }
    }
}
```

## Running a Connector

```rust
use parallax_connect::run_connector;
use parallax_ingest::commit_sync_exclusive;

let output = run_connector(&MyConnector, "my-account", "sync-001", None).await?;
let result = commit_sync_exclusive(
    &mut engine,
    &output.connector_id,
    &output.sync_id,
    output.entities,
    output.relationships,
)?;

println!("Created: {}", result.stats.entities_created);
println!("Updated: {}", result.stats.entities_updated);
println!("Deleted: {}", result.stats.entities_deleted);
```

Or with `SyncEngine` for shared engine access:

```rust
use parallax_connect::run_connector;
use parallax_ingest::SyncEngine;

let output = run_connector(&connector, "my-account", "sync-002", Some(&event_tx)).await?;
sync_engine.commit_sync(
    &output.connector_id,
    &output.sync_id,
    output.entities,
    output.relationships,
)?;
```

## Key Principles

- **Idempotent:** Running the same connector twice with the same source data
  produces no changes (entities_unchanged = n, created = 0, deleted = 0).
- **Source-scoped:** Connector A's data is never deleted by connector B's sync.
- **Atomic:** Either the entire sync batch lands or none of it does.
- **Fault-tolerant:** A failed step does not prevent independent steps from running.

See [Writing a Connector](./writing.md) for the full guide.
