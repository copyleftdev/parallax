//! Source-scoped sync protocol — diff and commit connector output.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.6
//!
//! INV-C01: A sync commit is atomic. Either all entities/relationships land or none.
//! INV-C02: Entities from connector A are never deleted by a sync from connector B.
//! INV-C03: Entity IDs are deterministic from (account_id, type, key).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use parallax_core::{
    entity::{Entity, EntityId},
    relationship::{Relationship, RelationshipId},
};
use parallax_store::{StorageEngine, WriteBatch};
use tracing::debug;

use crate::error::SyncError;
use crate::validate::validate_sync_batch;

/// Sync statistics from a single commit.
#[derive(Debug, Default, Clone)]
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

/// Result of a completed sync.
#[derive(Debug)]
pub struct SyncResult {
    pub sync_id: String,
    pub stats: SyncStats,
}

/// Handles source-scoped diff and atomic batch commit.
///
/// Multiple connectors share one `SyncEngine`, each calling `commit_sync`
/// independently. The engine lock is held only during the write phase.
#[derive(Clone)]
pub struct SyncEngine {
    store: Arc<Mutex<StorageEngine>>,
}

impl SyncEngine {
    pub fn new(store: Arc<Mutex<StorageEngine>>) -> Self {
        SyncEngine { store }
    }

    /// Commit a connector's sync output to the graph (INV-C01, INV-C02).
    ///
    /// Diffs the emitted data against the current snapshot for this connector,
    /// then commits the entire delta as a single WriteBatch.
    pub fn commit_sync(
        &self,
        connector_id: &str,
        sync_id: &str,
        entities: Vec<Entity>,
        relationships: Vec<Relationship>,
    ) -> Result<SyncResult, SyncError> {
        let seen_entity_ids: HashSet<EntityId> = entities.iter().map(|e| e.id).collect();
        let seen_rel_ids: HashSet<RelationshipId> = relationships.iter().map(|r| r.id).collect();

        // Take a lock-free snapshot for validation and diff, then drop the lock.
        let (existing_entities, existing_rels) = {
            let engine = self.store.lock().expect("engine lock");
            let snap = engine.snapshot();
            // Validate referential integrity and class/verb constraints (INV-04).
            validate_sync_batch(&entities, &relationships, &snap)?;
            let ents = snap
                .entities_by_source(connector_id)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            let rels = snap
                .relationships_by_source(connector_id)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            (ents, rels)
        };

        let mut batch = WriteBatch::new();
        let mut stats = SyncStats::default();

        // O(1) lookups via HashMap (was O(n) linear scan — M2 fix).
        let existing_entity_map: HashMap<EntityId, &Entity> =
            existing_entities.iter().map(|e| (e.id, e)).collect();
        let existing_rel_map: HashMap<RelationshipId, &Relationship> =
            existing_rels.iter().map(|r| (r.id, r)).collect();

        for entity in &entities {
            match existing_entity_map.get(&entity.id) {
                None => {
                    batch.upsert_entity(entity.clone());
                    stats.entities_created += 1;
                }
                // M3 fix: compare full entity content, not just properties.
                Some(ex) if !entities_equivalent(ex, entity) => {
                    batch.upsert_entity(entity.clone());
                    stats.entities_updated += 1;
                }
                Some(_) => {
                    stats.entities_unchanged += 1;
                }
            }
        }
        for existing in &existing_entities {
            if !seen_entity_ids.contains(&existing.id) {
                batch.delete_entity(existing.id);
                stats.entities_deleted += 1;
            }
        }
        for rel in &relationships {
            match existing_rel_map.get(&rel.id) {
                None => {
                    batch.upsert_relationship(rel.clone());
                    stats.relationships_created += 1;
                }
                Some(ex) if !relationships_equivalent(ex, rel) => {
                    batch.upsert_relationship(rel.clone());
                    stats.relationships_updated += 1;
                }
                Some(_) => {
                    stats.relationships_unchanged += 1;
                }
            }
        }
        for existing in &existing_rels {
            if !seen_rel_ids.contains(&existing.id) {
                batch.delete_relationship(existing.id);
                stats.relationships_deleted += 1;
            }
        }

        debug!(
            connector_id,
            sync_id,
            created = stats.entities_created,
            updated = stats.entities_updated,
            deleted = stats.entities_deleted,
            "sync diff computed"
        );

        // Commit atomically (INV-C01): acquire writer lock, write, release.
        if !batch.is_empty() {
            let mut engine = self.store.lock().expect("engine lock");
            engine.write(batch).map_err(SyncError::StoreError)?;
        }

        Ok(SyncResult {
            sync_id: sync_id.to_string(),
            stats,
        })
    }
}

