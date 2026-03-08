# MVCC Snapshots

Snapshots are the read interface to the storage engine. They provide a
frozen, immutable view of the graph at a point in time. Readers acquire
a snapshot once and hold it for the duration of a read operation — they
never block writers, and writers never invalidate their data.

## What a Snapshot Is

```rust
pub struct Snapshot {
    /// Immutable reference to MemTable at the time of snapshot creation.
    /// The MemTable is never mutated after the snapshot is published.
    memtable: Arc<MemTable>,

    /// Ordered list of segment references (oldest to newest).
    segments: Arc<Vec<SegmentRef>>,
}
```

The `Snapshot` is wrapped in `Arc<Snapshot>` so multiple readers can share
the same snapshot cheaply. Acquiring a snapshot is `Arc::clone` — one atomic
increment on the reference count.

## Snapshot Manager

The `SnapshotManager` maintains the current published snapshot using
`arc-swap` for lock-free atomic updates:

```rust
pub struct SnapshotManager {
    current: ArcSwap<Snapshot>,
}

impl SnapshotManager {
    /// Called by readers — O(1), no lock.
    pub fn load(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }

    /// Called by the writer after every commit — O(1) atomic swap.
    pub fn publish(&self, snapshot: Snapshot) {
        self.current.store(Arc::new(snapshot));
    }
}
```

`arc-swap` guarantees that:
- A reader loading the snapshot always gets a consistent, complete view.
- There is no moment when the snapshot pointer is null or partially updated.
- No reader needs to hold a lock to read the snapshot.

## Snapshot Lifetime

```rust
// Acquiring: O(1), no lock, no allocation
let snap = engine.snapshot();   // = manager.load()

// Using: reads go through the snapshot — guaranteed consistent view
let entity = snap.get_entity(id);
let hosts = snap.entities_by_class(&EntityClass::new_unchecked("Host"));

// Releasing: O(1) when Arc reference count drops to zero
drop(snap);
```

When a snapshot is dropped, its reference count decrements. If this was the
last reference, the Arc frees the MemTable and segment list it held. Old
MemTable data can then be freed.

## Snapshot Query Methods

```rust
impl Snapshot {
    // Point lookups (O(1) MemTable, O(n) segment scan)
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity>;
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&Relationship>;

    // Index-accelerated scans
    pub fn entities_by_type(&self, t: &EntityType) -> Vec<&Entity>;
    pub fn entities_by_class(&self, c: &EntityClass) -> Vec<&Entity>;
    pub fn entities_by_source(&self, connector_id: &str) -> Vec<&Entity>;
    pub fn relationships_by_source(&self, connector_id: &str) -> Vec<&Relationship>;
    pub fn all_entities(&self) -> impl Iterator<Item = &Entity> + '_;

    // Adjacency
    pub fn outgoing(&self, id: EntityId) -> Vec<&Relationship>;
    pub fn incoming(&self, id: EntityId) -> Vec<&Relationship>;

    // Stats
    pub fn entity_count(&self) -> usize;
    pub fn relationship_count(&self) -> usize;
}
```

## Consistency Guarantees

**Read-your-writes:** Within the same `StorageEngine` instance, a read
snapshot acquired after a `write()` call will always see the written data.

**Snapshot isolation:** A snapshot acquired at time T will never see writes
committed after T, even if those writes happen on the same thread.

**No dirty reads:** A snapshot only contains data from committed
`WriteBatch`es — data written to the WAL but not yet applied to the MemTable
is not visible.

## Using Snapshots in async Code

`Snapshot` contains `Arc` references, making it `Send + Sync`. However,
`GraphReader<'snap>` borrows the snapshot and cannot cross `await` points.
The recommended pattern:

```rust
async fn my_handler(engine: Arc<Mutex<StorageEngine>>) -> Vec<Entity> {
    // Block: acquire lock, snapshot, compute, release
    let results = {
        let engine = engine.lock().unwrap();
        let snap = engine.snapshot();
        // All computation here — no await
        snap.entities_by_class(&EntityClass::new_unchecked("Host"))
            .into_iter()
            .filter(|e| !e._deleted)
            .cloned()
            .collect::<Vec<_>>()
        // snap dropped, lock released
    };

    // Now you can await freely with owned Vec<Entity>
    process_results(results).await
}
```

Alternatively, clone the `Arc<Snapshot>` and pass it to a spawn_blocking task
for CPU-intensive graph operations that would otherwise block the async runtime.
