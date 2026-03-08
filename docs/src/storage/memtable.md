# MemTable

The MemTable is the in-memory write buffer. Every write is applied here
immediately after the WAL fsync. Reads check the MemTable first, then
fall back to segment files.

## Structure

```rust
pub struct MemTable {
    // Primary storage
    entities:      BTreeMap<EntityId, Entity>,
    relationships: BTreeMap<RelationshipId, Relationship>,

    // Secondary indices (maintained in sync with primary)
    type_index:    HashMap<EntityType, Vec<EntityId>>,
    class_index:   HashMap<EntityClass, Vec<EntityId>>,
    source_index:  HashMap<CompactString, Vec<EntityId>>,   // connector_id → entities
    adjacency:     HashMap<EntityId, (Vec<RelationshipId>,  // outgoing
                                      Vec<RelationshipId>)>, // incoming
}
```

## Operations

### Upsert

```rust
pub fn upsert_entity(&mut self, entity: Entity) {
    let id = entity.id;
    let entity_type = entity._type.clone();
    let entity_class = entity._class.clone();
    let connector_id = entity.source.connector_id.clone();

    // Update primary store
    self.entities.insert(id, entity);

    // Update all secondary indices
    self.type_index.entry(entity_type).or_default().push(id);
    self.class_index.entry(entity_class).or_default().push(id);
    self.source_index.entry(connector_id).or_default().push(id);
    // adjacency is updated by upsert_relationship
}
```

### Tombstone (Soft Delete)

When the sync protocol determines that an entity was removed from a source:

```rust
pub fn delete_entity(&mut self, id: EntityId) {
    if let Some(entity) = self.entities.get_mut(&id) {
        entity._deleted = true;  // soft delete — remains for snapshot visibility
    }
}
```

Soft-deleted entities are invisible to queries (INV-S08) but remain in memory
until the next compaction cycle removes them.

### Adjacency Index

The adjacency index enables O(1) neighbor lookups:

```rust
pub fn upsert_relationship(&mut self, rel: Relationship) {
    let rel_id = rel.id;
    let from_id = rel.from_id;
    let to_id = rel.to_id;

    self.relationships.insert(rel_id, rel);

    // Both endpoints track this relationship
    self.adjacency.entry(from_id).or_default().0.push(rel_id); // outgoing
    self.adjacency.entry(to_id).or_default().1.push(rel_id);   // incoming
}
```

This is the index that makes graph traversal fast. Without it, every hop
would require a full scan of all relationships.

## Flush to Segment

When `memtable.approx_bytes() > config.memtable_flush_size` (default: 64MB),
`StorageEngine::maybe_flush()` runs:

```rust
fn maybe_flush(&mut self) -> Result<(), StoreError> {
    if self.memtable.approx_bytes() <= self.config.memtable_flush_size {
        return Ok(());
    }

    // Write current MemTable contents to a new .pxs segment
    let segment_path = self.next_segment_path();
    Segment::write(&segment_path, &self.memtable)?;

    // Drain the MemTable: clears entity/rel data, preserves adjacency index
    let new_segment = SegmentRef::open(segment_path)?;
    let drained = self.memtable.drain_to_flush();
    self.segments.push(new_segment);

    // Publish new snapshot pointing to empty MemTable + new segment
    self.publish_snapshot();
    Ok(())
}
```

The `drain_to_flush()` operation is carefully designed:
- Entity and relationship data moves to the segment file
- The adjacency index is preserved (rebuilt from segments during recovery)
- Secondary indices are cleared (rebuilt from segment scans as needed)

## Memory Accounting

```rust
pub fn approx_bytes(&self) -> usize {
    // Rough estimate: sum of entity and relationship sizes
    self.entities.values().map(|e| std::mem::size_of_val(e)).sum::<usize>()
        + self.relationships.values().map(|r| std::mem::size_of_val(r)).sum::<usize>()
}
```

This is an approximation — it counts stack sizes of the structs but not
heap-allocated strings. For memory budgeting, assume 2-4× the struct size
per entity due to `CompactString` heap allocations for long strings.

## Query Methods

The MemTable exposes index-accelerated query methods used by `Snapshot`:

```rust
// O(1) lookup
pub fn get_entity(&self, id: EntityId) -> Option<&Entity>;
pub fn get_relationship(&self, id: RelationshipId) -> Option<&Relationship>;

// Index-accelerated scans
pub fn entities_by_type(&self, t: &EntityType) -> Vec<&Entity>;
pub fn entities_by_class(&self, c: &EntityClass) -> Vec<&Entity>;
pub fn entities_by_source(&self, connector_id: &str) -> Vec<&Entity>;
pub fn all_entities(&self) -> impl Iterator<Item = &Entity>;

// Adjacency (O(1) for the lookup, O(degree) for iteration)
pub fn outgoing_relationships(&self, id: EntityId) -> Vec<&Relationship>;
pub fn incoming_relationships(&self, id: EntityId) -> Vec<&Relationship>;
```
