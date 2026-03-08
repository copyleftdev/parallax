//! Ingest validation — referential integrity and schema checks.
//!
//! Called before every sync commit to catch problems at write time rather
//! than at query time (spec §1.3 "rejected at write time, not silently accepted").
//!
//! **Spec references:**
//! - `specs/01-data-model.md` §1.3 (EntityClass), §1.4 (RelationshipClass)
//! - `specs/01-data-model.md` §1.8 (Referential integrity — INV-04)
//!
//! INV-04: Every relationship's `from_id` and `to_id` must reference an
//!         entity that either exists in the current snapshot OR is being
//!         upserted in the same batch.

use std::collections::HashSet;

use parallax_core::{
    entity::{Entity, EntityId, KNOWN_CLASSES},
    relationship::{Relationship, KNOWN_VERBS},
};
use parallax_store::Snapshot;
use tracing::warn;

use crate::error::SyncError;

/// Validate a set of entities and relationships before committing.
///
/// Returns `Ok(())` if the batch is valid.
/// Returns `Err(SyncError::ValidationFailed)` on the first hard violation
/// (dangling reference). Class/verb warnings are logged but do not abort.
pub fn validate_sync_batch(
    entities: &[Entity],
    relationships: &[Relationship],
    snap: &Snapshot,
) -> Result<(), SyncError> {
    // Build the set of entity IDs available after this batch lands:
    // existing live entities + those being upserted now.
    let mut available: HashSet<EntityId> = entities.iter().map(|e| e.id).collect();
    for e in snap.all_entities() {
        available.insert(e.id);
    }

    // --- Referential integrity (INV-04) ---
    for rel in relationships {
        if !available.contains(&rel.from_id) {
            return Err(SyncError::DanglingRelationship {
                entity_id: format!("{}", rel.from_id),
            });
        }
        if !available.contains(&rel.to_id) {
            return Err(SyncError::DanglingRelationship {
                entity_id: format!("{}", rel.to_id),
            });
        }
    }

    // --- EntityClass validation (spec §1.3) ---
    // Unknown classes are warned about but do not abort in v0.1 to allow
    // connectors to use custom classes during development. Hard rejection
    // will be the default in v0.2 (configurable via IngestConfig).
    for entity in entities {
        let cls = entity._class.as_str();
        if !KNOWN_CLASSES.contains(&cls) {
            warn!(
                entity_id = %entity.id,
                class = cls,
                "EntityClass not in KNOWN_CLASSES — will be rejected in v0.2 strict mode"
            );
        }
    }

    // --- RelationshipClass validation (spec §1.4) ---
    for rel in relationships {
        let verb = rel._class.as_str();
        if !KNOWN_VERBS.contains(&verb) {
            warn!(
                rel_id = %rel.id,
                verb = verb,
                "RelationshipClass not in KNOWN_VERBS — will be rejected in v0.2 strict mode"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_str::CompactString;
    use parallax_core::{
        entity::{EntityClass, EntityType},
        relationship::{RelationshipClass, RelationshipId},
        source::SourceTag,
        timestamp::Timestamp,
    };
    use parallax_store::{StorageEngine, StoreConfig};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn make_entity(key: &str) -> Entity {
        Entity {
            id: EntityId::derive("acme", "host", key),
            _type: EntityType::new_unchecked("host"),
            _class: EntityClass::new_unchecked("Host"),
            display_name: CompactString::new(key),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    fn make_rel(from: &str, verb: &str, to: &str) -> Relationship {
        Relationship {
            id: RelationshipId::derive("acme", "host", from, verb, "host", to),
            from_id: EntityId::derive("acme", "host", from),
            to_id: EntityId::derive("acme", "host", to),
            _class: RelationshipClass::new_unchecked(verb),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    fn empty_snap() -> (StorageEngine, TempDir) {
        let dir = TempDir::new().unwrap();
        let engine = StorageEngine::open(StoreConfig::new(dir.path())).unwrap();
        (engine, dir)
    }

    #[test]
    fn valid_batch_passes() {
        let (engine, _dir) = empty_snap();
        let snap = engine.snapshot();
        let e = make_entity("h1");
        let r = make_rel("h1", "CONNECTS", "h1");
        assert!(validate_sync_batch(&[e], &[r], &snap).is_ok());
    }

    #[test]
    fn dangling_from_id_rejected() {
        let (engine, _dir) = empty_snap();
        let snap = engine.snapshot();
        // Relationship references h-ghost which is not in the batch or snapshot.
        let e = make_entity("h1");
        let r = make_rel("h-ghost", "CONNECTS", "h1");
        let result = validate_sync_batch(&[e], &[r], &snap);
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(SyncError::DanglingRelationship { .. })
        ));
    }

    #[test]
    fn dangling_to_id_rejected() {
        let (engine, _dir) = empty_snap();
        let snap = engine.snapshot();
        let e = make_entity("h1");
        let r = make_rel("h1", "CONNECTS", "h-ghost");
        assert!(validate_sync_batch(&[e], &[r], &snap).is_err());
    }

    #[test]
    fn existing_snapshot_entity_satisfies_reference() {
        let dir = TempDir::new().unwrap();
        let mut engine = StorageEngine::open(StoreConfig::new(dir.path())).unwrap();
        // h1 already in the store from a previous sync.
        let mut batch = parallax_store::WriteBatch::new();
        batch.upsert_entity(make_entity("h1"));
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        // New batch: only h2, but relationship goes h2 → h1 (h1 is in snap).
        let e2 = make_entity("h2");
        let r = make_rel("h2", "CONNECTS", "h1");
        assert!(validate_sync_batch(&[e2], &[r], &snap).is_ok());
    }

    #[test]
    fn empty_batch_passes() {
        let (engine, _dir) = empty_snap();
        let snap = engine.snapshot();
        assert!(validate_sync_batch(&[], &[], &snap).is_ok());
    }
}
