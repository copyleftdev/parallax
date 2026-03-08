# Storage Engine Overview

The `parallax-store` crate is the durable foundation of the entire system.
It provides:

1. **Durability** — entities and relationships are written to a WAL before
   being applied to memory.
2. **Point lookups** — retrieve an entity by ID in ≤1μs from MemTable.
3. **MVCC snapshots** — immutable, frozen views of the graph that readers
   hold without blocking writes.
4. **Compaction** — (v0.2) background reclamation of space from deleted
   versions.

The storage engine does **not** understand graph semantics. It stores keyed
records. The graph engine (`parallax-graph`) builds traversal and adjacency
on top of the storage snapshot interface.

## Architecture

```
StorageEngine
├── WriteAheadLog (WAL)
│   └── wal-00000000.pxw, wal-00000001.pxw, ...
│
├── MemTable (in-memory)
│   ├── entities: BTreeMap<EntityId, Entity>
│   ├── relationships: BTreeMap<RelationshipId, Relationship>
│   ├── type_index: HashMap<EntityType, Vec<EntityId>>
│   ├── class_index: HashMap<EntityClass, Vec<EntityId>>
│   ├── source_index: HashMap<ConnectorId, Vec<EntityId>>
│   └── adjacency: HashMap<EntityId, (Vec<RelId>, Vec<RelId>)>
│
├── Segments (on-disk, immutable)
│   └── seg-00000000.pxs, seg-00000001.pxs, ...
│
└── SnapshotManager
    └── current: ArcSwap<Snapshot>
        └── Snapshot { memtable_ref, segments: Arc<Vec<SegmentRef>> }
```

## Write Path

Every mutation follows this sequence:

1. Build a `WriteBatch` (set of upsert/delete operations)
2. Serialize and append to WAL with CRC32C checksum
3. `fsync()` — durability point; crash here loses nothing already committed
4. Apply batch to MemTable
5. Publish new snapshot via `ArcSwap::store`

```rust
let mut engine = StorageEngine::open(StoreConfig::new("/var/lib/parallax"))?;

let mut batch = WriteBatch::new();
batch.upsert_entity(entity);
batch.upsert_relationship(rel);

engine.write(batch)?;  // Steps 1-5 above
```

## Read Path

Reads always go through a `Snapshot`:

```rust
let snap = engine.snapshot();  // Arc::clone — O(1), no lock
// Entity lookup: MemTable first, then segment scan
if let Some(entity) = snap.get_entity(entity_id) {
    println!("{}", entity.display_name);
}
// snap dropped here; frees the Arc
```

## MemTable Flush

When the MemTable exceeds `memtable_flush_size` (default: 64MB), it is flushed
to an immutable segment file:

1. Serialize all entities and relationships to a `.pxs` segment file
2. The new snapshot points to the fresh (empty) MemTable + the new segment
3. The old MemTable data is freed

The adjacency index is preserved through flushes.

## Crash Recovery

On `StorageEngine::open()`, if WAL segments exist:

1. Replay WAL entries in order, verifying CRC32C on each
2. Stop at the first corrupt entry (INV-S05)
3. Apply all valid entries to rebuild the MemTable
4. Publish the recovered snapshot

Recovery is the only code path that rebuilds MemTable from WAL. Normal
operation never re-reads the WAL.

## Storage Engine API

```rust
// Open or create a storage engine
let engine = StorageEngine::open(StoreConfig::new(data_dir))?;

// Write a batch (atomic, durable)
engine.write(batch)?;

// Get an MVCC snapshot (O(1))
let snap = engine.snapshot();

// Access entity counts (for stats/metrics)
let metrics = engine.metrics().snapshot();
```

See [StorageEngine API](./engine-api.md) for the full interface.
