//! On-disk immutable segment files (.pxs — Parallax Segment).
//!
//! Segments are the SSTable equivalent for Parallax: immutable, sorted,
//! block-compressed files flushed from the MemTable.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.6
//!
//! # File Format (.pxs)
//!
//! ```text
//! [4 bytes] Magic: "PXSG"
//! [1 byte]  Version: 1
//! [N bytes] postcard-encoded SegmentData { entities, relationships }
//! ```
//!
//! v0.2: Entities and relationships are stored sorted by ID. A parallel
//! `Vec<Id>` index enables O(log n) binary-search point lookups. Scan
//! operations (filter by type/class) remain O(n) — sequential access is
//! cache-friendly and the sorted order makes compaction merges linear.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use memmap2::Mmap;
use serde::{Deserialize, Serialize};

use parallax_core::{
    entity::{Entity, EntityId, EntityType},
    relationship::{Relationship, RelationshipId},
};

use crate::error::StoreError;

/// Magic bytes written at the start of every .pxs file.
const MAGIC: &[u8; 4] = b"PXSG";
/// Current segment file format version.
const VERSION: u8 = 1;

/// On-disk payload: entities + relationships serialized with postcard.
#[derive(Serialize, Deserialize)]
struct SegmentData {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
}

/// A reference to an immutable on-disk segment file.
///
/// Segments are opened once and their data is deserialized into memory-resident
/// Vecs. The `mmap` keeps the file descriptor alive for the duration of all
/// snapshots that reference this segment (INV-S04).
///
/// Entities and relationships are sorted by ID (ascending). A parallel index
/// (`entity_index`, `relationship_index`) enables O(log n) point lookups via
/// binary search. Scan operations remain O(n) over the sorted Vec.
#[derive(Clone)]
pub struct SegmentRef {
    /// Path to the .pxs file on disk.
    pub path: PathBuf,
    /// Level in the LSM tree (0 = newest, flushed from MemTable).
    pub level: u8,
    /// Number of entity records in this segment.
    pub record_count: u64,
    /// Memory-mapped file handle — kept alive for INV-S04.
    ///
    /// # SAFETY
    ///
    /// The mmap is created once at segment open time and is never written to
    /// after creation. Segments are immutable. `Arc` ensures the mapping
    /// outlives all snapshots referencing this segment.
    pub mmap: Arc<Mmap>,
    /// Deserialized entities, sorted by `Entity.id` ascending.
    entities: Arc<Vec<Entity>>,
    /// Entity IDs in the same order as `entities` — binary search target.
    entity_index: Arc<Vec<EntityId>>,
    /// Deserialized relationships, sorted by `Relationship.id` ascending.
    relationships: Arc<Vec<Relationship>>,
    /// Relationship IDs in the same order as `relationships`.
    relationship_index: Arc<Vec<RelationshipId>>,
}

/// Sort entities by ID and build a parallel ID index for binary search.
fn build_entity_index(entities: &mut [Entity]) -> Vec<EntityId> {
    entities.sort_unstable_by_key(|e| e.id);
    entities.iter().map(|e| e.id).collect()
}

/// Sort relationships by ID and build a parallel ID index for binary search.
fn build_relationship_index(relationships: &mut [Relationship]) -> Vec<RelationshipId> {
    relationships.sort_unstable_by_key(|r| r.id);
    relationships.iter().map(|r| r.id).collect()
}

impl std::fmt::Debug for SegmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentRef")
            .field("path", &self.path)
            .field("level", &self.level)
            .field("record_count", &self.record_count)
            .finish()
    }
}

