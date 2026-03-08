//! Policy evaluator — runs rules against graph snapshots.
//!
//! **Spec reference:** `specs/08-policy-engine.md` §8.4
//!
//! INV-P01: Rule evaluation is atomic with respect to a snapshot.
//! INV-P02: Rule evaluation is read-only.
//! INV-P03: A failed rule doesn't prevent other rules from evaluating.

use std::time::{Duration, Instant};

use compact_str::CompactString;
use parallax_core::{
    entity::{EntityId, EntityType},
    timestamp::Timestamp,
};
use parallax_graph::GraphReader;
use parallax_query::{execute, parse, plan, ExecError, IndexStats, QueryLimits, QueryResult};
use tracing::warn;

use crate::rule::{PolicyError, PolicyRule};

/// Evaluates policy rules against a graph snapshot (INV-P01, INV-P02).
///
/// Holds a pre-loaded and validated set of rules.
#[derive(Debug)]
pub struct PolicyEvaluator {
    rules: Vec<(PolicyRule, parallax_query::Query, parallax_query::QueryPlan)>,
}

impl PolicyEvaluator {
    /// Load rules, validating PQL at load time (INV-P06).
    ///
    /// Returns an error if any rule's PQL fails to parse or plan.
    pub fn load(rules: Vec<PolicyRule>, stats: &IndexStats) -> Result<Self, PolicyError> {
        let mut validated = Vec::with_capacity(rules.len());
        for rule in rules {
            let query = parse(&rule.query).map_err(|e| PolicyError::InvalidQuery {
                rule_id: rule.id.clone(),
                parse_error: e.to_string(),
            })?;
            let query_plan = plan(query.clone(), stats).map_err(|e| PolicyError::InvalidQuery {
                rule_id: rule.id.clone(),
                parse_error: e.to_string(),
            })?;
            validated.push((rule, query, query_plan));
        }
        Ok(PolicyEvaluator { rules: validated })
    }

    /// Evaluate all enabled rules against the current graph snapshot (sequential).
    ///
    /// INV-P03: Errors in individual rules are captured; evaluation continues.
    pub fn evaluate_all<'snap>(
        &self,
        graph: &GraphReader<'snap>,
        limits: QueryLimits,
    ) -> Vec<RuleResult> {
        self.rules
            .iter()
            .map(|(rule, _q, plan)| evaluate_one(rule, plan, graph, limits.clone()))
            .collect()
    }

    /// Evaluate all enabled rules concurrently using scoped OS threads (3E).
    ///
    /// Each rule runs on its own thread; results are collected in rule-definition
    /// order. `GraphReader<'snap>` must be `Sync` (it is, since it holds only
    /// a shared reference to a thread-safe Snapshot).
    ///
    /// INV-P01: All threads read the same snapshot — evaluation is atomic.
    /// INV-P03: Each thread captures errors independently.
    pub fn par_evaluate_all<'snap>(&self, graph: &GraphReader<'snap>, limits: QueryLimits) -> Vec<RuleResult>
    where
        GraphReader<'snap>: Sync,
    {
        let n = self.rules.len();
        let slots: std::sync::Mutex<Vec<Option<RuleResult>>> =
            std::sync::Mutex::new((0..n).map(|_| None).collect());

        std::thread::scope(|s| {
            for (idx, (rule, _q, plan)) in self.rules.iter().enumerate() {
                let limits = limits.clone();
                let slots = &slots;
                s.spawn(move || {
                    let result = evaluate_one(rule, plan, graph, limits);
                    slots.lock().expect("slots lock")[idx] = Some(result);
                });
            }
        });

        slots.into_inner().expect("slots lock").into_iter().flatten().collect()
    }
}

