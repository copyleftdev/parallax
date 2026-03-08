//! Integration tests for parallax-query.
//!
//! Exercises the full parse → plan → execute pipeline against a live graph.

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    property::Value,
    relationship::{Relationship, RelationshipClass, RelationshipId},
    source::SourceTag,
    timestamp::Timestamp,
};
use parallax_graph::GraphReader;
use parallax_query::{execute, parse, plan, IndexStats, QueryLimits, QueryResult};
use parallax_store::{StorageEngine, StoreConfig, WriteBatch};
use std::collections::BTreeMap;
use tempfile::TempDir;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open_engine(dir: &TempDir) -> StorageEngine {
    StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine")
}

fn entity_with_props(typ: &str, class: &str, key: &str, props: &[(&str, Value)]) -> Entity {
    let mut properties = BTreeMap::new();
    for (k, v) in props {
        properties.insert(CompactString::new(*k), v.clone());
    }
    Entity {
        id: EntityId::derive("acme", typ, key),
        _type: EntityType::new_unchecked(typ),
        _class: EntityClass::new_unchecked(class),
        display_name: CompactString::new(key),
        properties,
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

fn make_rel(
    from_type: &str,
    from_key: &str,
    verb: &str,
    to_type: &str,
    to_key: &str,
) -> Relationship {
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

fn build_stats(engine: &StorageEngine) -> IndexStats {
    let snap = engine.snapshot();
    let all = snap.all_entities();
    let mut type_counts = std::collections::HashMap::new();
    let mut class_counts = std::collections::HashMap::new();
    for e in &all {
        *type_counts
            .entry(e._type.as_str().to_owned())
            .or_insert(0usize) += 1;
        *class_counts
            .entry(e._class.as_str().to_owned())
            .or_insert(0usize) += 1;
    }
    IndexStats::new(
        type_counts,
        class_counts,
        snap.entity_count(),
        snap.relationship_count(),
    )
}

/// Returns the `count()` of the PQL result.
/// Snapshot must be kept alive by the caller (inside the test).
fn pql_count(engine: &StorageEngine, pql: &str) -> u64 {
    let stats = build_stats(engine);
    let ast = parse(pql).expect("parse");
    let qplan = plan(ast, &stats).expect("plan");
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    let result = execute(&qplan, &graph, QueryLimits::default()).expect("execute");
    result.count()
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// `FIND host` returns all hosts.
#[test]
fn pql_find_by_type() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props("host", "Host", "h1", &[]));
    batch.upsert_entity(entity_with_props("host", "Host", "h2", &[]));
    batch.upsert_entity(entity_with_props("service", "Service", "svc1", &[]));
    engine.write(batch).unwrap();

    assert_eq!(pql_count(&engine, "FIND host"), 2);
}

/// `FIND host WITH state = 'running'` filters correctly.
#[test]
fn pql_find_with_property_filter() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h1",
        &[("state", Value::from("running"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h2",
        &[("state", Value::from("stopped"))],
    ));
    engine.write(batch).unwrap();

    assert_eq!(pql_count(&engine, "FIND host WITH state = 'running'"), 1);
}

/// `FIND host LIMIT 1` respects the limit.
#[test]
fn pql_find_with_limit() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    for i in 0..5 {
        batch.upsert_entity(entity_with_props("host", "Host", &format!("h{i}"), &[]));
    }
    engine.write(batch).unwrap();

    assert_eq!(pql_count(&engine, "FIND host LIMIT 1"), 1);
}

/// `FIND host RETURN COUNT` returns a scalar of 2.
#[test]
fn pql_return_count() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props("host", "Host", "h1", &[]));
    batch.upsert_entity(entity_with_props("host", "Host", "h2", &[]));
    engine.write(batch).unwrap();

    let stats = build_stats(&engine);
    let ast = parse("FIND host RETURN COUNT").expect("parse");
    let qplan = plan(ast, &stats).expect("plan");
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    let result = execute(&qplan, &graph, QueryLimits::default()).expect("execute");
    assert!(matches!(result, QueryResult::Scalar(2)));
}

