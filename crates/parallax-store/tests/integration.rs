//! Integration tests for parallax-store.
//!
//! These tests exercise the full storage stack: WAL → MemTable → Segment →
//! Snapshot, including cross-session recovery and flush behaviour.

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    relationship::{Relationship, RelationshipClass, RelationshipId},
    source::SourceTag,
    timestamp::Timestamp,
};
use parallax_store::{StorageEngine, StoreConfig, WriteBatch};
use std::collections::BTreeMap;
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open(dir: &TempDir) -> StorageEngine {
    StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine")
}

fn entity(account: &str, typ: &str, key: &str) -> Entity {
    Entity {
        id: EntityId::derive(account, typ, key),
        _type: EntityType::new_unchecked(typ),
        _class: EntityClass::new_unchecked("Generic"),
        display_name: CompactString::new(key),
        properties: BTreeMap::new(),
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

fn relationship(
    account: &str,
    from_type: &str,
    from_key: &str,
    verb: &str,
    to_type: &str,
    to_key: &str,
) -> Relationship {
    Relationship {
        id: RelationshipId::derive(account, from_type, from_key, verb, to_type, to_key),
        from_id: EntityId::derive(account, from_type, from_key),
        to_id: EntityId::derive(account, to_type, to_key),
        _class: RelationshipClass::new_unchecked(verb),
        properties: BTreeMap::new(),
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Write entities across multiple batches; all must be visible in latest snapshot.
#[test]
fn multi_batch_write_all_visible() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    for i in 0..5u32 {
        let mut batch = WriteBatch::new();
        batch.upsert_entity(entity("acme", "host", &format!("h{i}")));
        engine.write(batch).unwrap();
    }

    let snap = engine.snapshot();
    assert_eq!(snap.entity_count(), 5);
    for i in 0..5u32 {
        let id = EntityId::derive("acme", "host", &format!("h{i}"));
        assert!(snap.get_entity(id).is_some(), "missing h{i}");
    }
}

/// A snapshot taken before a write sees the old data; the new snapshot sees the new data.
#[test]
fn snapshot_isolation() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    let id = EntityId::derive("acme", "host", "h1");
    let mut b1 = WriteBatch::new();
    b1.upsert_entity(entity("acme", "host", "h1"));
    engine.write(b1).unwrap();

    let old = engine.snapshot();

    let mut b2 = WriteBatch::new();
    b2.delete_entity(id);
    engine.write(b2).unwrap();

    assert!(
        old.get_entity(id).is_some(),
        "old snapshot must still see h1"
    );
    assert!(
        engine.snapshot().get_entity(id).is_none(),
        "new snapshot must not see h1"
    );
}

/// Cross-session WAL recovery restores all entities.
#[test]
fn wal_recovery_restores_all_entities() {
    let dir = TempDir::new().unwrap();
    let ids: Vec<EntityId> = (0..3)
        .map(|i| EntityId::derive("acme", "host", &format!("h{i}")))
        .collect();

    {
        let mut engine = open(&dir);
        for (i, _id) in ids.iter().enumerate() {
            let mut batch = WriteBatch::new();
            batch.upsert_entity(entity("acme", "host", &format!("h{i}")));
            engine.write(batch).unwrap();
        }
    } // engine dropped — WAL fsynced

    let engine2 = open(&dir);
    let snap = engine2.snapshot();
    for id in &ids {
        assert!(snap.get_entity(*id).is_some());
    }
    assert_eq!(engine2.version(), 3);
}

/// Flush moves entity data to segment but snapshot still reads correctly.
#[test]
fn segment_flush_then_read() {
    let dir = TempDir::new().unwrap();
    let config = StoreConfig {
        data_dir: dir.path().to_path_buf(),
        memtable_flush_size: 0, // flush after every write
        ..Default::default()
    };
    let mut engine = StorageEngine::open(config).unwrap();

    let id = EntityId::derive("acme", "host", "h1");
    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity("acme", "host", "h1"));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let e = snap.get_entity(id).expect("must find entity after flush");
    assert_eq!(e._type.as_str(), "host");
}

/// Relationship adjacency is visible after write; adjacency list has correct degree.
#[test]
fn relationship_adjacency_end_to_end() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity("acme", "host", "a"));
    batch.upsert_entity(entity("acme", "host", "b"));
    batch.upsert_entity(entity("acme", "host", "c"));
    batch.upsert_relationship(relationship("acme", "host", "a", "CONNECTS", "host", "b"));
    batch.upsert_relationship(relationship("acme", "host", "a", "CONNECTS", "host", "c"));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let a = EntityId::derive("acme", "host", "a");
    assert_eq!(snap.adjacency(a).len(), 2, "a must have degree 2");
}

