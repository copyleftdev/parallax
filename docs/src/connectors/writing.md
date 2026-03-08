# Writing a Connector

A complete guide to implementing the `Connector` trait.

## 1. Create a Crate

Connectors are separate crates depending on `parallax-connect`:

```toml
# Cargo.toml
[package]
name = "connector-myservice"
version = "0.1.0"
edition = "2021"

[dependencies]
parallax-connect = { path = "../parallax-connect" }
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
# ... your API client crate
```

## 2. Define the Struct

```rust
use parallax_connect::prelude::*;

pub struct MyServiceConnector {
    api_base_url: String,
    api_token: String,
}

impl MyServiceConnector {
    pub fn new(api_base_url: impl Into<String>, api_token: impl Into<String>) -> Self {
        MyServiceConnector {
            api_base_url: api_base_url.into(),
            api_token: api_token.into(),
        }
    }
}
```

## 3. Define Steps

Steps are the units of collection. Each step is independent or depends on
prior steps. Define them in the `steps()` method:

```rust
#[async_trait]
impl Connector for MyServiceConnector {
    fn name(&self) -> &str {
        "my-service"
    }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            step("users", "Collect users").build(),
            step("hosts", "Collect hosts").build(),
            step("services", "Collect services")
                .depends_on(&["hosts"])     // runs after "hosts"
                .build(),
            step("relationships", "Collect relationships")
                .depends_on(&["users", "hosts", "services"])  // runs last
                .build(),
        ]
    }
    // ...
}
```

**INV-C05:** Step dependencies must form a DAG. Cycles are rejected at
connector load time.

**INV-C06:** A failed step does not prevent independent steps from running.
Steps that don't depend on the failed step still execute.

## 4. Implement Steps

```rust
#[async_trait]
impl Connector for MyServiceConnector {
    // ... name() and steps() ...

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "users" => self.collect_users(ctx).await,
            "hosts" => self.collect_hosts(ctx).await,
            "services" => self.collect_services(ctx).await,
            "relationships" => self.collect_relationships(ctx).await,
            _ => Err(ConnectorError::UnknownStep(step_id.to_string())),
        }
    }
}
```

## 5. Emit Entities

```rust
impl MyServiceConnector {
    async fn collect_users(&self, ctx: &mut StepContext) -> Result<(), ConnectorError> {
        let users = self.fetch_users_from_api().await?;

        for user in users {
            ctx.emit_entity(
                entity("user", &user.id)          // (type, key)
                    .class("User")                // entity class
                    .display_name(&user.name)
                    .property("email", user.email.as_str())
                    .property("active", user.active)
                    .property("mfa_enabled", user.mfa_enabled)
            )?;
        }
        Ok(())
    }
}
```

`ctx.emit_entity()` returns `Result<(), ConnectorError>`. Emit errors are
non-fatal by default — they log a warning but don't stop the step.
To make them fatal, propagate with `?`.

## 6. Emit Relationships

```rust
async fn collect_relationships(
    &self,
    ctx: &mut StepContext,
) -> Result<(), ConnectorError> {
    let assignments = self.fetch_role_assignments().await?;

    for assignment in assignments {
        ctx.emit_relationship(
            relationship(
                "user",         // from_type
                &assignment.user_id,  // from_key
                "ASSIGNED",     // verb
                "role",         // to_type
                &assignment.role_id,  // to_key
            )
            .property("assigned_at", assignment.timestamp.to_string().as_str())
        )?;
    }
    Ok(())
}
```

**INV-C04:** Referential integrity is enforced at commit time. A relationship
whose `from_key` or `to_key` doesn't exist in the batch *or* the current graph
will be rejected. Emit entities before the relationships that reference them.

## 7. Access Prior Step Data

Steps can read entities emitted by their dependencies:

```rust
async fn collect_services(
    &self,
    ctx: &mut StepContext,
) -> Result<(), ConnectorError> {
    // Read hosts emitted by the "hosts" step
    let host_ids: Vec<String> = ctx.prior
        .entities_by_type("host")
        .iter()
        .map(|e| e.entity_key.to_string())
        .collect();

    // Use host IDs to fetch services from the API
    for host_id in host_ids {
        let services = self.fetch_services_for_host(&host_id).await?;
        for service in services {
            ctx.emit_entity(
                entity("service", &service.id)
                    .class("Service")
                    .display_name(&service.name)
            )?;
        }
    }
    Ok(())
}
```

## 8. Error Handling

```rust
use parallax_connect::ConnectorError;

// For unrecognized step IDs (always include this)
Err(ConnectorError::UnknownStep(step_id.to_string()))

// For API errors
Err(ConnectorError::Custom(format!("API error: {}", response.status())))

// For configuration errors
Err(ConnectorError::Configuration("API token is empty".to_string()))
```

Connector errors are logged and reported in `SyncEvent::StepFailed`.
They do **not** abort the entire sync — independent steps still run.

## 9. Run the Connector

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let connector = MyServiceConnector::new(
        "https://api.myservice.com",
        std::env::var("MYSERVICE_TOKEN")?,
    );

    let mut engine = StorageEngine::open(StoreConfig::new("./data"))?;

    let output = parallax_connect::run_connector(
        &connector,
        "my-account-id",  // account_id
        "sync-001",       // sync_id (unique per run)
        None,             // event_tx (optional observability channel)
    ).await?;

    let result = parallax_ingest::commit_sync_exclusive(
        &mut engine,
        &output.connector_id,
        &output.sync_id,
        output.entities,
        output.relationships,
    )?;

    println!("Sync complete:");
    println!("  Created: {}", result.stats.entities_created);
    println!("  Updated: {}", result.stats.entities_updated);
    println!("  Deleted: {}", result.stats.entities_deleted);
    println!("  Relationships created: {}", result.stats.relationships_created);

    Ok(())
}
```
