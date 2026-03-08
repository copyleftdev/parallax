//! MVCC snapshot manager.
//!
//! A `Snapshot` is an immutable, consistent view of the graph at a point
//! in logical time. Readers hold an `Arc<Snapshot>` — acquiring one is a
//! single lock-free atomic load (via `arc-swap`).
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.7
//!
//! INV-S03: Snapshots are monotonically ordered by version number.
//! INV-S04: A segment file is never deleted while any live snapshot references it.
//! INV-S08: Readers never block writers. Writers never wait for readers.

use std::collections::HashSet;
use std::sync::Arc;

use arc_swap::ArcSwap;

use parallax_core::{
    entity::{Entity, EntityId, EntityType},
    relationship::{Relationship, RelationshipId},
    timestamp::Timestamp,
};

use crate::index::AdjEntry;
use crate::memtable::MemTable;
use crate::segment::SegmentRef;

/// A consistent, frozen view of the storage engine at a specific version.
///
/// Readers acquire a snapshot by calling `SnapshotManager::snapshot()`,
/// which returns an `arc_swap::Guard<Arc<Snapshot>>`. No locks are held.
///
/// The snapshot holds `Arc<MemTable>` and `Arc<Vec<SegmentRef>>`. These
/// are immutable after publication. Dropping the snapshot decrements the
/// Arc reference counts, eventually releasing memory (INV-S04).
pub struct Snapshot {
    /// Monotonically increasing version number.
    pub version: u64,
    /// Logical time of this snapshot (HLC).
    pub timestamp: Timestamp,
    memtable: Arc<MemTable>,
    /// Segment list. Arc keeps segment files alive for INV-S04.
    segments: Arc<Vec<SegmentRef>>,
}

impl Snapshot {
    /// Create a new snapshot from the writer's current state.
    pub(crate) fn new(
        version: u64,
        memtable: Arc<MemTable>,
        segments: Arc<Vec<SegmentRef>>,
    ) -> Self {
        Snapshot {
            version,
            timestamp: Timestamp::now(),
            memtable,
            segments,
        }
    }

    /// Look up a live entity by ID.
    ///
    /// Checks MemTable first; falls through to segments in newest-first order.
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity> {
        if let Some(e) = self.memtable.get_entity(id) {
            return Some(e);
        }
        for seg in self.segments.iter().rev() {
            if let Some(e) = seg.get_entity(id) {
                return Some(e);
            }
        }
        None
    }

