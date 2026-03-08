# Entity Finder

The entity finder is a fluent builder for filtering entities from a snapshot.
It uses secondary indices (type, class) when available and falls back to
a full scan for property-level filters.

## Basic Usage

```rust
let graph = GraphReader::new(&snap);

// Find all entities of a type
let hosts: Vec<&Entity> = graph.find("host").collect();

// Find all entities of a class
let all_hosts: Vec<&Entity> = graph
    .find_by_class("Host")
    .collect();

// Find with a property filter
let running: Vec<&Entity> = graph
    .find("host")
    .with_property("state", Value::from("running"))
    .collect();
```

## EntityFinder Methods

```rust
impl<'snap> EntityFinder<'snap> {
    /// Filter to entities of the specified class.
    /// Uses class index — no full scan.
    pub fn class(self, c: &str) -> Self;

    /// Filter to entities where the named property equals the value.
    /// Uses full scan (no property index in v0.1).
    pub fn with_property(self, key: &str, value: Value) -> Self;

    /// Limit the number of results.
    pub fn limit(self, n: usize) -> Self;

    /// Collect all matching entities.
    pub fn collect(self) -> Vec<&'snap Entity>;

    /// Count matching entities without materializing them.
    pub fn count(self) -> usize;
}
```

## GraphReader Finder Variants

```rust
impl<'snap> GraphReader<'snap> {
    /// Find entities by type ("host", "aws_ec2_instance").
    /// Uses the type index.
    pub fn find(&self, entity_type: &str) -> EntityFinder<'snap>;

    /// Find entities by class ("Host", "User", "DataStore").
    /// Uses the class index.
    pub fn find_by_class(&self, class: &str) -> EntityFinder<'snap>;

    /// Find all entities (no type filter). Full scan.
    pub fn find_all(&self) -> EntityFinder<'snap>;

    /// Direct lookup by EntityId. O(1).
    pub fn get_entity(&self, id: EntityId) -> Option<&'snap Entity>;
}
```

## Examples

### Find running hosts

```rust
let running_hosts = graph
    .find("host")
    .with_property("state", Value::from("running"))
    .collect();
```

### Count all services

```rust
let count = graph.find_by_class("Service").count();
```

### Find with limit

```rust
// First 100 hosts for pagination
let page = graph.find("host").limit(100).collect();
```

## Index Strategy

The planner chooses the access strategy based on the query:

| Filter | Access Method | Cost |
|---|---|---|
| Type only | Type index lookup | O(n_type) |
| Class only | Class index lookup | O(n_class) |
| Type + property | Type index + filter scan | O(n_type) |
| Class + property | Class index + filter scan | O(n_class) |
| Property only | Full scan | O(n_total) |

Property-level secondary indexes are planned for v0.2. For v0.1, property
filters always require a linear scan over the type/class result set.

## Deleted Entity Handling

The finder automatically excludes soft-deleted entities (`_deleted = true`).
You never see deleted entities in query results (INV-S08).
