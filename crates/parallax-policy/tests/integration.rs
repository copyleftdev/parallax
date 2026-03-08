//! Integration tests for parallax-policy v0.2.
//!
//! Tests the full pipeline: YAML load → PQL validate → graph evaluate.
//! Covers rule loading (3C), sequential + parallel evaluation (3E), and
//! framework posture scoring.

use std::collections::BTreeMap;
use std::io::Write as _;

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    property::Value,
    source::SourceTag,
    timestamp::Timestamp,
};
use parallax_graph::GraphReader;
use parallax_policy::{
    compute_posture, load_rules_from_yaml, PolicyEvaluator, PolicyRule, RuleStatus, Schedule,
    Severity,
};
use parallax_query::{IndexStats, QueryLimits};
use parallax_store::{StorageEngine, StoreConfig, WriteBatch};
use tempfile::{NamedTempFile, TempDir};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open_engine(dir: &TempDir) -> StorageEngine {
    StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine")
}

fn host_entity(key: &str, props: &[(&str, Value)]) -> Entity {
    let mut properties = BTreeMap::new();
    for (k, v) in props {
        properties.insert(CompactString::new(*k), v.clone());
    }
    Entity {
        id: EntityId::derive("acme", "host", key),
        _type: EntityType::new_unchecked("host"),
        _class: EntityClass::new_unchecked("Host"),
        display_name: CompactString::new(key),
        properties,
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

fn service_entity(key: &str) -> Entity {
    Entity {
        id: EntityId::derive("acme", "service", key),
        _type: EntityType::new_unchecked("service"),
        _class: EntityClass::new_unchecked("Service"),
        display_name: CompactString::new(key),
        properties: BTreeMap::new(),
        source: SourceTag::default(),
        created_at: Timestamp::default(),
        updated_at: Timestamp::default(),
        _deleted: false,
    }
}

fn make_stats(type_counts: &[(&str, usize)]) -> IndexStats {
    let tc: std::collections::HashMap<String, usize> = type_counts
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    let cc = std::collections::HashMap::new();
    let total: usize = tc.values().sum();
    IndexStats::new(tc, cc, total, 0)
}

fn write_yaml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f
}

// ─── YAML rule loading (3C) ───────────────────────────────────────────────────

/// Minimal YAML rule file is parsed correctly.
#[test]
fn v02_yaml_load_minimal_rule() {
    let yaml = r#"
rules:
  - id: host-check-001
    name: All Hosts
    severity: medium
    description: "Finds all hosts"
    query: "FIND host"
    frameworks: []
    schedule: "manual"
    remediation: "N/A"
    enabled: true
"#;
    let f = write_yaml(yaml);
    let rules = load_rules_from_yaml(f.path()).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, "host-check-001");
    assert_eq!(rules[0].severity, Severity::Medium);
    assert!(matches!(rules[0].schedule, Schedule::Manual));
    assert!(rules[0].enabled);
}

/// Multiple rules in a single file are all loaded.
#[test]
fn v02_yaml_load_multiple_rules() {
    let yaml = r#"
rules:
  - id: r1
    name: Rule 1
    severity: high
    description: ""
    query: "FIND host"
    frameworks: []
    schedule: "manual"
    remediation: ""
    enabled: true
  - id: r2
    name: Rule 2
    severity: critical
    description: ""
    query: "FIND service"
    frameworks: []
    schedule: "every:5m"
    remediation: ""
    enabled: false
  - id: r3
    name: Rule 3
    severity: low
    description: ""
    query: "FIND host"
    frameworks:
      - framework: CIS-Controls-v8
        control: "10.1"
    schedule: "on_sync:aws,gcp"
    remediation: ""
    enabled: true
"#;
    let f = write_yaml(yaml);
    let rules = load_rules_from_yaml(f.path()).unwrap();

    assert_eq!(rules.len(), 3);

    // r1: high severity, manual, enabled
    assert_eq!(rules[0].severity, Severity::High);
    assert!(matches!(rules[0].schedule, Schedule::Manual));
    assert!(rules[0].enabled);

    // r2: critical, every:5m, disabled
    assert_eq!(rules[1].severity, Severity::Critical);
    assert!(matches!(rules[1].schedule, Schedule::Every(d) if d.as_secs() == 300));
    assert!(!rules[1].enabled);

    // r3: low, on_sync, 1 framework mapping
    assert!(matches!(&rules[2].schedule, Schedule::OnSync(v) if v == &["aws", "gcp"]));
    assert_eq!(rules[2].frameworks.len(), 1);
    assert_eq!(rules[2].frameworks[0].framework, "CIS-Controls-v8");
    assert_eq!(rules[2].frameworks[0].control, "10.1");
}

