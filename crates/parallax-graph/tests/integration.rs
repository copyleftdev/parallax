//! Integration tests for parallax-graph.
//!
//! Exercises traversal, pattern matching, blast radius, and coverage
//! analysis against a realistic multi-node graph.

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    relationship::{Direction, Relationship, RelationshipClass, RelationshipId},
    source::SourceTag,
    timestamp::Timestamp,
};
use parallax_graph::GraphReader;
use parallax_store::{StoreConfig, StorageEngine, WriteBatch};
use std::collections::BTreeMap;
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open_engine(dir: &TempDir) -> StorageEngine {
    StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine")
}

fn make_entity(typ: &str, class: &str, key: &str) -> Entity {
    Entity {
        id: EntityId::derive("acme", typ, key),
        _type: EntityType::new_unchecked(typ),
        _class: EntityClass::new_unchecked(class),
        display_name: CompactString::new(key),
        properties: BTreeMap::new(),
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

fn make_rel(from_type: &str, from_key: &str, verb: &str, to_type: &str, to_key: &str) -> Relationship {
    Relationship {
        id: RelationshipId::derive("acme", from_type, from_key, verb, to_type, to_key),
        from_id: EntityId::derive("acme", from_type, from_key),
        to_id: EntityId::derive("acme", to_type, to_key),
        _class: RelationshipClass::new_unchecked(verb),
        properties: BTreeMap::new(),
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

/// Build a small test graph:
///
///   host:a --RUNS--> service:svc1 --READS--> database:db1
///   host:b --RUNS--> service:svc1
///   host:c (isolated)
fn build_test_graph(engine: &mut StorageEngine) {
    let mut batch = WriteBatch::new();
    batch.upsert_entity(make_entity("host", "Host", "a"));
    batch.upsert_entity(make_entity("host", "Host", "b"));
    batch.upsert_entity(make_entity("host", "Host", "c"));
    batch.upsert_entity(make_entity("service", "Service", "svc1"));
    batch.upsert_entity(make_entity("database", "DataStore", "db1"));

    batch.upsert_relationship(make_rel("host", "a", "RUNS", "service", "svc1"));
    batch.upsert_relationship(make_rel("host", "b", "RUNS", "service", "svc1"));
    batch.upsert_relationship(make_rel("service", "svc1", "READS", "database", "db1"));
    engine.write(batch).unwrap();
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Single-hop traversal from host:a reaches svc1.
#[test]
fn single_hop_traversal() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let a = EntityId::derive("acme", "host", "a");
    let results = graph.traverse(a).direction(Direction::Outgoing).collect();
    let ids: Vec<EntityId> = results.iter().map(|r| r.entity.id).collect();

    let svc1 = EntityId::derive("acme", "service", "svc1");
    assert!(ids.contains(&svc1), "single-hop must reach svc1");
}

/// Multi-hop (depth 3) traversal from host:a reaches db1.
#[test]
fn multi_hop_traversal_reaches_database() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let a = EntityId::derive("acme", "host", "a");
    let results = graph.traverse(a).direction(Direction::Outgoing).max_depth(3).collect();
    let ids: Vec<EntityId> = results.iter().map(|r| r.entity.id).collect();

    let db1 = EntityId::derive("acme", "database", "db1");
    assert!(ids.contains(&db1), "depth-3 traversal must reach db1");
}

/// Blast radius from host:a covers svc1 (via RUNS attack edge).
#[test]
fn blast_radius_covers_downstream() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let a = EntityId::derive("acme", "host", "a");
    // Use a custom attack edge "RUNS" since default rules don't include it.
    let result = graph
        .blast_radius(a)
        .add_attack_edge("RUNS", Direction::Outgoing)
        .add_attack_edge("READS", Direction::Outgoing)
        .analyze();

    let impacted_ids: Vec<EntityId> = result.impacted.iter().map(|r| r.entity.id).collect();
    let svc1 = EntityId::derive("acme", "service", "svc1");
    assert!(impacted_ids.contains(&svc1), "svc1 must be in blast radius");
}

/// Isolated host:c has empty blast radius.
#[test]
fn blast_radius_isolated_node_is_empty() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let c = EntityId::derive("acme", "host", "c");
    let result = graph.blast_radius(c).default_rules().analyze();
    assert!(result.impacted.is_empty());
}

/// `find` by type returns the right set.
#[test]
fn find_by_type_returns_correct_set() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let hosts = graph.find("host").collect();
    assert_eq!(hosts.len(), 3, "must find all 3 hosts");
    let dbs = graph.find("database").collect();
    assert_eq!(dbs.len(), 1);
}

/// Shortest path from host:a to database:db1 via RUNS+READS has 2 hops.
#[test]
fn shortest_path_a_to_db() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let a = EntityId::derive("acme", "host", "a");
    let db1 = EntityId::derive("acme", "database", "db1");

    let path = graph.shortest_path(a, db1).find();
    assert!(path.is_some(), "path must exist from a to db1");
    assert_eq!(path.unwrap().segments.len(), 2, "path must be 2 hops");
}

/// No path exists from isolated host:c to db1.
#[test]
fn no_path_from_isolated_host() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let c = EntityId::derive("acme", "host", "c");
    let db1 = EntityId::derive("acme", "database", "db1");

    assert!(graph.shortest_path(c, db1).find().is_none());
}

/// Coverage gap: host:c has no RUNS edge, so it is in the coverage gap.
#[test]
fn coverage_gap_finds_isolated_host() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    build_test_graph(&mut engine);

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let unprotected = graph
        .coverage_gap("RUNS")
        .target_type("host")
        .neighbor_type("service")
        .find();

    let ids: Vec<EntityId> = unprotected.iter().map(|e| e.id).collect();
    let c = EntityId::derive("acme", "host", "c");
    assert!(ids.contains(&c), "isolated host:c must be in coverage gap");

    // Hosts a and b do RUNS → service; they must not appear.
    let a = EntityId::derive("acme", "host", "a");
    let b = EntityId::derive("acme", "host", "b");
    assert!(!ids.contains(&a));
    assert!(!ids.contains(&b));
}
