//! Test helpers for parallax-graph unit tests.
//!
//! Provides a `GraphBuilder` and `make_graph()` that construct an in-memory
//! `StorageEngine` with a small synthetic graph for testing.

use std::collections::BTreeMap;

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    property::Value,
    relationship::{Relationship, RelationshipClass, RelationshipId},
    source::SourceTag,
    timestamp::Timestamp,
};
use parallax_store::{StoreConfig, StorageEngine, WriteBatch};
use tempfile::TempDir;

/// Fluent graph builder for test scenarios.
///
/// Methods take `&mut self` so test closures can end with `;` without
/// type-mismatch errors.
pub struct GraphBuilder {
    batch: WriteBatch,
    pending_props: Vec<(EntityId, String, Value)>,
}

impl GraphBuilder {
    fn new() -> Self {
        GraphBuilder {
            batch: WriteBatch::new(),
            pending_props: Vec::new(),
        }
    }

    /// Add a `host` entity.
    pub fn host(&mut self, account: &str, key: &str) -> &mut Self {
        self.entity(account, "host", "Host", key)
    }

    /// Add a `service` entity.
    pub fn service(&mut self, account: &str, key: &str) -> &mut Self {
        self.entity(account, "service", "Service", key)
    }

    /// Add a `host` entity with one property set.
    pub fn host_with(
        &mut self,
        account: &str,
        key: &str,
        prop_key: &str,
        prop_val: impl Into<Value>,
    ) -> &mut Self {
        self.host(account, key);
        let id = EntityId::derive(account, "host", key);
        self.pending_props.push((id, prop_key.to_owned(), prop_val.into()));
        self
    }

    /// Add an entity of any type and class.
    pub fn entity(&mut self, account: &str, typ: &str, class: &str, key: &str) -> &mut Self {
        let id = EntityId::derive(account, typ, key);
        let entity = Entity {
            id,
            _type: EntityType::new_unchecked(typ),
            _class: EntityClass::new_unchecked(class),
            display_name: CompactString::new(key),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        };
        self.batch.upsert_entity(entity);
        self
    }

    /// Set a property on an existing entity by type+key.
    pub fn prop(
        &mut self,
        account: &str,
        typ: &str,
        key: &str,
        prop_key: &str,
        prop_val: impl Into<Value>,
    ) -> &mut Self {
        let id = EntityId::derive(account, typ, key);
        self.pending_props.push((id, prop_key.to_owned(), prop_val.into()));
        self
    }

    /// Add a relationship between two entities.
    pub fn rel(
        &mut self,
        account: &str,
        from_type: &str,
        from_key: &str,
        class: &str,
        to_type: &str,
        to_key: &str,
    ) -> &mut Self {
        let from_id = EntityId::derive(account, from_type, from_key);
        let to_id = EntityId::derive(account, to_type, to_key);
        let rel_id =
            RelationshipId::derive(account, from_type, from_key, class, to_type, to_key);
        let rel = Relationship {
            id: rel_id,
            from_id,
            to_id,
            _class: RelationshipClass::new_unchecked(class),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        };
        self.batch.upsert_relationship(rel);
        self
    }
}

/// Build a `StorageEngine` from a closure that configures a `GraphBuilder`.
///
/// The closure receives `&mut GraphBuilder` so trailing semicolons are fine.
/// Returns `(engine, TempDir)` — keep `TempDir` alive for the duration of the test.
pub fn make_graph<F>(configure: F) -> (StorageEngine, TempDir)
where
    F: FnOnce(&mut GraphBuilder),
{
    let dir = tempfile::tempdir().expect("tempdir");
    let config = StoreConfig::new(dir.path());
    let mut engine = StorageEngine::open(config).expect("open engine");

    let mut builder = GraphBuilder::new();
    configure(&mut builder);

    if !builder.batch.is_empty() {
        engine.write(builder.batch).expect("write batch");
    }

    // Apply property overrides as individual upserts.
    for (entity_id, prop_key, prop_val) in builder.pending_props {
        let snap = engine.snapshot();
        if let Some(existing) = snap.get_entity(entity_id) {
            let mut updated = existing.clone();
            updated.properties.insert(CompactString::new(&prop_key), prop_val);
            let mut batch = WriteBatch::new();
            batch.upsert_entity(updated);
            drop(snap);
            engine.write(batch).expect("write prop update");
        }
    }

    (engine, dir)
}
