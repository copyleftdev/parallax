# StorageEngine API

`StorageEngine` is the top-level coordinator that ties together the WAL,
MemTable, Segments, and SnapshotManager.

## Opening an Engine

```rust
use parallax_store::{StorageEngine, StoreConfig};

// Open an existing engine or create a new one
let engine = StorageEngine::open(StoreConfig::new("/var/lib/parallax"))?;
```

`StoreConfig` accepts:

```rust
pub struct StoreConfig {
    /// Root directory for all engine data.
    pub data_dir: PathBuf,
    /// Flush MemTable to segment when size exceeds this (default: 64MB).
    pub memtable_flush_size: usize,
    /// Maximum WAL segment size before rotation (default: 64MB).
    pub wal_segment_max_size: u64,
}

impl StoreConfig {
    /// Create a config with default settings.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self { ... }
}
```

## Writing Data

All writes go through `WriteBatch`:

```rust
use parallax_store::WriteBatch;

let mut batch = WriteBatch::new();

// Upsert an entity (insert or update)
batch.upsert_entity(entity);

// Upsert a relationship
batch.upsert_relationship(relationship);

// Soft-delete an entity
batch.delete_entity(entity_id);

// Soft-delete a relationship
batch.delete_relationship(rel_id);

// Commit atomically (WAL + MemTable + snapshot publish)
engine.write(batch)?;
```

`WriteBatch` is opaque — you cannot inspect it after creation. The write is
atomic: either all operations succeed or none are applied.

### WriteOp Internals

Under the hood, `WriteBatch` is a `Vec<WriteOp>`:

```rust
pub enum WriteOp {
    UpsertEntity(Entity),
    UpsertRelationship(Relationship),
    DeleteEntity(EntityId),
    DeleteRelationship(RelationshipId),
}

pub struct WriteBatch {
    pub(crate) ops: Vec<WriteOp>,
}

impl WriteBatch {
    pub fn is_empty(&self) -> bool { self.ops.is_empty() }
    pub fn len(&self) -> usize { self.ops.len() }
}
```

## Reading Data

All reads go through a `Snapshot`:

```rust
// Acquire a snapshot (O(1), no lock)
let snap = engine.snapshot();

// Point lookups
let entity = snap.get_entity(entity_id);
let rel = snap.get_relationship(rel_id);

// Type/class scans (uses MemTable indices)
let hosts = snap.entities_by_type(&EntityType::new_unchecked("host"));
let services = snap.entities_by_class(&EntityClass::new_unchecked("Service"));

// Source-scoped scans (for sync diff)
let from_aws = snap.entities_by_source("connector-aws");

// Counts
let total = snap.entity_count();
```

## Metrics

```rust
// Get a snapshot of current engine metrics
let metrics = engine.metrics().snapshot();
println!("Total entities: {}", metrics.entity_count);
println!("Total relationships: {}", metrics.relationship_count);
println!("Writes: {}", metrics.writes_total);
println!("Reads: {}", metrics.reads_total);
```

Metrics are maintained as atomic counters and can be read from any thread
without acquiring the engine lock.

## Engine in a Shared Context

In server mode, the engine is shared via `Arc<Mutex<StorageEngine>>`:

```rust
use std::sync::{Arc, Mutex};
use parallax_store::StorageEngine;

let engine = StorageEngine::open(config)?;
let shared = Arc::new(Mutex::new(engine));

// In a handler:
let snap = {
    let engine = shared.lock().unwrap();
    engine.snapshot()
    // Lock released here — snapshot is Arc-owned, not borrowed from engine
};
// Use snap freely without holding the lock
```

## Error Types

```rust
pub enum StoreError {
    /// I/O error from the OS (file not found, permission denied, etc.)
    Io(io::Error),
    /// Data corruption detected (bad magic, CRC mismatch, etc.)
    Corruption(String),
    /// Serialization error (postcard format error)
    Serialization(String),
}
```

`StoreError` implements `std::error::Error` and `Display`. Use `?` to
propagate it up the call stack.

## Thread Safety

`StorageEngine` is `Send` but not `Sync`. Wrap it in `Arc<Mutex<StorageEngine>>`
for shared access. The lock should be held as briefly as possible:

```rust
// Good: lock, snapshot, release, use
let snap = engine.lock().unwrap().snapshot();
let entity = snap.get_entity(id);

// Bad: hold lock for the duration of graph computation
let engine = engine.lock().unwrap();
let entity = engine.snapshot().get_entity(id); // lock held too long
```
