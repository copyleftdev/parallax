# Sync Protocol

The sync protocol handles the transition from collected data to committed
graph state. It diffs the new batch against the existing data and commits
the delta atomically.

## The Diff Algorithm

For each connector sync, the engine computes a diff between:
- **Emitted:** entities and relationships from the current connector run
- **Existing:** entities and relationships already in the graph from the same connector

```
Emitted entities:  {A, B, C}
Existing entities: {A, B, D}  ← D was in the last sync but not emitted this time

Delta:
  Upsert A (unchanged if properties match)
  Upsert B (unchanged if properties match)
  Upsert C (new)
  Delete D (not seen in this sync)
```

## Source Scope (INV-C02)

The diff is scoped to the connector's source. Connector B's sync never
deletes entities created by connector A:

```
Graph state:
  Host web-01 (source: aws-connector)
  Host web-02 (source: aws-connector)
  User alice   (source: okta-connector)

Okta sync: emits [alice-v2]
Result:
  Host web-01 (source: aws-connector) — UNCHANGED
  Host web-02 (source: aws-connector) — UNCHANGED
  User alice   (source: okta-connector) — UPDATED
```

## Atomic Commit (INV-C01)

The entire delta (creates + updates + deletes) is committed as a single
`WriteBatch`. Either all changes land or none do:

```rust
// SyncEngine::commit_sync internals:
let mut batch = WriteBatch::new();

for entity in &entities {
    match existing.find(|e| e.id == entity.id) {
        None => batch.upsert_entity(entity.clone()),           // create
        Some(ex) if ex.properties != entity.properties =>
            batch.upsert_entity(entity.clone()),               // update
        Some(_) => {}                                          // unchanged
    }
}

for existing in &existing_entities {
    if !seen_ids.contains(&existing.id) {
        batch.delete_entity(existing.id);                      // delete
    }
}

// Atomic commit
engine.write(batch)?;
```

## Referential Integrity (INV-C04)

Before committing, the ingest layer validates that every relationship's
endpoints exist. The check considers:
1. Entities in the current batch (being committed now)
2. Entities already in the graph from any connector

```rust
// validate_sync_batch checks each relationship:
let available: HashSet<EntityId> = batch_entities.iter().map(|e| e.id).collect::<_>()
    .union(&snapshot_entities.iter().map(|e| e.id).collect::<_>())
    .copied()
    .collect();

for rel in &relationships {
    if !available.contains(&rel.from_id) {
        return Err(SyncError::DanglingRelationship { ... });
    }
    if !available.contains(&rel.to_id) {
        return Err(SyncError::DanglingRelationship { ... });
    }
}
```

A sync batch with a dangling relationship returns `SyncError::DanglingRelationship`
and no data is committed.

## SyncResult

```rust
pub struct SyncResult {
    pub sync_id: String,
    pub stats: SyncStats,
}

pub struct SyncStats {
    pub entities_created: u64,
    pub entities_updated: u64,
    pub entities_unchanged: u64,
    pub entities_deleted: u64,
    pub relationships_created: u64,
    pub relationships_updated: u64,
    pub relationships_unchanged: u64,
    pub relationships_deleted: u64,
}
```

## Two Commit Modes

### `commit_sync_exclusive` — Exclusive Engine Access

Used when you hold `&mut StorageEngine`. Best for single-threaded usage
or CLI tools:

```rust
let result = commit_sync_exclusive(
    &mut engine,
    &output.connector_id,
    &output.sync_id,
    output.entities,
    output.relationships,
)?;
```

### `SyncEngine::commit_sync` — Shared Engine Access

Used in server mode where multiple connectors share one engine via
`Arc<Mutex<StorageEngine>>`:

```rust
let sync_engine = SyncEngine::new(Arc::clone(&engine));
let result = sync_engine.commit_sync(
    &output.connector_id,
    &output.sync_id,
    output.entities,
    output.relationships,
)?;
```

`SyncEngine::commit_sync` holds the engine lock only during the brief write
step, not during diff computation. Multiple connectors can diff concurrently;
they only serialize at the write step.

## Idempotency

Running the same sync twice with identical data is safe and efficient:

```
First run:   entities_created = 5, entities_deleted = 0
Second run:  entities_unchanged = 5, entities_created = 0, entities_deleted = 0
```

The diff detects that nothing changed and the `WriteBatch` is empty,
so no WAL write occurs.
