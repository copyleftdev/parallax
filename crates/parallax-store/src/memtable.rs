//! In-memory table with integrated secondary indices.
//!
//! The MemTable is the mutable, in-memory representation of the most recent
//! write operations. It is owned exclusively by the writer thread during
//! mutation. When a snapshot is published, an immutable `Arc<MemTable>` is
//! cloned and handed to readers.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.5
//!
//! INV-S07: Only the writer thread mutates MemTable.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use parallax_core::{
    entity::{Entity, EntityId},
    relationship::{Direction, Relationship, RelationshipId},
};

use crate::index::{AdjEntry, AdjList};
use crate::write_batch::{WriteBatch, WriteOp};

/// In-memory sorted store of entities and relationships, with secondary indices.
///
/// Owned by the writer thread during writes. Readers receive an immutable
/// `Arc<MemTable>` snapshot that is never modified after publication.
#[derive(Debug, Clone, Default)]
pub struct MemTable {
    /// Entities keyed by EntityId. Includes tombstones.
    entities: BTreeMap<EntityId, Entity>,
    /// Relationships keyed by RelationshipId. Includes tombstones.
    relationships: BTreeMap<RelationshipId, Relationship>,
    /// EntityType string → set of EntityIds with that type.
    by_type: HashMap<String, BTreeSet<EntityId>>,
    /// EntityClass string → set of EntityIds with that class.
    by_class: HashMap<String, BTreeSet<EntityId>>,
    /// Connector ID → set of EntityIds from that source (INV-C01/C02).
    by_source: HashMap<String, BTreeSet<EntityId>>,
    /// Connector ID → set of RelationshipIds from that source.
    by_source_rel: HashMap<String, BTreeSet<RelationshipId>>,
    /// EntityId → adjacency list (both outgoing and incoming edges).
    adjacency: HashMap<EntityId, AdjList>,
    /// Approximate heap usage in bytes (for flush threshold decisions).
    approx_bytes: usize,
}

impl MemTable {
    /// Create an empty MemTable.
    pub fn new() -> Self {
        MemTable::default()
    }

    /// Apply all operations in a write batch. Must only be called from the
    /// writer thread.
    pub fn apply(&mut self, batch: &WriteBatch) {
        for op in &batch.operations {
            match op {
                WriteOp::UpsertEntity(entity) => self.apply_upsert_entity(entity),
                WriteOp::DeleteEntity(id) => self.apply_delete_entity(*id),
                WriteOp::UpsertRelationship(rel) => self.apply_upsert_rel(rel),
                WriteOp::DeleteRelationship(id) => self.apply_delete_rel(*id),
            }
        }
    }