/// Entities can be filtered by source connector.
#[test]
fn entities_by_source_partition() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    let mut batch = WriteBatch::new();
    // Two entities for connector "aws"
    let mut e_aws = entity("acme", "host", "aws1");
    e_aws.source.connector_id = CompactString::new("aws");
    let mut e_aws2 = entity("acme", "host", "aws2");
    e_aws2.source.connector_id = CompactString::new("aws");
    // One entity for connector "okta"
    let mut e_okta = entity("acme", "user", "u1");
    e_okta.source.connector_id = CompactString::new("okta");

    batch.upsert_entity(e_aws);
    batch.upsert_entity(e_aws2);
    batch.upsert_entity(e_okta);
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let aws_ents = snap.entities_by_source("aws");
    let okta_ents = snap.entities_by_source("okta");
    assert_eq!(aws_ents.len(), 2);
    assert_eq!(okta_ents.len(), 1);
}

// ─── v0.2: WAL group commit + dump_wal ───────────────────────────────────────

/// `write_many` commits N batches atomically; all entities visible after reopen.
#[test]
fn v02_group_commit_all_entities_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let n = 20usize;

    {
        let mut engine = open(&dir);
        let batches: Vec<WriteBatch> = (0..n)
            .map(|i| {
                let mut b = WriteBatch::new();
                b.upsert_entity(entity("acme", "host", &format!("gc-h{i}")));
                b
            })
            .collect();
        engine.write_many(batches).expect("write_many");
    } // engine dropped — WAL fsynced

    let engine2 = open(&dir);
    let snap = engine2.snapshot();
    assert_eq!(
        snap.entity_count(),
        n,
        "group-committed entities must survive reopen"
    );
    for i in 0..n {
        let id = EntityId::derive("acme", "host", &format!("gc-h{i}"));
        assert!(
            snap.get_entity(id).is_some(),
            "gc-h{i} missing after reopen"
        );
    }
}

/// `write_many` skips empty batches — only non-empty batches are stored.
#[test]
fn v02_group_commit_skips_empty_batches() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    let mut filled = WriteBatch::new();
    filled.upsert_entity(entity("acme", "host", "real"));

    let seqs = engine
        .write_many(vec![
            WriteBatch::new(), // empty — skipped
            filled,
            WriteBatch::new(), // empty — skipped
        ])
        .expect("write_many");

    // Only one non-empty batch → one WAL entry
    assert_eq!(seqs.len(), 1);
    assert_eq!(engine.snapshot().entity_count(), 1);
}

/// `write_many` with mixed writes + deletes: net result correct.
#[test]
fn v02_group_commit_with_delete_in_same_call() {
    let dir = TempDir::new().unwrap();
    let mut engine = open(&dir);

    let id = EntityId::derive("acme", "host", "doomed");
    let mut b1 = WriteBatch::new();
    b1.upsert_entity(entity("acme", "host", "doomed"));
    b1.upsert_entity(entity("acme", "host", "survivor"));
    let mut b2 = WriteBatch::new();
    b2.delete_entity(id);

    engine.write_many(vec![b1, b2]).expect("write_many");

    let snap = engine.snapshot();
    assert!(snap.get_entity(id).is_none(), "doomed must be deleted");
    assert_eq!(snap.entity_count(), 1, "only survivor remains");
}

/// `dump_wal` returns one entry per WAL batch written.
#[test]
fn v02_dump_wal_returns_all_batches() {
    use parallax_store::dump_wal;

    let dir = TempDir::new().unwrap();
    let n = 5usize;

    {
        let mut engine = open(&dir);
        for i in 0..n {
            let mut b = WriteBatch::new();
            b.upsert_entity(entity("acme", "host", &format!("d{i}")));
            engine.write(b).unwrap();
        }
    }

    let entries = dump_wal(dir.path()).expect("dump_wal");
    assert_eq!(entries.len(), n, "dump_wal must return one entry per write");
    // Sequences must be strictly increasing
    for w in entries.windows(2) {
        assert!(w[0].seq < w[1].seq, "WAL sequences must be monotonic");
    }
}

/// `dump_wal` entries carry the correct operation counts.
#[test]
fn v02_dump_wal_entry_op_counts_correct() {
    use parallax_store::dump_wal;

    let dir = TempDir::new().unwrap();
    {
        let mut engine = open(&dir);
        let mut b = WriteBatch::new();
        b.upsert_entity(entity("acme", "host", "e1"));
        b.upsert_entity(entity("acme", "host", "e2"));
        b.upsert_entity(entity("acme", "host", "e3"));
        engine.write(b).unwrap();
    }

    let entries = dump_wal(dir.path()).expect("dump_wal");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].batch.len(), 3, "batch must contain 3 ops");
}

/// `dump_wal` on an empty data dir returns an empty vec, not an error.
#[test]
fn v02_dump_wal_empty_dir_returns_empty() {
    use parallax_store::dump_wal;

    let dir = TempDir::new().unwrap();
    // Just open and close without writing — WAL dir created but no entries
    let _engine = open(&dir);
    drop(_engine);

    let entries = dump_wal(dir.path()).expect("dump_wal on empty");
    assert!(entries.is_empty());
}
