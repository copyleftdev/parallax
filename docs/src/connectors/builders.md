# Entity & Relationship Builders

The builder API is the primary way to construct entities and relationships
in connector code. It uses a fluent interface designed to be readable and
hard to misuse.

## Entity Builder

### `entity(type, key)` — Start a Builder

```rust
use parallax_connect::builder::entity;

let builder = entity("host", "web-01");
```

`type` is the entity type (open set, snake_case, e.g., `"host"`, `"aws_ec2_instance"`).
`key` is the source-system's unique identifier for this entity.

### Builder Methods

```rust
entity("host", "web-01")
    // Required: entity class (closed set of ~40 values)
    .class("Host")

    // Optional: human-readable name
    .display_name("Web Server 01")

    // Add a single property (value can be any Value type)
    .property("state", "running")        // String
    .property("cpu_count", 4i64)         // Integer
    .property("memory_gb", 32.0f64)      // Float
    .property("active", true)            // Boolean
    .property("terminated_at", Value::Null)  // Null

    // Add multiple properties at once
    .properties([
        ("region", Value::from("us-east-1")),
        ("az", Value::from("us-east-1a")),
    ])
```

### `.property()` Value Types

The `.property()` method accepts anything that implements `Into<Value>`:

| Rust Type | PQL Value Type | Example |
|---|---|---|
| `&str`, `String` | String | `.property("state", "running")` |
| `i64`, `i32`, `usize` | Int | `.property("port", 443i64)` |
| `f64`, `f32` | Float | `.property("score", 9.8f64)` |
| `bool` | Bool | `.property("active", true)` |
| `Value::Null` | Null | `.property("deleted_at", Value::Null)` |

### Emitting the Entity

```rust
ctx.emit_entity(
    entity("host", "web-01")
        .class("Host")
        .display_name("Web Server 01")
        .property("state", "running")
)?;
```

`emit_entity()` returns `Result<(), ConnectorError>`. The error is returned
if the entity builder is invalid (e.g., missing class).

## Relationship Builder

### `relationship(from_type, from_key, verb, to_type, to_key)` — Start a Builder

```rust
use parallax_connect::builder::relationship;

let builder = relationship("host", "web-01", "RUNS", "service", "nginx");
```

### Builder Methods

```rust
relationship("host", "web-01", "RUNS", "service", "nginx")
    // Add properties to the edge (optional)
    .property("since", "2024-01-15")
    .property("port", 8080i64)
```

### Emitting the Relationship

```rust
ctx.emit_relationship(
    relationship("host", "web-01", "RUNS", "service", "nginx")
        .property("port", 443i64)
)?;
```

**Important:** Both endpoints of the relationship must exist in the current
batch or in the committed graph. Emitting a relationship to a non-existent
entity is not an error at emit time, but it will be rejected at commit time
(INV-C04).

## Full Example

```rust
async fn collect_hosts_and_services(ctx: &mut StepContext) -> Result<(), ConnectorError> {
    // Emit host
    ctx.emit_entity(
        entity("host", "web-01")
            .class("Host")
            .display_name("Web Server 01")
            .property("state", "running")
            .property("region", "us-east-1")
            .property("cpu_count", 8i64)
            .property("memory_gb", 32.0f64)
    )?;

    // Emit service
    ctx.emit_entity(
        entity("service", "nginx-web-01")
            .class("Service")
            .display_name("Nginx on web-01")
            .property("port", 443i64)
            .property("protocol", "https")
    )?;

    // Emit relationship (host RUNS service)
    ctx.emit_relationship(
        relationship("host", "web-01", "RUNS", "service", "nginx-web-01")
            .property("since", "2024-01-15")
    )?;

    Ok(())
}
```

## StepContext Metrics

After emitting, you can inspect how many entities/relationships were emitted:

```rust
ctx.emit_entity(...)?;
ctx.emit_entity(...)?;
ctx.emit_relationship(...)?;

// Metrics for this step
println!("Entities emitted: {}", ctx.metrics.entities_emitted);
println!("Relationships emitted: {}", ctx.metrics.relationships_emitted);
```