/// Traversal PQL: FIND host THAT USES service reaches svc1.
#[test]
fn pql_traversal() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props("host", "Host", "h1", &[]));
    batch.upsert_entity(entity_with_props("service", "Service", "svc1", &[]));
    batch.upsert_relationship(make_rel("host", "h1", "USES", "service", "svc1"));
    engine.write(batch).unwrap();

    let count = pql_count(&engine, "FIND host THAT USES service");
    assert!(count >= 1, "traversal must reach at least svc1");
}

/// `FIND host WITH state != 'stopped'` excludes stopped hosts.
#[test]
fn pql_ne_filter() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h1",
        &[("state", Value::from("running"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h2",
        &[("state", Value::from("stopped"))],
    ));
    engine.write(batch).unwrap();

    assert_eq!(pql_count(&engine, "FIND host WITH state != 'stopped'"), 1);
}

/// Parse error on invalid PQL returns an error.
#[test]
fn pql_parse_error() {
    assert!(parse("NOT VALID PQL !!!").is_err());
}

/// `FIND *` returns all entity types.
#[test]
fn pql_find_wildcard() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props("host", "Host", "h1", &[]));
    batch.upsert_entity(entity_with_props("service", "Service", "svc1", &[]));
    engine.write(batch).unwrap();

    assert_eq!(pql_count(&engine, "FIND *"), 2);
}

// ─── v0.2: OR filters ────────────────────────────────────────────────────────

/// `FIND host WITH os = 'linux' OR os = 'windows'` returns both matching hosts.
#[test]
fn pql_or_filter_returns_both_matching() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "linux-h",
        &[("os", Value::from("linux"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "windows-h",
        &[("os", Value::from("windows"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "macos-h",
        &[("os", Value::from("macos"))],
    ));
    engine.write(batch).unwrap();

    let count = pql_count(&engine, "FIND host WITH os = 'linux' OR os = 'windows'");
    assert_eq!(
        count, 2,
        "OR filter must return linux AND windows hosts, not macos"
    );
}

/// `OR` with a single arm behaves identically to a plain equality check.
#[test]
fn pql_or_single_arm_same_as_eq() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h1",
        &[("env", Value::from("prod"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h2",
        &[("env", Value::from("dev"))],
    ));
    engine.write(batch).unwrap();

    let eq_count = pql_count(&engine, "FIND host WITH env = 'prod'");
    let or_count = pql_count(&engine, "FIND host WITH env = 'prod' OR env = 'prod'");
    assert_eq!(
        eq_count, or_count,
        "single-armed OR must match plain equality"
    );
    assert_eq!(eq_count, 1);
}

/// OR + AND combination: `FIND host WITH (os = 'linux' OR os = 'windows') AND env = 'prod'`.
/// (The AND is implicit: multiple WITH conditions are ANDed; each OR expr is one condition.)
#[test]
fn pql_or_combined_with_and_property() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    // Should match: linux + prod
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "match",
        &[("os", Value::from("linux")), ("env", Value::from("prod"))],
    ));
    // OS matches OR but env doesn't → no match
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "no-env",
        &[("os", Value::from("windows")), ("env", Value::from("dev"))],
    ));
    // env matches but OS doesn't → no match
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "no-os",
        &[("os", Value::from("macos")), ("env", Value::from("prod"))],
    ));
    engine.write(batch).unwrap();

    let count = pql_count(
        &engine,
        "FIND host WITH os = 'linux' OR os = 'windows' AND env = 'prod'",
    );
    assert_eq!(count, 1, "only the linux+prod host matches both conditions");
}