/// YAML parse error returns Err.
#[test]
fn v02_yaml_bad_syntax_is_err() {
    let f = write_yaml("not: [valid: yaml\n");
    assert!(load_rules_from_yaml(f.path()).is_err());
}

/// Missing file returns Err.
#[test]
fn v02_yaml_missing_file_is_err() {
    assert!(load_rules_from_yaml(std::path::Path::new("/nonexistent/path/rules.yaml")).is_err());
}

/// Duration schedule variants parse correctly.
#[test]
fn v02_yaml_schedule_duration_variants() {
    let yaml = r#"
rules:
  - id: r30s
    name: r30s
    severity: info
    description: ""
    query: "FIND host"
    frameworks: []
    schedule: "every:30s"
    remediation: ""
    enabled: true
  - id: r2h
    name: r2h
    severity: info
    description: ""
    query: "FIND host"
    frameworks: []
    schedule: "every:2h"
    remediation: ""
    enabled: true
  - id: r1d
    name: r1d
    severity: info
    description: ""
    query: "FIND host"
    frameworks: []
    schedule: "every:1d"
    remediation: ""
    enabled: true
"#;
    let f = write_yaml(yaml);
    let rules = load_rules_from_yaml(f.path()).unwrap();
    assert!(matches!(rules[0].schedule, Schedule::Every(d) if d.as_secs() == 30));
    assert!(matches!(rules[1].schedule, Schedule::Every(d) if d.as_secs() == 7200));
    assert!(matches!(rules[2].schedule, Schedule::Every(d) if d.as_secs() == 86400));
}

// ─── PQL validation at load time (INV-P06) ───────────────────────────────────

/// A rule with invalid PQL is rejected at load time, not evaluation time.
#[test]
fn v02_invalid_pql_rejected_at_load() {
    let rules = vec![PolicyRule::new("bad", "Bad PQL", "INVALID SYNTAX !!!")];
    let stats = make_stats(&[("host", 5)]);
    let result = PolicyEvaluator::load(rules, &stats);
    assert!(result.is_err(), "invalid PQL must fail at load time");
}

/// Valid rules load successfully.
#[test]
fn v02_valid_pql_loads_ok() {
    let rules = vec![
        PolicyRule::new("r1", "Find hosts", "FIND host"),
        PolicyRule::new("r2", "Find services", "FIND service"),
    ];
    let stats = make_stats(&[("host", 5), ("service", 3)]);
    assert!(PolicyEvaluator::load(rules, &stats).is_ok());
}

// ─── Sequential evaluate_all ──────────────────────────────────────────────────

/// PASS when query returns no results.
#[test]
fn v02_evaluate_pass_no_violations() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    batch.upsert_entity(host_entity("h1", &[("active", Value::Bool(true))]));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let rules = vec![PolicyRule::new(
        "inactive",
        "Inactive hosts",
        "FIND host WITH active = false",
    )];
    let stats = make_stats(&[("host", 1)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].rule_id, "inactive");
    assert!(results[0].is_pass());
    assert!(results[0].violations.is_empty());
}

