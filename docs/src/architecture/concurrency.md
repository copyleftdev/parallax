# Concurrency Model

Parallax uses a **single-writer, multi-reader** model with MVCC snapshots.
This is the most important architectural decision in the codebase.

## The Single-Writer Invariant

All mutations to the graph flow through a single serialized write path. At
any point in time, at most one `WriteBatch` is being committed.

**What this eliminates:**
- Write-write conflicts
- Deadlocks
- Lock ordering bugs
- Non-deterministic mutation ordering

**What this does not prevent:**
- Multiple concurrent readers (unlimited, lock-free)
- Multiple concurrent connector syncs (they queue at the ingest layer)

## MVCC Snapshots

```
                    ┌─────────────────────┐
                    │    Writer Path       │
                    │  (single, serial)    │
                    │                      │
  WriteBatch ──────►│  1. Validate         │
  WriteBatch ──────►│  2. WAL append+fsync │
  WriteBatch ──────►│  3. Apply to MemTable│
                    │  4. Update indices   │
                    │  5. Publish snapshot  │
                    └──────────┬───────────┘
                               │ ArcSwap::store (atomic)
                    ┌──────────▼───────────┐
                    │   SnapshotManager     │
                    │                      │
                    │  current: Arc<Snap>  │
                    └──┬─────┬─────┬───────┘
                       │     │     │  Arc::clone (one atomic increment)
                    ┌──▼─┐┌──▼─┐┌──▼──┐
                    │ R1 ││ R2 ││ R3  │  Reader threads (unlimited)
                    └────┘└────┘└─────┘
```

A snapshot is an immutable view of the graph at a point in time. Readers
acquire a snapshot with `Arc::clone` — one atomic increment, no lock. They
hold it as long as needed without blocking writes.

When the writer commits a new batch, it atomically publishes a new snapshot
via `arc-swap`. Existing readers keep their old snapshot until they drop it.
The old snapshot's memory is freed when all readers release their `Arc`.

## Shared State Inventory

| Shared State | Accessed By | Synchronization |
|---|---|---|
| `current_snapshot` | Writer (store), Readers (load) | `arc-swap` — lock-free atomic pointer |
| WAL file | Writer only | Single owner, no sync needed |
| MemTable | Writer (mut), Snapshot (immutable ref) | Writer publishes new snapshot; never mutated after publish |
| Segment inventory | Writer (during compaction), Readers (via snapshot) | Snapshots hold `Arc<Vec<SegmentRef>>`; writer builds new Vec |
| Metrics counters | Writer + Readers | `Relaxed` atomics (counters only) |

**Key insight:** The only truly shared mutable state is the snapshot pointer.
Everything else is either single-owner (WAL, MemTable before publish) or
immutable-after-publish (snapshots, segments).

## The SyncEngine Lock Pattern

The REST server shares the storage engine across concurrent HTTP handlers via
`Arc<Mutex<StorageEngine>>`:

```rust
pub struct AppState {
    pub engine: Arc<Mutex<StorageEngine>>,
    pub sync: SyncEngine,       // wraps the same Arc<Mutex<StorageEngine>>
    ...
}
```

The ingest path follows this protocol to minimize lock-hold time:

```rust
// 1. Lock briefly to take a snapshot for diff computation.
let (existing_entities, existing_rels) = {
    let engine = self.store.lock().expect("engine lock");
    let snap = engine.snapshot();
    validate_sync_batch(&entities, &relationships, &snap)?;
    let ents = snap.entities_by_source(connector_id)...collect();
    let rels = snap.relationships_by_source(connector_id)...collect();
    (ents, rels)  // snap dropped here, lock released
};

// 2. Compute diff without holding any lock (pure CPU work).
let mut batch = WriteBatch::new();
// ... diff logic ...

// 3. Lock briefly again to commit the batch.
if !batch.is_empty() {
    let mut engine = self.store.lock().expect("engine lock");
    engine.write(batch)?;
}
```

This pattern ensures the lock is held for microseconds, not milliseconds.
Multiple connectors can prepare their diffs concurrently; they only contend
at the write step.

## Async Compatibility

`GraphReader<'snap>` borrows its snapshot and cannot cross `await` points
because Rust's borrow checker prevents holding non-`Send` borrows across
awaits. The correct pattern for async handlers:

```rust
async fn query_handler(state: State<AppState>) -> Json<Value> {
    // Acquire engine lock, take snapshot, compute result, drop snapshot.
    // All synchronous — no await while holding the borrow.
    let result = {
        let engine = state.engine.lock().unwrap();
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        graph.find("host").collect::<Vec<_>>()
            .into_iter().map(entity_to_json).collect::<Vec<_>>()
        // snap drops here, lock released
    };
    Json(json!({ "entities": result }))
}
```

## Performance Characteristics

With this model on modern NVMe hardware:

| Operation | Throughput | Notes |
|---|---|---|
| Snapshot acquisition | ~1ns | `Arc::clone` = one atomic increment |
| Entity lookup (MemTable) | ≤1μs p99 | BTreeMap lookup |
| Entity lookup (Segment) | ≤100μs p99 | Linear scan; will improve with index in v0.2 |
| WAL write throughput | ≥500K ops/sec | With batching |
| Write lock contention | <5μs typical | Lock held only for MemTable update |