impl SegmentRef {
    /// Write a new segment file at `path` from the given entities and relationships.
    ///
    /// After writing, opens and returns a `SegmentRef` ready for reads.
    pub fn write(
        path: &Path,
        level: u8,
        entities: Vec<Entity>,
        relationships: Vec<Relationship>,
    ) -> Result<Self, StoreError> {
        // Sort by ID so binary search works on both write and re-open.
        let mut entities = entities;
        let mut relationships = relationships;
        let entity_index = build_entity_index(&mut entities);
        let relationship_index = build_relationship_index(&mut relationships);
        let record_count = entities.len() as u64;

        // Serialize with postcard (sorted order is now on-disk).
        let payload = postcard::to_allocvec(&SegmentData {
            entities: entities.clone(),
            relationships: relationships.clone(),
        })
        .map_err(|e| StoreError::Corruption(format!("segment serialize: {e}")))?;

        // Write: magic + version + postcard payload.
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let mut file = std::fs::File::create(path)?;
        file.write_all(MAGIC)?;
        file.write_all(&[VERSION])?;
        file.write_all(&payload)?;
        file.sync_all()?;
        drop(file);

        // Open mmap.
        let file = std::fs::File::open(path)?;
        // SAFETY: file is newly created and will not be mutated; we hold an Arc.
        let mmap = unsafe { Mmap::map(&file) }?;

        Ok(SegmentRef {
            path: path.to_path_buf(),
            level,
            record_count,
            mmap: Arc::new(mmap),
            entities: Arc::new(entities),
            entity_index: Arc::new(entity_index),
            relationships: Arc::new(relationships),
            relationship_index: Arc::new(relationship_index),
        })
    }

    /// Open an existing segment file at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let file = std::fs::File::open(path)?;
        // SAFETY: file is opened read-only; no external writes once a segment is created.
        let mmap = unsafe { Mmap::map(&file) }?;

        // Validate magic + version.
        if mmap.len() < 5 {
            return Err(StoreError::Corruption(format!(
                "segment file too short: {}",
                path.display()
            )));
        }
        if &mmap[..4] != MAGIC {
            return Err(StoreError::Corruption(format!(
                "bad magic in segment: {}",
                path.display()
            )));
        }
        if mmap[4] != VERSION {
            return Err(StoreError::Corruption(format!(
                "unknown segment version {} in: {}",
                mmap[4],
                path.display()
            )));
        }

        // Deserialize payload.
        let data: SegmentData = postcard::from_bytes(&mmap[5..])
            .map_err(|e| StoreError::Corruption(format!("segment deserialize: {e}")))?;

        // Sort and build index — also handles segments written by v0.1 (unsorted).
        let mut entities = data.entities;
        let mut relationships = data.relationships;
        let entity_index = build_entity_index(&mut entities);
        let relationship_index = build_relationship_index(&mut relationships);
        let record_count = entities.len() as u64;