/// FAIL when query returns violations.
#[test]
fn v02_evaluate_fail_with_violations() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    batch.upsert_entity(host_entity("bad", &[("active", Value::Bool(false))]));
    batch.upsert_entity(host_entity("good", &[("active", Value::Bool(true))]));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let rules = vec![PolicyRule::new(
        "inactive",
        "Inactive hosts",
        "FIND host WITH active = false",
    )];
    let stats = make_stats(&[("host", 2)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    assert_eq!(results.len(), 1);
    assert!(results[0].is_fail());
    assert_eq!(results[0].violations.len(), 1);
    assert_eq!(results[0].violations[0].display_name.as_str(), "bad");
}

/// Disabled rules are Skipped, not evaluated.
#[test]
fn v02_disabled_rule_is_skipped() {
    let dir = TempDir::new().unwrap();
    let engine = open_engine(&dir);
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let mut rule = PolicyRule::new("skip-me", "Skipped rule", "FIND host");
    rule.enabled = false;

    let stats = make_stats(&[("host", 0)]);
    let evaluator = PolicyEvaluator::load(vec![rule], &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    assert_eq!(results[0].status, RuleStatus::Skipped);
}

/// Multiple independent rules are all evaluated; a failing rule doesn't stop others (INV-P03).
#[test]
fn v02_multiple_rules_all_evaluated() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    batch.upsert_entity(host_entity("h1", &[("active", Value::Bool(false))]));
    batch.upsert_entity(service_entity("svc1"));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let rules = vec![
        PolicyRule::new(
            "host-inactive",
            "Inactive hosts",
            "FIND host WITH active = false",
        ),
        PolicyRule::new("all-services", "All services", "FIND service"),
        PolicyRule::new("all-hosts", "All hosts", "FIND host"),
    ];
    let stats = make_stats(&[("host", 1), ("service", 1)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    assert_eq!(results.len(), 3, "all 3 rules must be evaluated");
    assert!(results[0].is_fail()); // found the inactive host
    assert!(results[1].is_fail()); // found the service (violations = services found)
    assert!(results[2].is_fail()); // found the host
}

// ─── YAML load → evaluate pipeline ───────────────────────────────────────────

/// Load rules from YAML, evaluate against a real graph — full pipeline test.
#[test]
fn v02_yaml_load_then_evaluate_pipeline() {
    let yaml = r#"
rules:
  - id: edr-001
    name: Missing EDR
    severity: high
    description: "Hosts without EDR protection"
    query: "FIND host WITH active = false"
    frameworks:
      - framework: CIS-Controls-v8
        control: "10.1"
    schedule: "manual"
    remediation: "Deploy EDR agent."
    enabled: true
  - id: all-hosts
    name: Inventory Check
    severity: info
    description: "Finds all hosts"
    query: "FIND host"
    frameworks: []
    schedule: "manual"
    remediation: ""
    enabled: true
"#;
    let f = write_yaml(yaml);
    let rules = load_rules_from_yaml(f.path()).unwrap();

    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    batch.upsert_entity(host_entity("good", &[("active", Value::Bool(true))]));
    batch.upsert_entity(host_entity("bad", &[("active", Value::Bool(false))]));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let stats = make_stats(&[("host", 2)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    assert_eq!(results.len(), 2);
    // edr-001: bad host is inactive → FAIL
    let edr = results.iter().find(|r| r.rule_id == "edr-001").unwrap();
    assert!(edr.is_fail());
    assert_eq!(edr.violations.len(), 1);

    // all-hosts: finds both hosts → FAIL (any result = violation)
    let inv = results.iter().find(|r| r.rule_id == "all-hosts").unwrap();
    assert!(inv.is_fail());
}

// ─── Parallel evaluation (3E) ─────────────────────────────────────────────────

/// par_evaluate_all produces the same results as evaluate_all.
#[test]
fn v02_par_evaluate_matches_sequential() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    for i in 0..5 {
        let active = i % 2 == 0;
        batch.upsert_entity(host_entity(
            &format!("h{i}"),
            &[("active", Value::Bool(active))],
        ));
    }
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let rules = vec![
        PolicyRule::new("r1", "Inactive", "FIND host WITH active = false"),
        PolicyRule::new("r2", "All hosts", "FIND host"),
        PolicyRule::new("r3", "Active", "FIND host WITH active = true"),
    ];
    let stats = make_stats(&[("host", 5)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();

    let seq = evaluator.evaluate_all(&graph, QueryLimits::default());
    let par = evaluator.par_evaluate_all(&graph, QueryLimits::default());

    assert_eq!(seq.len(), par.len(), "same number of results");
    for (s, p) in seq.iter().zip(par.iter()) {
        assert_eq!(s.rule_id, p.rule_id, "results in same order");
        assert_eq!(s.status, p.status, "same status for {}", s.rule_id);
        assert_eq!(
            s.violations.len(),
            p.violations.len(),
            "same violation count for {}",
            s.rule_id
        );
    }
}

/// par_evaluate_all with a single rule works correctly.
#[test]
fn v02_par_evaluate_single_rule() {
    let dir = TempDir::new().unwrap();
    let engine = open_engine(&dir);
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let rules = vec![PolicyRule::new("r1", "All hosts", "FIND host")];
    let stats = make_stats(&[("host", 0)]);
    let evaluator = PolicyEvaluator::load(rules, &stats).unwrap();

    let results = evaluator.par_evaluate_all(&graph, QueryLimits::default());
    assert_eq!(results.len(), 1);
    assert!(results[0].is_pass(), "empty graph → no violations → PASS");
}

/// par_evaluate_all with empty rule list returns empty vec.
#[test]
fn v02_par_evaluate_empty_rules() {
    let dir = TempDir::new().unwrap();
    let engine = open_engine(&dir);
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let stats = make_stats(&[]);
    let evaluator = PolicyEvaluator::load(vec![], &stats).unwrap();

    let results = evaluator.par_evaluate_all(&graph, QueryLimits::default());
    assert!(results.is_empty());
}

// ─── Framework posture scoring ────────────────────────────────────────────────

/// compute_posture with all-PASS rules gives score 1.0.
#[test]
fn v02_posture_all_pass_score_one() {
    let dir = TempDir::new().unwrap();
    let engine = open_engine(&dir);
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    // Query that always passes on empty graph.
    let mut rule = PolicyRule::new("r1", "No inactive hosts", "FIND host WITH active = false");
    rule.frameworks.push(parallax_policy::FrameworkMapping {
        framework: "CIS-Controls-v8".to_owned(),
        control: "10.1".to_owned(),
    });

    let stats = make_stats(&[("host", 0)]);
    let evaluator = PolicyEvaluator::load(vec![rule.clone()], &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());
    let posture = compute_posture("CIS-Controls-v8", &[rule], &results);

    // One control, one passing rule → score = 1.0.
    assert!(
        (posture.overall_score - 1.0).abs() < f64::EPSILON,
        "all-pass → score = 1.0, got {}",
        posture.overall_score
    );
}

/// compute_posture with all-FAIL rules gives score 0.0.
#[test]
fn v02_posture_all_fail_score_zero() {
    let dir = TempDir::new().unwrap();
    let mut engine = open_engine(&dir);
    let mut batch = WriteBatch::new();
    batch.upsert_entity(host_entity("h1", &[("active", Value::Bool(false))]));
    engine.write(batch).unwrap();

    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let mut rule = PolicyRule::new("r1", "Inactive hosts", "FIND host WITH active = false");
    rule.frameworks.push(parallax_policy::FrameworkMapping {
        framework: "CIS-Controls-v8".to_owned(),
        control: "10.1".to_owned(),
    });

    let stats = make_stats(&[("host", 1)]);
    let evaluator = PolicyEvaluator::load(vec![rule.clone()], &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());
    let posture = compute_posture("CIS-Controls-v8", &[rule], &results);

    assert!(
        posture.overall_score < 0.01,
        "all-fail → score ≈ 0.0, got {}",
        posture.overall_score
    );
}

/// Posture for unknown framework has no controls.
#[test]
fn v02_posture_unknown_framework_empty_controls() {
    let dir = TempDir::new().unwrap();
    let engine = open_engine(&dir);
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);

    let mut rule = PolicyRule::new("r1", "All hosts", "FIND host");
    rule.frameworks.push(parallax_policy::FrameworkMapping {
        framework: "CIS-Controls-v8".to_owned(),
        control: "10.1".to_owned(),
    });

    let stats = make_stats(&[("host", 0)]);
    let evaluator = PolicyEvaluator::load(vec![rule.clone()], &stats).unwrap();
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());
    let posture = compute_posture("NIST-CSF", &[rule], &results);

    assert!(
        posture.controls.is_empty(),
        "no rules mapped to NIST-CSF → no controls"
    );
}
