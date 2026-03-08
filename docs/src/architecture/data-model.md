# Data Model

Parallax models the world as a **typed property graph**:

- **Entities** are nodes. Each has a type, a class, and a bag of flat properties.
- **Relationships** are directed edges. Each has a verb (class), connects two
  entities, and may carry properties.
- **Properties** are flat key-value pairs. No nesting. If you need structure,
  model it as another entity and a relationship.

## Entity Identity

### Design Goals

An entity ID must be:
1. **Stable** — the same logical entity always gets the same ID.
2. **Deterministic** — given the same inputs, we always produce the same ID.
3. **Collision-free** — two different entities never share an ID.
4. **Source-independent** — the ID doesn't change if we re-ingest from a
   different connector.

### Implementation

```rust
/// EntityId is a 128-bit blake3 hash of (account_id, entity_type, entity_key).
/// It is never randomly generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub [u8; 16]);

impl EntityId {
    pub fn derive(account_id: &str, entity_type: &str, entity_key: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(account_id.as_bytes());
        hasher.update(b":");
        hasher.update(entity_type.as_bytes());
        hasher.update(b":");
        hasher.update(entity_key.as_bytes());
        let hash = hasher.finalize();
        let mut id = [0u8; 16];
        id.copy_from_slice(&hash.as_bytes()[..16]);
        EntityId(id)
    }
}
```

**Why blake3:** Fast (SIMD-accelerated), cryptographically strong, deterministic,
no seed needed. 128-bit truncation gives collision probability of ~1 in 10^18
at 10 billion entities.

**Why not UUID v4:** UUIDs are random and not deterministic from source data.
We need idempotent re-ingestion — inserting the same entity twice must produce
the same ID, not a duplicate.

## Entity Structure

```rust
pub struct Entity {
    /// Deterministic, content-addressed identity.
    pub id: EntityId,

    /// Specific type: "aws_ec2_instance", "okta_user", "github_repo".
    /// Open set — connectors define new types freely.
    pub _type: EntityType,

    /// Broad classification: "Host", "User", "DataStore".
    /// Closed set of ~40 values. Enables cross-type queries.
    pub _class: EntityClass,

    /// Human-readable display name. Not unique, not stable.
    pub display_name: CompactString,

    /// Which connector + sync cycle produced this version.
    pub source: SourceTag,

    /// When this entity was first observed by Parallax.
    pub created_at: Timestamp,

    /// When this entity was last modified.
    pub updated_at: Timestamp,

    /// Soft-delete flag. Set by sync diff when entity disappears from source.
    pub _deleted: bool,

    /// Flat key-value property bag.
    pub properties: PropertyMap,
}
```

## Type vs. Class: The Two-Level Hierarchy

```
Class (broad, ~40 total)     Type (specific, unbounded)
─────────────────────────    ───────────────────────────────
Host                         aws_ec2_instance, azure_vm, host
User                         okta_user, aws_iam_user, user
DataStore                    aws_s3_bucket, aws_dynamodb_table
CodeRepo                     github_repo, gitlab_project
Firewall                     aws_security_group, gcp_firewall_rule
Service                      service, nginx, microservice
```

**Class** enables generic queries across cloud providers:
`FIND Host WITH active = true`

**Type** enables specific queries:
`FIND aws_ec2_instance WITH instanceType = 'm5.xlarge'`

Class is a **closed set** (defined by Parallax, ~40 values). Type is an
**open set** (defined by connectors, unlimited). See the full class list in
[Known Entity Classes](../reference/entity-classes.md).

## Relationship Structure

```rust
pub struct Relationship {
    /// Deterministic identity derived from (from_id, class, to_id).
    pub id: RelationshipId,

    /// The verb: HAS, RUNS, CONTAINS, ALLOWS, etc.
    pub _class: RelationshipClass,

    /// Source entity. Must exist at commit time.
    pub from_id: EntityId,

    /// Target entity. Must exist at commit time.
    pub to_id: EntityId,

    /// Optional properties on the relationship edge.
    pub properties: PropertyMap,

    pub source: SourceTag,
    pub _deleted: bool,
}
```

**Referential integrity (INV-03):** The `from_id` and `to_id` of every committed
relationship must reference entities that exist in the graph. The ingest layer
enforces this at commit time — dangling relationships are rejected, not silently
stored.

## Property Values

Properties are flat key-value pairs with a small set of value types:

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

pub type PropertyMap = BTreeMap<CompactString, Value>;
```

**Why no nested objects:** Flat properties are indexable, queryable, and diff
cleanly (key-by-key comparison). JSON path expressions inside a graph query
create a second query language inside the first. If your source data has nested
objects, either flatten them into properties, or model the structure as entities
and relationships.

## Source Tracking

Every entity and relationship knows which connector produced it:

```rust
pub struct SourceTag {
    /// Connector identifier: "connector-aws", "my-scanner", etc.
    pub connector_id: CompactString,
    /// Unique ID for this specific sync execution.
    pub sync_id: CompactString,
    /// When this sync started (HLC timestamp).
    pub sync_timestamp: Timestamp,
}
```

This enables:
1. **Differential sync:** Delete entities from connector X that weren't seen in
   the latest sync — without touching entities from connector Y.
2. **Provenance:** Know which connector is the authority for each entity.

## Hybrid Logical Clocks

```rust
pub struct Timestamp {
    /// Milliseconds since Unix epoch (wall clock component).
    pub wall_ms: u64,
    /// Logical counter — breaks ties when wall_ms is equal.
    pub logical: u32,
    /// Node identifier (0 for single-node; reserved for clustering).
    pub node_id: u16,
}
```

HLC is used instead of wall-clock `SystemTime` because:
- Wall clocks can go backwards (NTP adjustments).
- Two events in the same millisecond need deterministic ordering.
- HLC extends naturally when clustering is added.

## Invariants

```
INV-01: Every entity has a non-empty _type, _class, and entity_key.
INV-02: EntityId is deterministic: same (account, type, key) → same id.
INV-03: Every relationship's from_id and to_id reference existing entities.
INV-04: No two entities in the same account share (type, key).
INV-05: No two relationships share (from_id, class, to_id) unless
        explicitly keyed with derive_with_key.
INV-06: Timestamps are monotonically increasing per node.
INV-07: Property types are stable within an entity type across versions.
```