    /// Look up a live relationship by ID.
    ///
    /// Checks MemTable first; falls through to segments in newest-first order.
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        if let Some(r) = self.memtable.get_relationship(id) {
            return Some(r);
        }
        for seg in self.segments.iter().rev() {
            if let Some(r) = seg.get_relationship(id) {
                return Some(r);
            }
        }
        None
    }

    /// Return all live entities of the given type (MemTable + all segments, deduplicated).
    ///
    /// MemTable version is authoritative. Entities present in multiple segments
    /// (due to repeated upserts across flushes) are returned once, newest first.
    pub fn entities_of_type(&self, entity_type: &EntityType) -> Vec<&Entity> {
        let mt = self.memtable.entities_of_type(entity_type.as_str());
        let mut seen: HashSet<EntityId> = mt.iter().map(|e| e.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for e in seg.entities_of_type(entity_type) {
                if seen.insert(e.id) {
                    result.push(e);
                }
            }
        }
        result
    }

    /// Return all live entities of the given class (MemTable + all segments, deduplicated).
    pub fn entities_of_class(&self, class: &str) -> Vec<&Entity> {
        let mt = self.memtable.entities_of_class(class);
        let mut seen: HashSet<EntityId> = mt.iter().map(|e| e.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for e in seg.entities_of_class(class) {
                if seen.insert(e.id) {
                    result.push(e);
                }
            }
        }
        result
    }

    /// Get the adjacency list for an entity (MemTable only).
    ///
    /// Traversal works correctly for flushed entities because the MemTable
    /// retains the adjacency index even after entity data is flushed to segments.
    pub fn adjacency(&self, id: EntityId) -> &[AdjEntry] {
        self.memtable.adjacency(id)
    }

    /// Number of live entities visible in this snapshot (unique across MemTable + all segments).
    ///
    /// MemTable is authoritative (newest version). An entity present in both a
    /// segment and the MemTable — or in multiple segments — is counted once.
    pub fn entity_count(&self) -> usize {
        let mut seen: HashSet<EntityId> =
            self.memtable.all_entities().iter().map(|e| e.id).collect();
        for seg in self.segments.iter().rev() {
            for e in seg.all_entities() {
                seen.insert(e.id);
            }
        }
        seen.len()
    }

    /// Number of live relationships visible in this snapshot (unique across MemTable + all segments).
    pub fn relationship_count(&self) -> usize {
        let mut seen: HashSet<RelationshipId> = self
            .memtable
            .all_relationships()
            .iter()
            .map(|r| r.id)
            .collect();
        for seg in self.segments.iter().rev() {
            for r in seg.all_relationships() {
                seen.insert(r.id);
            }
        }
        seen.len()
    }

    /// Return all live entities (MemTable + all segments, deduplicated by ID).
    ///
    /// MemTable version is authoritative when an entity exists in multiple layers.
    /// Segments are iterated newest-first so the newest segment version is preferred
    /// over older segments when the MemTable has no entry.
    pub fn all_entities(&self) -> Vec<&Entity> {
        let mt = self.memtable.all_entities();
        let mut seen: HashSet<EntityId> = mt.iter().map(|e| e.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for e in seg.all_entities() {
                if seen.insert(e.id) {
                    result.push(e);
                }
            }
        }
        result
    }

    /// Return all live relationships (MemTable + all segments, deduplicated by ID).
    pub fn all_relationships(&self) -> Vec<&Relationship> {
        let mt = self.memtable.all_relationships();
        let mut seen: HashSet<RelationshipId> = mt.iter().map(|r| r.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for r in seg.all_relationships() {
                if seen.insert(r.id) {
                    result.push(r);
                }
            }
        }
        result
    }

    /// Return all live entities from a specific connector (source-scoped diff, deduplicated).
    pub fn entities_by_source(&self, connector_id: &str) -> Vec<&Entity> {
        let mt = self.memtable.entities_by_source(connector_id);
        let mut seen: HashSet<EntityId> = mt.iter().map(|e| e.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for e in seg.entities_by_source(connector_id) {
                if seen.insert(e.id) {
                    result.push(e);
                }
            }
        }
        result
    }

    /// Return all live relationships from a specific connector (source-scoped diff, deduplicated).
    pub fn relationships_by_source(&self, connector_id: &str) -> Vec<&Relationship> {
        let mt = self.memtable.relationships_by_source(connector_id);
        let mut seen: HashSet<RelationshipId> = mt.iter().map(|r| r.id).collect();
        let mut result = mt;
        for seg in self.segments.iter().rev() {
            for r in seg.relationships_by_source(connector_id) {
                if seen.insert(r.id) {
                    result.push(r);
                }
            }
        }
        result
    }
}

/// Atomic snapshot publisher.
///
/// Holds the current snapshot behind an `ArcSwap<Snapshot>`. The writer
/// calls `publish()` to atomically replace it. Readers call `snapshot()`
/// which is lock-free and wait-free.
///
/// # Concurrency
///
/// - `snapshot()` → one atomic load + Arc clone. Never blocks.
/// - `publish()` → one atomic store. Never blocks readers.
pub struct SnapshotManager {
    current: ArcSwap<Snapshot>,
}

impl SnapshotManager {
    /// Create a new manager with the given initial snapshot.
    pub fn new(initial: Snapshot) -> Self {
        SnapshotManager {
            current: ArcSwap::from_pointee(initial),
        }
    }

    /// Acquire the current snapshot. Lock-free, wait-free.
    ///
    /// The returned `Guard` keeps the snapshot alive until dropped.
    pub fn snapshot(&self) -> arc_swap::Guard<Arc<Snapshot>> {
        self.current.load()
    }

    /// Publish a new snapshot. Called only by the writer thread.
    ///
    /// After this returns, all new calls to `snapshot()` will return the
    /// new version. Existing guards continue to hold the old version until
    /// they are dropped.
    pub fn publish(&self, snapshot: Snapshot) {
        self.current.store(Arc::new(snapshot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snapshot(version: u64) -> Snapshot {
        Snapshot::new(version, Arc::new(MemTable::new()), Arc::new(Vec::new()))
    }

    #[test]
    fn snapshot_version_is_correct() {
        let snap = empty_snapshot(42);
        assert_eq!(snap.version, 42);
    }

    #[test]
    fn manager_publish_updates_version() {
        let mgr = SnapshotManager::new(empty_snapshot(1));
        assert_eq!(mgr.snapshot().version, 1);

        mgr.publish(empty_snapshot(2));
        assert_eq!(mgr.snapshot().version, 2);
    }

    #[test]
    fn held_guard_retains_old_snapshot() {
        let mgr = SnapshotManager::new(empty_snapshot(1));
        let old_guard = mgr.snapshot(); // holds version 1
        mgr.publish(empty_snapshot(2));

        // Old guard still sees version 1.
        assert_eq!(old_guard.version, 1);
        // New load sees version 2.
        assert_eq!(mgr.snapshot().version, 2);
    }
}