fn evaluate_one<'snap>(
    rule: &PolicyRule,
    plan: &parallax_query::QueryPlan,
    graph: &GraphReader<'snap>,
    limits: QueryLimits,
) -> RuleResult {
    if !rule.enabled {
        return RuleResult {
            rule_id: rule.id.clone(),
            status: RuleStatus::Skipped,
            violations: vec![],
            error: None,
            evaluated_at: Timestamp::now(),
            duration: Duration::ZERO,
        };
    }

    let start = Instant::now();
    let exec_result = match execute(plan, graph, limits) {
        Ok(qr) => violations_from_result(qr, rule),
        Err(e) => {
            warn!(rule_id = %rule.id, error = %e, "Rule evaluation failed");
            Err(e)
        }
    };
    let duration = start.elapsed();

    match exec_result {
        Ok(violations) => {
            let status = if violations.is_empty() { RuleStatus::Pass } else { RuleStatus::Fail };
            RuleResult {
                rule_id: rule.id.clone(),
                status,
                violations,
                error: None,
                evaluated_at: Timestamp::now(),
                duration,
            }
        }
        Err(e) => RuleResult {
            rule_id: rule.id.clone(),
            status: RuleStatus::Error,
            violations: vec![],
            error: Some(e.to_string()),
            evaluated_at: Timestamp::now(),
            duration,
        },
    }
}

fn violations_from_result<'snap>(
    result: QueryResult<'snap>,
    rule: &PolicyRule,
) -> Result<Vec<Violation>, ExecError> {
    Ok(match result {
        QueryResult::Entities(entities) => entities
            .iter()
            .map(|e| Violation {
                entity_id: e.id,
                entity_type: e._type.clone(),
                display_name: e.display_name.clone(),
                details: format!("{} violates rule '{}'", e.display_name, rule.name),
            })
            .collect(),
        QueryResult::Scalar(0) => vec![],
        QueryResult::Scalar(n) => vec![Violation {
            entity_id: EntityId::default(),
            entity_type: EntityType::new_unchecked("aggregate"),
            display_name: CompactString::new(""),
            details: format!("{n} entities violate rule '{}'", rule.name),
        }],
        _ => vec![],
    })
}

/// Result of evaluating a single rule.
#[derive(Debug)]
pub struct RuleResult {
    pub rule_id: String,
    pub status: RuleStatus,
    pub violations: Vec<Violation>,
    /// Set on RuleStatus::Error.
    pub error: Option<String>,
    pub evaluated_at: Timestamp,
    pub duration: Duration,
}

impl RuleResult {
    pub fn is_pass(&self) -> bool {
        matches!(self.status, RuleStatus::Pass)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self.status, RuleStatus::Fail)
    }
}

/// Status of a rule evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleStatus {
    Pass,
    Fail,
    Error,
    Skipped,
}

/// A single entity that violates a rule.
#[derive(Debug, Clone)]
pub struct Violation {
    pub entity_id: EntityId,
    pub entity_type: EntityType,
    pub display_name: CompactString,
    pub details: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use parallax_core::{
        entity::{Entity, EntityClass, EntityId, EntityType},
        property::Value,
        source::SourceTag,
        timestamp::Timestamp as Ts,
    };
    use parallax_query::IndexStats;
    use parallax_store::{StoreConfig, StorageEngine, WriteBatch};
    use compact_str::CompactString;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn open_engine() -> (StorageEngine, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let engine = StorageEngine::open(StoreConfig::new(dir.path())).expect("open");
        (engine, dir)
    }