/// OR with three alternatives.
#[test]
fn pql_or_three_alternatives() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    for region in ["us-east-1", "eu-west-1", "ap-southeast-1", "sa-east-1"] {
        batch.upsert_entity(entity_with_props(
            "host",
            "Host",
            region,
            &[("region", Value::from(region))],
        ));
    }
    engine.write(batch).unwrap();

    let count = pql_count(
        &engine,
        "FIND host WITH region = 'us-east-1' OR region = 'eu-west-1' OR region = 'ap-southeast-1'",
    );
    assert_eq!(count, 3, "three-armed OR returns exactly 3 of 4 hosts");
}

// ─── v0.2: GROUP BY ──────────────────────────────────────────────────────────

/// `FIND host GROUP BY os` returns one group per distinct OS value.
#[test]
fn pql_group_by_correct_groups() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h1",
        &[("os", Value::from("linux"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h2",
        &[("os", Value::from("linux"))],
    ));
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "h3",
        &[("os", Value::from("windows"))],
    ));
    engine.write(batch).unwrap();

    let stats = build_stats(&engine);
    let ast = parse("FIND host GROUP BY os").expect("parse");
    let qplan = plan(ast, &stats).expect("plan");
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    let result = execute(&qplan, &graph, QueryLimits::default()).expect("execute");

    let QueryResult::Grouped(groups) = result else {
        panic!("expected Grouped")
    };
    assert_eq!(groups.len(), 2, "two distinct OS values → two groups");

    let linux_count = groups
        .iter()
        .find(|(v, _)| matches!(v, Value::String(s) if s.as_str() == "linux"))
        .map(|(_, c)| *c);
    let windows_count = groups
        .iter()
        .find(|(v, _)| matches!(v, Value::String(s) if s.as_str() == "windows"))
        .map(|(_, c)| *c);

    assert_eq!(linux_count, Some(2), "linux group must have count 2");
    assert_eq!(windows_count, Some(1), "windows group must have count 1");
}

/// `FIND host GROUP BY region` — entities with no `region` property land in a group.
#[test]
fn pql_group_by_missing_field_handled() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    batch.upsert_entity(entity_with_props(
        "host",
        "Host",
        "with-region",
        &[("region", Value::from("us-east-1"))],
    ));
    batch.upsert_entity(entity_with_props("host", "Host", "without-region", &[]));
    engine.write(batch).unwrap();

    let stats = build_stats(&engine);
    let ast = parse("FIND host GROUP BY region").expect("parse");
    let qplan = plan(ast, &stats).expect("plan");
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    let result = execute(&qplan, &graph, QueryLimits::default()).expect("execute");

    let QueryResult::Grouped(groups) = result else {
        panic!("expected Grouped")
    };
    // Two groups: one for "us-east-1", one for Null (or missing)
    assert_eq!(
        groups.len(),
        2,
        "with-region and without-region are two distinct groups"
    );
}

/// `FIND host GROUP BY env RETURN COUNT` — GroupBy wraps correctly; count is total grouped records.
#[test]
fn pql_group_by_total_count_equals_entity_count() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);

    let mut batch = WriteBatch::new();
    for (i, env) in ["prod", "prod", "dev", "staging"].iter().enumerate() {
        batch.upsert_entity(entity_with_props(
            "host",
            "Host",
            &format!("h{i}"),
            &[("env", Value::from(*env))],
        ));
    }
    engine.write(batch).unwrap();

    let stats = build_stats(&engine);
    let ast = parse("FIND host GROUP BY env").expect("parse");
    let qplan = plan(ast, &stats).expect("plan");
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    let result = execute(&qplan, &graph, QueryLimits::default()).expect("execute");

    let QueryResult::Grouped(groups) = result else {
        panic!("expected Grouped")
    };
    let total: u64 = groups.iter().map(|(_, c)| c).sum();
    assert_eq!(
        total, 4,
        "sum of group counts must equal total entity count"
    );
    assert_eq!(groups.len(), 3, "prod, dev, staging = 3 distinct groups");
}