/// Convenience entry point when you have exclusive ownership of the engine.
///
/// Used by `parallax-connect`'s step scheduler which holds `&mut StorageEngine`.
pub fn commit_sync_exclusive(
    engine: &mut StorageEngine,
    connector_id: &str,
    sync_id: &str,
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
) -> Result<SyncResult, SyncError> {
    let seen_entity_ids: HashSet<EntityId> = entities.iter().map(|e| e.id).collect();
    let seen_rel_ids: HashSet<RelationshipId> = relationships.iter().map(|r| r.id).collect();

    let snap = engine.snapshot();
    // Validate referential integrity and class/verb constraints (INV-04).
    validate_sync_batch(&entities, &relationships, &snap)?;
    let existing_entities: Vec<Entity> = snap
        .entities_by_source(connector_id)
        .into_iter()
        .cloned()
        .collect();
    let existing_rels: Vec<Relationship> = snap
        .relationships_by_source(connector_id)
        .into_iter()
        .cloned()
        .collect();
    drop(snap);

    let mut batch = WriteBatch::new();
    let mut stats = SyncStats::default();

    let existing_entity_map: HashMap<EntityId, &Entity> =
        existing_entities.iter().map(|e| (e.id, e)).collect();
    let existing_rel_map: HashMap<RelationshipId, &Relationship> =
        existing_rels.iter().map(|r| (r.id, r)).collect();

    for entity in &entities {
        match existing_entity_map.get(&entity.id) {
            None => {
                batch.upsert_entity(entity.clone());
                stats.entities_created += 1;
            }
            Some(ex) if !entities_equivalent(ex, entity) => {
                batch.upsert_entity(entity.clone());
                stats.entities_updated += 1;
            }
            Some(_) => {
                stats.entities_unchanged += 1;
            }
        }
    }
    for existing in &existing_entities {
        if !seen_entity_ids.contains(&existing.id) {
            batch.delete_entity(existing.id);
            stats.entities_deleted += 1;
        }
    }
    for rel in &relationships {
        match existing_rel_map.get(&rel.id) {
            None => {
                batch.upsert_relationship(rel.clone());
                stats.relationships_created += 1;
            }
            Some(ex) if !relationships_equivalent(ex, rel) => {
                batch.upsert_relationship(rel.clone());
                stats.relationships_updated += 1;
            }
            Some(_) => {
                stats.relationships_unchanged += 1;
            }
        }
    }
    for existing in &existing_rels {
        if !seen_rel_ids.contains(&existing.id) {
            batch.delete_relationship(existing.id);
            stats.relationships_deleted += 1;
        }
    }

    if !batch.is_empty() {
        engine.write(batch).map_err(SyncError::StoreError)?;
    }

    Ok(SyncResult {
        sync_id: sync_id.to_string(),
        stats,
    })
}

// ─── Diff helpers ─────────────────────────────────────────────────────────────

/// True if two entity snapshots are semantically identical (no write needed).
///
/// Compares all mutable fields that a connector may update: display_name,
/// class, and properties. Source/timestamp fields are excluded (they change
/// on every sync and do not affect graph semantics).
fn entities_equivalent(existing: &Entity, incoming: &Entity) -> bool {
    existing.display_name == incoming.display_name
        && existing._class == incoming._class
        && existing.properties == incoming.properties
}

