# Value Types

Parallax properties use a flat, fixed set of value types.

## The Value Enum

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(CompactString),
    Timestamp(Timestamp),
    StringArray(Vec<CompactString>),
}
```

## Type Details

### Null

Represents the absence of a value. Use `Null` for optional properties
that are not set.

```rust
// Rust
entity.properties.insert("terminated_at".into(), Value::Null);

// REST API JSON
"properties": { "terminated_at": null }

// PQL filter
FIND host WITH terminated_at = null
```

### Bool

```rust
// Rust
Value::from(true)
Value::Bool(false)

// REST API JSON
"properties": { "active": true, "mfa_enabled": false }

// PQL filter
FIND user WITH active = true
FIND user WITH mfa_enabled = false
```

### Int (i64)

Integer values in the range `[-2^63, 2^63 - 1]`.

```rust
// Rust
Value::from(443i64)
Value::Int(8080)

// REST API JSON — must be a JSON integer
"properties": { "port": 443, "cpu_count": 8 }

// PQL filter
FIND host WITH cpu_count > 4
FIND service WITH port = 443
```

### Float (f64)

Double-precision floating point.

```rust
// Rust
Value::from(9.8f64)
Value::Float(0.5)

// REST API JSON
"properties": { "score": 9.8, "utilization": 0.75 }

// PQL filter
FIND host WITH cpu_utilization > 0.8
```

### String

Short-to-medium strings, backed by `CompactString` (stack-allocated for
strings ≤24 bytes; heap-allocated for longer strings).

```rust
// Rust
Value::from("running")
Value::String(CompactString::new("us-east-1"))

// REST API JSON
"properties": { "state": "running", "region": "us-east-1" }

// PQL filter (single quotes only)
FIND host WITH state = 'running'
FIND user WITH email LIKE '%@corp.com'
```

### Timestamp

Hybrid Logical Clock timestamp. Primarily used for audit fields like
`created_at`, `updated_at`, `sync_timestamp`.

```rust
// Rust
Value::Timestamp(Timestamp::now())

// REST API JSON — ISO 8601 string
"properties": { "last_seen": "2024-01-15T10:30:00Z" }
```

Not directly filterable in PQL v0.1. Use string properties for
time-based filtering in the current version.

### StringArray

A flat array of strings. Common for tags, labels, and group memberships.
No nesting within arrays (no arrays of objects).

```rust
// Rust
Value::StringArray(vec!["web".into(), "production".into(), "us-east-1".into()])

// REST API JSON
"properties": { "tags": ["web", "production", "us-east-1"] }
```

StringArray is not filterable in PQL v0.1. Filter via scalar properties.

## Type Stability (INV-07/08)

**INV-07:** Property types must be stable within an entity type. If `port`
is an `Int` for `aws_security_group_rule`, it must always be `Int` for
that type. A connector that sends it as a `String` gets a warning in v0.1
and a hard error in v0.2.

**INV-08:** Properties are flat — no nested objects or arrays-of-objects.

## JSON Type Mapping

| JSON Type | Parallax Value |
|---|---|
| `null` | `Value::Null` |
| `true` / `false` | `Value::Bool` |
| Integer (no decimal) | `Value::Int` |
| Number (with decimal) | `Value::Float` |
| String | `Value::String` |
| Array of strings | `Value::StringArray` |
| Object | **Rejected** — no nested objects |
| Array of objects | **Rejected** — no nested arrays |