        Ok(SegmentRef {
            path: path.to_path_buf(),
            level: 0, // recovered from file name in future; defaulting to 0
            record_count,
            mmap: Arc::new(mmap),
            entities: Arc::new(entities),
            entity_index: Arc::new(entity_index),
            relationships: Arc::new(relationships),
            relationship_index: Arc::new(relationship_index),
        })
    }

    /// Look up an entity by ID. O(log n) via binary search on the sorted index.
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity> {
        match self.entity_index.binary_search(&id) {
            Ok(pos) => {
                let e = &self.entities[pos];
                if e.is_tombstone() { None } else { Some(e) }
            }
            Err(_) => None,
        }
    }

    /// Look up a relationship by ID. O(log n) via binary search on the sorted index.
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        match self.relationship_index.binary_search(&id) {
            Ok(pos) => {
                let r = &self.relationships[pos];
                if r.is_tombstone() { None } else { Some(r) }
            }
            Err(_) => None,
        }
    }

    /// Return all live entities of the given type.
    pub fn entities_of_type(&self, entity_type: &EntityType) -> Vec<&Entity> {
        self.entities
            .iter()
            .filter(|e| !e.is_tombstone() && &e._type == entity_type)
            .collect()
    }

    /// Return all live entities of the given class.
    pub fn entities_of_class(&self, class: &str) -> Vec<&Entity> {
        self.entities
            .iter()
            .filter(|e| !e.is_tombstone() && e._class.as_str() == class)
            .collect()
    }

    /// Return all live entities from a specific connector.
    pub fn entities_by_source(&self, connector_id: &str) -> Vec<&Entity> {
        self.entities
            .iter()
            .filter(|e| !e.is_tombstone() && e.source.connector_id.as_str() == connector_id)
            .collect()
    }

    /// Return all live relationships from a specific connector.
    pub fn relationships_by_source(&self, connector_id: &str) -> Vec<&Relationship> {
        self.relationships
            .iter()
            .filter(|r| !r.is_tombstone() && r.source.connector_id.as_str() == connector_id)
            .collect()
    }

    /// All live entities in this segment.
    pub fn all_entities(&self) -> Vec<&Entity> {
        self.entities.iter().filter(|e| !e.is_tombstone()).collect()
    }

    /// All live relationships in this segment.
    pub fn all_relationships(&self) -> Vec<&Relationship> {
        self.relationships.iter().filter(|r| !r.is_tombstone()).collect()
    }

    /// Entity count (live only).
    pub fn entity_count(&self) -> usize {
        self.entities.iter().filter(|e| !e.is_tombstone()).count()
    }

    /// Relationship count (live only).
    pub fn relationship_count(&self) -> usize {
        self.relationships.iter().filter(|r| !r.is_tombstone()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_str::CompactString;
    use parallax_core::{
        entity::{EntityClass, EntityType},
        relationship::RelationshipClass,
        source::SourceTag,
        timestamp::Timestamp,
    };
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn tmp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn make_entity(id: EntityId, typ: &str, class: &str) -> Entity {
        Entity {
            id,
            _type: EntityType::new_unchecked(typ),
            _class: EntityClass::new_unchecked(class),
            display_name: CompactString::default(),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    fn make_rel(id: RelationshipId, from: EntityId, to: EntityId, class: &str) -> Relationship {
        Relationship {
            id,
            from_id: from,
            to_id: to,
            _class: RelationshipClass::new_unchecked(class),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    #[test]
    fn write_and_read_segment() {
        let dir = tmp_dir();
        let path = dir.path().join("test.pxs");

        let id = EntityId::derive("acc", "host", "h1");
        let entity = make_entity(id, "host", "Host");

        let seg = SegmentRef::write(&path, 0, vec![entity.clone()], vec![]).unwrap();
        assert_eq!(seg.record_count, 1);
        assert!(seg.get_entity(id).is_some());
        assert!(seg.get_entity(EntityId::derive("acc", "host", "ghost")).is_none());
    }

    #[test]
    fn open_existing_segment() {
        let dir = tmp_dir();
        let path = dir.path().join("test.pxs");

        let id = EntityId::derive("acc", "host", "h1");
        let entity = make_entity(id, "host", "Host");

        SegmentRef::write(&path, 0, vec![entity], vec![]).unwrap();

        // Re-open from disk.
        let seg = SegmentRef::open(&path).unwrap();
        assert!(seg.get_entity(id).is_some());
    }

    #[test]
    fn segment_with_relationships() {
        let dir = tmp_dir();
        let path = dir.path().join("test.pxs");

        let a = EntityId::derive("acc", "host", "h1");
        let b = EntityId::derive("acc", "host", "h2");
        let rel_id = RelationshipId::derive("acc", "host", "h1", "CONNECTS", "host", "h2");

        let rel = make_rel(rel_id, a, b, "CONNECTS");
        let seg = SegmentRef::write(&path, 0, vec![], vec![rel]).unwrap();
        assert!(seg.get_relationship(rel_id).is_some());
    }

    #[test]
    fn bad_magic_returns_error() {
        let dir = tmp_dir();
        let path = dir.path().join("bad.pxs");
        std::fs::write(&path, b"XXXX\x01garbage").unwrap();
        assert!(SegmentRef::open(&path).is_err());
    }
}