    /// Look up a live entity by ID. Returns `None` for tombstones.
    pub fn get_entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id).filter(|e| !e.is_tombstone())
    }

    /// Look up a live relationship by ID. Returns `None` for tombstones.
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        self.relationships.get(&id).filter(|r| !r.is_tombstone())
    }

    /// Return all live entities of a specific type.
    pub fn entities_of_type(&self, entity_type: &str) -> Vec<&Entity> {
        self.by_type
            .get(entity_type)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entities.get(id).filter(|e| !e.is_tombstone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return all live entities of a specific class.
    pub fn entities_of_class(&self, entity_class: &str) -> Vec<&Entity> {
        self.by_class
            .get(entity_class)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entities.get(id).filter(|e| !e.is_tombstone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the adjacency list for an entity (both outgoing and incoming edges).
    pub fn adjacency(&self, id: EntityId) -> &[AdjEntry] {
        self.adjacency.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Return all live entities (full scan, excludes tombstones).
    pub fn all_entities(&self) -> Vec<&Entity> {
        self.entities
            .values()
            .filter(|e| !e.is_tombstone())
            .collect()
    }

    /// Return all live relationships (full scan, excludes tombstones).
    pub fn all_relationships(&self) -> Vec<&Relationship> {
        self.relationships
            .values()
            .filter(|r| !r.is_tombstone())
            .collect()
    }

    /// Approximate heap usage in bytes.
    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    /// Number of live entities (excludes tombstones).
    pub fn entity_count(&self) -> usize {
        self.entities.values().filter(|e| !e.is_tombstone()).count()
    }

    /// Number of live relationships (excludes tombstones).
    pub fn relationship_count(&self) -> usize {
        self.relationships
            .values()
            .filter(|r| !r.is_tombstone())
            .count()
    }

    /// Return all live entities from a specific connector (source-scoped).
    pub fn entities_by_source(&self, connector_id: &str) -> Vec<&Entity> {
        self.by_source
            .get(connector_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entities.get(id).filter(|e| !e.is_tombstone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return all live relationships from a specific connector (source-scoped).
    pub fn relationships_by_source(&self, connector_id: &str) -> Vec<&Relationship> {
        self.by_source_rel
            .get(connector_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.relationships.get(id).filter(|r| !r.is_tombstone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Extract all live entities and relationships for flushing to a segment.
    ///
    /// Clears entity/relationship data from this MemTable while preserving the
    /// adjacency index so that traversals remain correct after the flush.
    /// Called only from the writer thread (INV-S07).
    pub fn drain_to_flush(&mut self) -> (Vec<Entity>, Vec<Relationship>) {
        let entities: Vec<Entity> = self
            .entities
            .values()
            .filter(|e| !e.is_tombstone())
            .cloned()
            .collect();
        let relationships: Vec<Relationship> = self
            .relationships
            .values()
            .filter(|r| !r.is_tombstone())
            .cloned()
            .collect();

        // Clear entity/relationship payload maps and their secondary indices.
        // The adjacency map is preserved so traversal still works.
        self.entities.clear();
        self.relationships.clear();
        self.by_type.clear();
        self.by_class.clear();
        self.by_source.clear();
        self.by_source_rel.clear();
        // Approximate remaining size: adjacency entries only.
        self.approx_bytes = self.adjacency.values().map(|l| l.len() * 33).sum();

        (entities, relationships)
    }

    /// Clone this MemTable into an immutable Arc for snapshot use.
    ///
    /// This is O(n) — called once per published snapshot. Acceptable for v0.1.
    pub fn as_arc_snapshot(&self) -> Arc<MemTable> {
        Arc::new(self.clone())
    }

    // --- Private write helpers ---

    fn apply_upsert_entity(&mut self, entity: &Entity) {
        // Remove stale type/class/source index entries if the entity existed before.
        if let Some(old) = self.entities.get(&entity.id) {
            if !old.is_tombstone() {
                if let Some(set) = self.by_type.get_mut(old._type.as_str()) {
                    set.remove(&entity.id);
                }
                if let Some(set) = self.by_class.get_mut(old._class.as_str()) {
                    set.remove(&entity.id);
                }
                if let Some(set) = self.by_source.get_mut(old.source.connector_id.as_str()) {
                    set.remove(&entity.id);
                }
            }
        }
        // Insert into type, class, and source indices.
        self.by_type
            .entry(entity._type.as_str().to_owned())
            .or_default()
            .insert(entity.id);
        self.by_class
            .entry(entity._class.as_str().to_owned())
            .or_default()
            .insert(entity.id);
        if !entity.source.connector_id.is_empty() {
            self.by_source
                .entry(entity.source.connector_id.as_str().to_owned())
                .or_default()
                .insert(entity.id);
        }
        self.approx_bytes += entity.approx_size();
        self.entities.insert(entity.id, entity.clone());
    }

    fn apply_delete_entity(&mut self, id: EntityId) {
        // Remove from type/class/source indices.
        if let Some(old) = self.entities.get(&id) {
            if !old.is_tombstone() {
                if let Some(set) = self.by_type.get_mut(old._type.as_str()) {
                    set.remove(&id);
                }
                if let Some(set) = self.by_class.get_mut(old._class.as_str()) {
                    set.remove(&id);
                }
                if let Some(set) = self.by_source.get_mut(old.source.connector_id.as_str()) {
                    set.remove(&id);
                }
            }
        }
        let tombstone = Entity::tombstone(id);
        self.approx_bytes += tombstone.approx_size();
        self.entities.insert(id, tombstone);
    }

    fn apply_upsert_rel(&mut self, rel: &Relationship) {
        // Clone stale data before releasing the immutable borrow.
        let old_info = self.relationships.get(&rel.id).and_then(|old| {
            if old.is_tombstone() {
                None
            } else {
                Some((
                    old.from_id,
                    old.to_id,
                    old.source.connector_id.as_str().to_owned(),
                ))
            }
        });
        if let Some((from, to, src)) = old_info {
            self.remove_adj(rel.id, from, to);
            if let Some(set) = self.by_source_rel.get_mut(src.as_str()) {
                set.remove(&rel.id);
            }
        }
        // Add new adjacency entries and source index.
        self.add_adj(rel.id, rel.from_id, rel.to_id);
        if !rel.source.connector_id.is_empty() {
            self.by_source_rel
                .entry(rel.source.connector_id.as_str().to_owned())
                .or_default()
                .insert(rel.id);
        }
        self.approx_bytes += rel.approx_size();
        self.relationships.insert(rel.id, rel.clone());
    }

    fn apply_delete_rel(&mut self, id: RelationshipId) {
        // Clone stale data before releasing the immutable borrow.
        let old_info = self.relationships.get(&id).and_then(|old| {
            if old.is_tombstone() {
                None
            } else {
                Some((
                    old.from_id,
                    old.to_id,
                    old.source.connector_id.as_str().to_owned(),
                ))
            }
        });
        if let Some((from, to, src)) = old_info {
            self.remove_adj(id, from, to);
            if let Some(set) = self.by_source_rel.get_mut(src.as_str()) {
                set.remove(&id);
            }
        }
        let tombstone = Relationship::tombstone(id);
        self.approx_bytes += tombstone.approx_size();
        self.relationships.insert(id, tombstone);
    }

    fn add_adj(&mut self, rel_id: RelationshipId, from_id: EntityId, to_id: EntityId) {
        self.adjacency.entry(from_id).or_default().push(AdjEntry {
            relationship_id: rel_id,
            direction: Direction::Outgoing,
            neighbor_id: to_id,
        });
        self.adjacency.entry(to_id).or_default().push(AdjEntry {
            relationship_id: rel_id,
            direction: Direction::Incoming,
            neighbor_id: from_id,
        });
    }

    fn remove_adj(&mut self, rel_id: RelationshipId, from_id: EntityId, to_id: EntityId) {
        if let Some(list) = self.adjacency.get_mut(&from_id) {
            list.retain(|e| e.relationship_id != rel_id);
        }
        if let Some(list) = self.adjacency.get_mut(&to_id) {
            list.retain(|e| e.relationship_id != rel_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_str::CompactString;
    use parallax_core::{
        entity::{EntityClass, EntityType},
        property::Value,
        source::SourceTag,
        timestamp::Timestamp,
    };
    use std::collections::BTreeMap;

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
        use parallax_core::relationship::RelationshipClass;
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
    fn upsert_and_get_entity() {
        let mut mt = MemTable::new();
        let id = EntityId::derive("a", "host", "h1");
        let entity = make_entity(id, "host", "Host");
        let mut batch = WriteBatch::new();
        batch.upsert_entity(entity);
        mt.apply(&batch);

        assert!(mt.get_entity(id).is_some());
        assert_eq!(mt.entity_count(), 1);
    }

    #[test]
    fn delete_entity_creates_tombstone() {
        let mut mt = MemTable::new();
        let id = EntityId::derive("a", "host", "h1");
        let entity = make_entity(id, "host", "Host");

        let mut b1 = WriteBatch::new();
        b1.upsert_entity(entity);
        mt.apply(&b1);

        let mut b2 = WriteBatch::new();
        b2.delete_entity(id);
        mt.apply(&b2);

        assert!(mt.get_entity(id).is_none());
        assert_eq!(mt.entity_count(), 0);
    }

    #[test]
    fn type_index_works() {
        let mut mt = MemTable::new();
        let id1 = EntityId::derive("a", "host", "h1");
        let id2 = EntityId::derive("a", "host", "h2");
        let id3 = EntityId::derive("a", "service", "s1");

        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(id1, "host", "Host"));
        batch.upsert_entity(make_entity(id2, "host", "Host"));
        batch.upsert_entity(make_entity(id3, "service", "Service"));
        mt.apply(&batch);

        let hosts = mt.entities_of_type("host");
        assert_eq!(hosts.len(), 2);

        let services = mt.entities_of_type("service");
        assert_eq!(services.len(), 1);
    }

    #[test]
    fn adjacency_index_maintained() {
        let mut mt = MemTable::new();
        let a = EntityId::derive("acc", "host", "h1");
        let b = EntityId::derive("acc", "host", "h2");
        let rel_id = RelationshipId::derive("acc", "host", "h1", "CONNECTS", "host", "h2");

        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(a, "host", "Host"));
        batch.upsert_entity(make_entity(b, "host", "Host"));
        batch.upsert_relationship(make_rel(rel_id, a, b, "CONNECTS"));
        mt.apply(&batch);

        let a_adj = mt.adjacency(a);
        assert_eq!(a_adj.len(), 1);
        assert_eq!(a_adj[0].direction, Direction::Outgoing);
        assert_eq!(a_adj[0].neighbor_id, b);

        let b_adj = mt.adjacency(b);
        assert_eq!(b_adj.len(), 1);
        assert_eq!(b_adj[0].direction, Direction::Incoming);
        assert_eq!(b_adj[0].neighbor_id, a);
    }

    #[test]
    fn delete_rel_removes_adjacency() {
        let mut mt = MemTable::new();
        let a = EntityId::derive("acc", "host", "h1");
        let b = EntityId::derive("acc", "host", "h2");
        let rel_id = RelationshipId::derive("acc", "host", "h1", "CONNECTS", "host", "h2");

        let mut b1 = WriteBatch::new();
        b1.upsert_entity(make_entity(a, "host", "Host"));
        b1.upsert_entity(make_entity(b, "host", "Host"));
        b1.upsert_relationship(make_rel(rel_id, a, b, "CONNECTS"));
        mt.apply(&b1);

        let mut b2 = WriteBatch::new();
        b2.delete_relationship(rel_id);
        mt.apply(&b2);

        assert_eq!(mt.adjacency(a).len(), 0);
        assert_eq!(mt.adjacency(b).len(), 0);
    }

    #[test]
    fn source_index_filters_by_connector() {
        use compact_str::CompactString;
        use parallax_core::source::SourceTag;

        let mut mt = MemTable::new();
        let id_a = EntityId::derive("acc", "host", "h1");
        let id_b = EntityId::derive("acc", "host", "h2");

        let mut e_a = make_entity(id_a, "host", "Host");
        e_a.source = SourceTag {
            connector_id: CompactString::new("aws"),
            sync_id: CompactString::new("s1"),
            sync_timestamp: parallax_core::timestamp::Timestamp::default(),
        };
        let mut e_b = make_entity(id_b, "host", "Host");
        e_b.source = SourceTag {
            connector_id: CompactString::new("gcp"),
            sync_id: CompactString::new("s1"),
            sync_timestamp: parallax_core::timestamp::Timestamp::default(),
        };

        let mut batch = WriteBatch::new();
        batch.upsert_entity(e_a);
        batch.upsert_entity(e_b);
        mt.apply(&batch);

        assert_eq!(mt.entities_by_source("aws").len(), 1);
        assert_eq!(mt.entities_by_source("gcp").len(), 1);
        assert_eq!(mt.entities_by_source("azure").len(), 0);
    }

    #[test]
    fn source_index_cleared_on_delete() {
        use compact_str::CompactString;
        use parallax_core::source::SourceTag;

        let mut mt = MemTable::new();
        let id = EntityId::derive("acc", "host", "h1");
        let mut entity = make_entity(id, "host", "Host");
        entity.source = SourceTag {
            connector_id: CompactString::new("aws"),
            sync_id: CompactString::new("s1"),
            sync_timestamp: parallax_core::timestamp::Timestamp::default(),
        };

        let mut b1 = WriteBatch::new();
        b1.upsert_entity(entity);
        mt.apply(&b1);
        assert_eq!(mt.entities_by_source("aws").len(), 1);

        let mut b2 = WriteBatch::new();
        b2.delete_entity(id);
        mt.apply(&b2);
        assert_eq!(mt.entities_by_source("aws").len(), 0);
    }

    #[test]
    fn value_unused_import_check() {
        // Ensures the Value import in the parent module is reachable.
        let _ = Value::Null;
    }
}
