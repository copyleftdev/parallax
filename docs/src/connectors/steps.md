# Step Definitions

Steps are the units of collection within a connector. They enable parallel
execution of independent collection tasks and sequential execution of
dependent tasks.

## Defining Steps

```rust
fn steps(&self) -> Vec<StepDefinition> {
    vec![
        // Independent steps (no dependencies — can run in parallel in future)
        step("users", "Collect IAM users").build(),
        step("roles", "Collect IAM roles").build(),
        step("hosts", "Collect EC2 instances").build(),

        // Dependent steps (run after their dependencies complete)
        step("policies", "Collect IAM policies")
            .depends_on(&["roles"])
            .build(),

        step("services", "Collect services on hosts")
            .depends_on(&["hosts"])
            .build(),

        // Step that depends on multiple prior steps
        step("relationships", "Wire all relationships")
            .depends_on(&["users", "roles", "policies", "hosts", "services"])
            .build(),
    ]
}
```

## StepDefinition Builder

```rust
// Start a step definition
step(id: &str, description: &str)

// Methods
.depends_on(step_ids: &[&str]) -> Self    // declare dependencies
.build() -> StepDefinition                // finalize
```

## Execution Order — Parallel Waves

The scheduler groups steps into **topological waves**. Steps within the same
wave have no inter-dependencies and execute concurrently via
`tokio::task::JoinSet`. Waves execute sequentially so downstream steps always
see completed upstream data.

```
Given: users, roles, hosts, policies(→roles), services(→hosts), relationships(→all)

Wave 0 (parallel): users, roles, hosts       ← no dependencies
Wave 1 (parallel): policies, services        ← depend only on wave 0
Wave 2 (sequential): relationships           ← depends on everything
```

The wave grouping is computed by assigning each step a level:
`level = max(level of dependencies) + 1`, with roots at level 0.

## INV-C05: No Cycles

Step dependencies must form a DAG. Circular dependencies are detected at
connector load time and return an error:

```rust
// This will fail validation:
step("a", "Step A").depends_on(&["b"]).build(),
step("b", "Step B").depends_on(&["a"]).build(),
// Error: "cycle detected in step dependencies: a -> b -> a"
```

## INV-C06: Fault Isolation

A failed step does not prevent **sibling** steps in the same wave from running.
The scheduler logs the error and the wave completes with whatever steps
succeeded:

```
Wave 0:
  Step "hosts" completed: 100 entities
  Step "users" FAILED: API timeout after 30s   ← sibling steps still run
  Step "roles" completed: 25 entities           ← unaffected by "users" failure

Wave 1:
  Step "policies" completed: 50 entities        ← runs despite "users" failure
```

Steps that *depend on* a failed step are not skipped automatically — they run
but see an incomplete `prior_entities` set (the failed step contributed
nothing to it).

## Prior Step Data

Downstream steps can read entities emitted by their dependencies via
`ctx.prior`:

```rust
async fn collect_services(
    &self,
    step_id: &str,
    ctx: &mut StepContext,
) -> Result<(), ConnectorError> {
    // Access entities from prior steps
    for host in ctx.prior.entities_by_type("host") {
        let services = self.api_client
            .get_services_for_host(&host.entity_key)
            .await?;
        // emit services...
    }
    Ok(())
}
```

`ctx.prior` is a snapshot of all entities emitted by all successfully
completed prior steps (not just direct dependencies — all prior steps).

## Step Naming Conventions

| Convention | Example |
|---|---|
| Lowercase kebab-case | `"iam-users"`, `"ec2-instances"` |
| Noun or noun-phrase | `"users"`, `"role-assignments"` |
| No spaces | `"security-groups"` not `"security groups"` |

Steps IDs are used in log output and `SyncEvent` messages, so choose
descriptive names.