/// True if two relationship snapshots are semantically identical.
fn relationships_equivalent(existing: &Relationship, incoming: &Relationship) -> bool {
    existing._class == incoming._class && existing.properties == incoming.properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_str::CompactString;
    use parallax_core::{
        entity::{Entity, EntityClass, EntityId, EntityType},
        property::Value,
        source::SourceTag,
        timestamp::Timestamp,
    };
    use parallax_store::{StorageEngine, StoreConfig};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn open_engine() -> (StorageEngine, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let engine = StorageEngine::open(StoreConfig::new(dir.path())).expect("open");
        (engine, dir)
    }

    fn make_entity(
        connector_id: &str,
        sync_id: &str,
        key: &str,
        props: Vec<(&str, Value)>,
    ) -> Entity {
        let id = EntityId::derive("acme", "host", key);
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(CompactString::new(k), v);
        }
        Entity {
            id,
            _type: EntityType::new_unchecked("host"),
            _class: EntityClass::new_unchecked("Host"),
            display_name: CompactString::new(key),
            properties,
            source: SourceTag {
                connector_id: CompactString::new(connector_id),
                sync_id: CompactString::new(sync_id),
                sync_timestamp: Timestamp::now(),
            },
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    #[test]
    fn first_sync_creates_entities() {
        let (mut engine, _dir) = open_engine();
        let entities = vec![
            make_entity("aws", "sync-1", "h1", vec![]),
            make_entity("aws", "sync-1", "h2", vec![]),
        ];
        let result = commit_sync_exclusive(&mut engine, "aws", "sync-1", entities, vec![]).unwrap();
        assert_eq!(result.stats.entities_created, 2);
        assert_eq!(result.stats.entities_deleted, 0);
    }

    #[test]
    fn second_sync_removes_departed_entity() {
        let (mut engine, _dir) = open_engine();

        // First sync: h1 and h2
        let e1 = make_entity("aws", "sync-1", "h1", vec![]);
        let e2 = make_entity("aws", "sync-1", "h2", vec![]);
        commit_sync_exclusive(&mut engine, "aws", "sync-1", vec![e1, e2], vec![]).unwrap();

        // Second sync: only h1 (h2 was deleted from AWS)
        let e1_again = make_entity("aws", "sync-2", "h1", vec![]);
        let result =
            commit_sync_exclusive(&mut engine, "aws", "sync-2", vec![e1_again], vec![]).unwrap();
        assert_eq!(result.stats.entities_deleted, 1);
        assert_eq!(result.stats.entities_unchanged, 1);
    }

    #[test]
    fn connector_b_does_not_delete_connector_a_entities() {
        let (mut engine, _dir) = open_engine();

        // Connector A syncs h1.
        let e_a = make_entity("aws", "sync-a", "h1", vec![]);
        commit_sync_exclusive(&mut engine, "aws", "sync-a", vec![e_a], vec![]).unwrap();

        // Connector B syncs nothing. Should not delete A's entity.
        let result = commit_sync_exclusive(&mut engine, "okta", "sync-b", vec![], vec![]).unwrap();
        assert_eq!(result.stats.entities_deleted, 0);

        // A's entity still exists.
        let snap = engine.snapshot();
        let h1_id = EntityId::derive("acme", "host", "h1");
        assert!(snap.get_entity(h1_id).is_some());
    }

    #[test]
    fn updated_properties_trigger_upsert() {
        let (mut engine, _dir) = open_engine();

        let e1 = make_entity(
            "aws",
            "sync-1",
            "h1",
            vec![("state", Value::from("running"))],
        );
        commit_sync_exclusive(&mut engine, "aws", "sync-1", vec![e1], vec![]).unwrap();

        // Same entity but state changed.
        let e1_updated = make_entity(
            "aws",
            "sync-2",
            "h1",
            vec![("state", Value::from("stopped"))],
        );
        let result =
            commit_sync_exclusive(&mut engine, "aws", "sync-2", vec![e1_updated], vec![]).unwrap();
        assert_eq!(result.stats.entities_updated, 1);
        assert_eq!(result.stats.entities_created, 0);
    }
}