    fn add_host(batch: &mut WriteBatch, key: &str, props: Vec<(&str, Value)>) {
        let id = EntityId::derive("a", "host", key);
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(CompactString::new(k), v);
        }
        batch.upsert_entity(Entity {
            id,
            _type: EntityType::new_unchecked("host"),
            _class: EntityClass::new_unchecked("Host"),
            display_name: CompactString::new(key),
            properties,
            source: SourceTag::default(),
            created_at: Ts::default(),
            updated_at: Ts::default(),
            _deleted: false,
        });
    }

    fn stats() -> IndexStats {
        let mut tc = std::collections::HashMap::new();
        tc.insert("host".into(), 10);
        tc.insert("edr_agent".into(), 3);
        let mut cc = std::collections::HashMap::new();
        cc.insert("Host".into(), 10);
        IndexStats::new(tc, cc, 13, 0)
    }

    #[test]
    fn rule_load_invalid_pql_fails() {
        let rule = PolicyRule::new("r1", "Bad Rule", "INVALID SYNTAX HERE");
        let result = PolicyEvaluator::load(vec![rule], &stats());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PolicyError::InvalidQuery { .. }));
    }

    #[test]
    fn evaluate_pass_when_no_violations() {
        let (mut engine, _dir) = open_engine();
        let mut batch = WriteBatch::new();
        // host with active = true → no violations for "find inactive hosts"
        add_host(&mut batch, "h1", vec![("active", Value::Bool(true))]);
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);

        let rule = PolicyRule::new(
            "inactive-hosts",
            "Hosts must be active",
            "FIND host WITH active = false",
        );
        let evaluator = PolicyEvaluator::load(vec![rule], &stats()).unwrap();
        let results = evaluator.evaluate_all(&graph, QueryLimits::default());

        assert_eq!(results.len(), 1);
        assert!(results[0].is_pass());
        assert!(results[0].violations.is_empty());
    }

    #[test]
    fn evaluate_fail_with_violations() {
        let (mut engine, _dir) = open_engine();
        let mut batch = WriteBatch::new();
        add_host(&mut batch, "bad-host", vec![("active", Value::Bool(false))]);
        add_host(&mut batch, "good-host", vec![("active", Value::Bool(true))]);
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);

        let rule = PolicyRule::new(
            "inactive-hosts",
            "Hosts must be active",
            "FIND host WITH active = false",
        );
        let evaluator = PolicyEvaluator::load(vec![rule], &stats()).unwrap();
        let results = evaluator.evaluate_all(&graph, QueryLimits::default());

        assert_eq!(results.len(), 1);
        assert!(results[0].is_fail());
        assert_eq!(results[0].violations.len(), 1);
        assert_eq!(results[0].violations[0].display_name.as_str(), "bad-host");
    }

    #[test]
    fn skipped_rule_not_evaluated() {
        let (engine, _dir) = open_engine();
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);

        let mut rule = PolicyRule::new("skip-me", "Skipped", "FIND host");
        rule.enabled = false;

        let evaluator = PolicyEvaluator::load(vec![rule], &stats()).unwrap();
        let results = evaluator.evaluate_all(&graph, QueryLimits::default());
        assert_eq!(results[0].status, RuleStatus::Skipped);
    }

    #[test]
    fn par_evaluate_all_matches_sequential() {
        let (mut engine, _dir) = open_engine();
        let mut batch = WriteBatch::new();
        add_host(&mut batch, "h1", vec![("active", Value::Bool(false))]);
        add_host(&mut batch, "h2", vec![("active", Value::Bool(true))]);
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);

        let rules = vec![
            PolicyRule::new("r1", "Inactive hosts", "FIND host WITH active = false"),
            PolicyRule::new("r2", "All hosts", "FIND host"),
        ];
        let evaluator = PolicyEvaluator::load(rules, &stats()).unwrap();
        let seq = evaluator.evaluate_all(&graph, QueryLimits::default());
        let par = evaluator.par_evaluate_all(&graph, QueryLimits::default());

        assert_eq!(seq.len(), par.len());
        for (s, p) in seq.iter().zip(par.iter()) {
            assert_eq!(s.rule_id, p.rule_id);
            assert_eq!(s.status, p.status);
            assert_eq!(s.violations.len(), p.violations.len());
        }
    }

    #[test]
    fn multiple_rules_independent() {
        let (mut engine, _dir) = open_engine();
        let mut batch = WriteBatch::new();
        add_host(&mut batch, "h1", vec![("active", Value::Bool(false))]);
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);

        let rules = vec![
            PolicyRule::new("r1", "Active hosts", "FIND host WITH active = false"),
            PolicyRule::new("r2", "All hosts", "FIND host"),
        ];
        let evaluator = PolicyEvaluator::load(rules, &stats()).unwrap();
        let results = evaluator.evaluate_all(&graph, QueryLimits::default());

        assert_eq!(results.len(), 2);
        assert!(results[0].is_fail()); // r1: finds the inactive host
        assert!(results[1].is_fail()); // r2: finds all hosts (they're violations)
    }
}
