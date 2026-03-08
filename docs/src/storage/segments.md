# Segments

Segment files are immutable, on-disk snapshots of MemTable contents. When
the MemTable grows beyond `memtable_flush_size`, it is flushed to a new
segment file and the in-memory data is freed.

## On-Disk Format

Segment files live in `{data_dir}/segments/` and are named
`seg-{index:08}.pxs` (e.g., `seg-00000000.pxs`).

```
┌──────────────┬────────────────────────────────────────┐
│  Header (5)  │         Payload (postcard)              │
├──────────────┼────────────────────────────────────────┤
│ magic (4)    │  SegmentData {                          │
│ version (1)  │    entities: Vec<Entity>,               │
│              │    relationships: Vec<Relationship>,    │
│              │  }                                      │
└──────────────┴────────────────────────────────────────┘
```

| Field | Size | Value |
|---|---|---|
| `magic` | 4 bytes | `0x50585347` — ASCII "PXSG" (Parallax SeGment) |
| `version` | 1 byte | `1` (current format version) |
| `payload` | N bytes | `postcard::to_allocvec(SegmentData { entities, relationships })` |

**INV-S07:** Segment files are immutable after creation. Compaction produces
new segment files — it never modifies existing ones.

## Writing a Segment

```rust
pub fn write(path: &Path, memtable: &MemTable) -> Result<(), StoreError> {
    let entities: Vec<Entity> = memtable.all_entities()
        .filter(|e| !e._deleted)
        .cloned()
        .collect();
    let relationships: Vec<Relationship> = memtable.all_relationships()
        .filter(|r| !r._deleted)
        .cloned()
        .collect();

    let data = SegmentData { entities, relationships };
    let payload = postcard::to_allocvec(&data)
        .map_err(|e| StoreError::Corruption(e.to_string()))?;

    let mut file = File::create(path)?;
    file.write_all(&SEGMENT_MAGIC)?;
    file.write_all(&[SEGMENT_VERSION])?;
    file.write_all(&payload)?;
    file.sync_all()?;
    Ok(())
}
```

Soft-deleted entities and relationships are excluded from the segment. This
is the mechanism by which deletes are physically reclaimed.

## Reading a Segment

```rust
pub fn open(path: &Path) -> Result<SegmentRef, StoreError> {
    let bytes = std::fs::read(path)?;

    // Verify magic
    if bytes.len() < 5 || &bytes[..4] != SEGMENT_MAGIC {
        return Err(StoreError::Corruption("bad segment magic".into()));
    }

    // Check version
    let version = bytes[4];
    if version != SEGMENT_VERSION {
        return Err(StoreError::Corruption(
            format!("unknown segment version {version}")
        ));
    }

    // Deserialize
    let data: SegmentData = postcard::from_bytes(&bytes[5..])?;
    Ok(SegmentRef { path: path.to_owned(), data })
}
```

## Snapshot Integration

A `Snapshot` holds an `Arc<Vec<SegmentRef>>` in addition to a MemTable
reference. When reading, the snapshot:

1. Checks the MemTable first (most recent data)
2. Scans segments in reverse order (newest to oldest)
3. Returns the first match found

```rust
impl Snapshot {
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity> {
        // MemTable first
        if let Some(e) = self.memtable.get_entity(id) {
            return if e._deleted { None } else { Some(e) };
        }
        // Then segments (newest first)
        for segment in self.segments.iter().rev() {
            if let Some(e) = segment.get_entity(id) {
                return if e._deleted { None } else { Some(e) };
            }
        }
        None
    }
}
```

## Compaction (v0.2)

In v0.1, segments are never compacted. They accumulate until you delete the
data directory. Background segment compaction is planned for v0.2:

1. Merge multiple small segments into fewer large segments
2. Remove soft-deleted entities and stale versions
3. Build a sparse index on segment boundaries for faster lookups

The current linear scan approach is correct for graphs up to ~1M entities.
At larger scale, the segment index becomes important for performance.
