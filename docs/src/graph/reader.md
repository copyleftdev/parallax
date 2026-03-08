# GraphReader

`GraphReader<'snap>` is the primary entry point for all graph operations.
It wraps an MVCC snapshot and provides typed, zero-copy access to graph data.

## Creating a GraphReader

```rust
use parallax_graph::GraphReader;
use parallax_store::StorageEngine;

let engine = StorageEngine::open(config)?;
let snap = engine.snapshot();      // O(1), no lock after this
let graph = GraphReader::new(&snap);
// graph can now be used freely; snap must not be dropped while graph is in scope
```

## Full API Reference

```rust
impl<'snap> GraphReader<'snap> {
    pub fn new(snapshot: &'snap Snapshot) -> Self;

    // ── Entity finding ────────────────────────────────────────────────
    /// Find entities by type. Uses type index.
    pub fn find(&self, entity_type: &str) -> EntityFinder<'snap>;

    /// Find entities by class. Uses class index.
    pub fn find_by_class(&self, class: &str) -> EntityFinder<'snap>;

    /// Find all entities. Full scan.
    pub fn find_all(&self) -> EntityFinder<'snap>;

    /// O(1) point lookup by EntityId.
    pub fn get_entity(&self, id: EntityId) -> Option<&'snap Entity>;

    /// O(1) point lookup by RelationshipId.
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&'snap Relationship>;

    // ── Traversal ─────────────────────────────────────────────────────
    /// Start a traversal from a specific entity.
    pub fn traverse(&self, start: EntityId) -> TraversalBuilder<'snap>;

    // ── Path finding ──────────────────────────────────────────────────
    /// Find the shortest path between two entities.
    pub fn shortest_path(&self, from: EntityId, to: EntityId) -> ShortestPathBuilder<'snap>;

    // ── Blast radius ──────────────────────────────────────────────────
    /// Compute the blast radius from a target entity.
    pub fn blast_radius(&self, origin: EntityId) -> BlastRadiusBuilder<'snap>;

    // ── Coverage analysis ─────────────────────────────────────────────
    /// Find entities missing a qualifying neighbor via the given verb.
    pub fn coverage_gap(&self, verb: &str) -> CoverageGapBuilder<'snap>;
}
```

## Lifetime Annotation

The `'snap` lifetime on `GraphReader<'snap>` means:
- Every `&'snap Entity` reference returned by the reader borrows from the snapshot
- The snapshot must outlive the reader and all references derived from it
- The Rust borrow checker enforces this at compile time — no runtime cost

This design guarantees:
- **Zero-copy reads:** Entity data is never cloned from the snapshot
- **No use-after-free:** You cannot hold a reference to freed snapshot data
- **No stale reads:** You always read from the snapshot you created the reader with

## Collecting vs Cloning

Since `GraphReader` returns references with snapshot lifetimes, you must
either use the data within the snapshot's scope or clone it:

```rust
// Use within scope — zero allocation
let snap = engine.snapshot();
let graph = GraphReader::new(&snap);
let count = graph.find("host").count(); // no allocation
let first_host = graph.find("host").collect().first().map(|e| e.display_name.as_str());

// Clone if you need owned data past the snapshot
let hosts: Vec<Entity> = graph
    .find("host")
    .collect()
    .into_iter()
    .cloned()  // Clone here when crossing async boundaries
    .collect();
drop(snap); // now safe to drop
```

## In Server Handlers

```rust
async fn list_hosts(State(state): State<AppState>) -> Json<Value> {
    let hosts = {
        let engine = state.engine.lock().unwrap();
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        // Collect into owned Vec before dropping snap + lock
        graph.find("host")
            .collect()
            .into_iter()
            .cloned()
            .collect::<Vec<Entity>>()
    }; // engine lock + snap released here

    Json(json!({ "entities": hosts.iter().map(entity_to_json).collect::<Vec<_>>() }))
}
```
