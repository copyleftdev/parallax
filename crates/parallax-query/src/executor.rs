//! PQL executor — walks a `QueryPlan` tree, calling `parallax-graph` APIs.
//!
//! **Spec reference:** `specs/04-query-language.md` §4.6, §4.9
//!
//! INV-Q03: All operations are read-only. No mutation through PQL.
//! INV-Q04: Every query respects QueryLimits.

use std::time::{Duration, Instant};

use std::collections::HashMap;

use parallax_core::{entity::Entity, property::Value};
use parallax_graph::{GraphPath, GraphReader, TraversalResult};

use crate::error::ExecError;
use crate::planner::{IndexAccess, PlannedTraversal, QueryPlan};

// ─── Query limits ─────────────────────────────────────────────────────────────

/// Resource budget for a single query execution (INV-Q04).
#[derive(Debug, Clone)]
pub struct QueryLimits {
    pub timeout: Duration,
    pub max_entities_scanned: u64,
    pub max_edges_traversed: u64,
    pub max_results: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        QueryLimits {
            timeout: Duration::from_secs(30),
            max_entities_scanned: 1_000_000,
            max_edges_traversed: 10_000_000,
            max_results: 10_000,
        }
    }
}

// ─── Query results ────────────────────────────────────────────────────────────

/// The result of executing a `QueryPlan`.
pub enum QueryResult<'snap> {
    Entities(Vec<&'snap Entity>),
    Traversals(Vec<TraversalResult<'snap>>),
    Paths(Vec<GraphPath<'snap>>),
    Scalar(u64),
    /// GROUP BY result: sorted list of (group_key_value, entity_count) pairs.
    Grouped(Vec<(Value, u64)>),
}

impl<'snap> QueryResult<'snap> {
    pub fn count(&self) -> u64 {
        match self {
            QueryResult::Entities(v) => v.len() as u64,
            QueryResult::Traversals(v) => v.len() as u64,
            QueryResult::Paths(v) => v.len() as u64,
            QueryResult::Scalar(n) => *n,
            QueryResult::Grouped(groups) => groups.iter().map(|(_, c)| c).sum(),
        }
    }

    pub fn truncate(&mut self, n: usize) {
        match self {
            QueryResult::Entities(v) => v.truncate(n),
            QueryResult::Traversals(v) => v.truncate(n),
            QueryResult::Paths(v) => v.truncate(n),
            QueryResult::Grouped(v) => v.truncate(n),
            QueryResult::Scalar(_) => {}
        }
    }

    /// Extract the entity list, converting traversal results if needed.
    pub fn into_entities(self) -> Result<Vec<&'snap Entity>, ExecError> {
        match self {
            QueryResult::Entities(v) => Ok(v),
            QueryResult::Traversals(v) => Ok(v.into_iter().map(|r| r.entity).collect()),
            _ => Err(ExecError::TypeMismatch),
        }
    }
}

// ─── Execution context ────────────────────────────────────────────────────────

struct ExecCtx {
    limits: QueryLimits,
    start: Instant,
    entities_scanned: u64,
    edges_traversed: u64,
}

impl ExecCtx {
    fn new(limits: QueryLimits) -> Self {
        ExecCtx {
            limits,
            start: Instant::now(),
            entities_scanned: 0,
            edges_traversed: 0,
        }
    }

    fn check_timeout(&self) -> Result<(), ExecError> {
        let elapsed = self.start.elapsed();
        if elapsed > self.limits.timeout {
            Err(ExecError::Timeout {
                limit: self.limits.timeout,
                elapsed,
            })
        } else {
            Ok(())
        }
    }

    fn add_scanned(&mut self, n: u64) -> Result<(), ExecError> {
        self.entities_scanned += n;
        if self.entities_scanned > self.limits.max_entities_scanned {
            Err(ExecError::ScanLimitExceeded {
                scanned: self.entities_scanned,
                limit: self.limits.max_entities_scanned,
            })
        } else {
            Ok(())
        }
    }

    fn add_edges(&mut self, n: u64) -> Result<(), ExecError> {
        self.edges_traversed += n;
        if self.edges_traversed > self.limits.max_edges_traversed {
            Err(ExecError::TraversalLimitExceeded {
                traversed: self.edges_traversed,
                limit: self.limits.max_edges_traversed,
            })
        } else {
            Ok(())
        }
    }
}

// ─── Executor entry point ─────────────────────────────────────────────────────

/// Execute a query plan against a graph snapshot (INV-Q03, INV-Q04).
pub fn execute<'snap>(
    plan: &QueryPlan,
    graph: &GraphReader<'snap>,
    limits: QueryLimits,
) -> Result<QueryResult<'snap>, ExecError> {
    let mut ctx = ExecCtx::new(limits);
    execute_inner(plan, graph, &mut ctx)
}

fn execute_inner<'snap>(
    plan: &QueryPlan,
    graph: &GraphReader<'snap>,
    ctx: &mut ExecCtx,
) -> Result<QueryResult<'snap>, ExecError> {
    ctx.check_timeout()?;

    match plan {
        QueryPlan::IndexScan { index, filters } => {
            let entities: Vec<&'snap Entity> = match index {
                IndexAccess::TypeIndex(t) => graph.find(t.as_str()).collect(),
                IndexAccess::ClassIndex(c) => graph.find_by_class(c.as_str()).collect(),
                IndexAccess::FullScan => graph.find_all().collect(),
            };
            ctx.add_scanned(entities.len() as u64)?;

            let filtered: Vec<&'snap Entity> = entities
                .into_iter()
                .filter(|e| filters.iter().all(|f| f.matches(e)))
                .collect();

            Ok(QueryResult::Entities(filtered))
        }

        QueryPlan::Traverse { source, step } => {
            let source_entities = execute_inner(source, graph, ctx)?.into_entities()?;
            let mut results: Vec<TraversalResult<'snap>> = Vec::new();

            for entity in source_entities {
                let traversal = build_traversal(graph, entity.id, step);
                let neighbors = traversal.collect();
                ctx.add_edges(neighbors.len() as u64)?;

                for neighbor in neighbors {
                    if step.target_filter.matches(neighbor.entity)
                        && step
                            .target_property_filters
                            .iter()
                            .all(|f| f.matches(neighbor.entity))
                    {
                        results.push(neighbor);
                    }
                }
            }

            Ok(QueryResult::Traversals(results))
        }

        QueryPlan::NegatedTraverse { source, step } => {
            let source_entities = execute_inner(source, graph, ctx)?.into_entities()?;
            let mut results: Vec<&'snap Entity> = Vec::new();

            for entity in source_entities {
                let traversal = build_traversal(graph, entity.id, step);
                let neighbors = traversal.collect();
                ctx.add_edges(neighbors.len() as u64)?;

                let has_match = neighbors.iter().any(|n| {
                    step.target_filter.matches(n.entity)
                        && step
                            .target_property_filters
                            .iter()
                            .all(|f| f.matches(n.entity))
                });

                if !has_match {
                    results.push(entity);
                }
            }

            Ok(QueryResult::Entities(results))
        }

        QueryPlan::Count { source } => {
            let inner = execute_inner(source, graph, ctx)?;
            Ok(QueryResult::Scalar(inner.count()))
        }

        QueryPlan::Project { source, .. } => {
            // In v0.1, Project just passes through entities. Field projection
            // (property subsetting) is handled at the serialization layer (API / CLI).
            // Full property-level projection is deferred to v0.2.
            execute_inner(source, graph, ctx)
        }

        QueryPlan::Limit { source, n } => {
            let mut inner = execute_inner(source, graph, ctx)?;
            inner.truncate(*n);
            Ok(inner)
        }

        QueryPlan::ShortestPath {
            from,
            to,
            max_depth,
        } => {
            let from_entities = execute_inner(from, graph, ctx)?.into_entities()?;
            let to_entities = execute_inner(to, graph, ctx)?.into_entities()?;

            let from_id = from_entities
                .first()
                .ok_or(ExecError::NoMatchingEntity { side: "from" })?
                .id;
            let to_id = to_entities
                .first()
                .ok_or(ExecError::NoMatchingEntity { side: "to" })?
                .id;

            let path = graph
                .shortest_path(from_id, to_id)
                .max_depth(*max_depth)
                .find();

            match path {
                Some(p) => Ok(QueryResult::Paths(vec![p])),
                None => Ok(QueryResult::Paths(vec![])),
            }
        }

        QueryPlan::GroupBy { source, field } => {
            let entities = execute_inner(source, graph, ctx)?.into_entities()?;

            // `Value` doesn't implement `Ord` or `Hash` (Float), so we key on
            // a canonical debug string and keep the actual Value alongside.
            let mut groups: HashMap<String, (Value, u64)> = HashMap::new();
            for entity in entities {
                let val = entity
                    .properties
                    .get(field.as_str())
                    .cloned()
                    .unwrap_or(Value::Null);
                let key_str = format!("{val:?}");
                let entry = groups.entry(key_str).or_insert((val, 0));
                entry.1 += 1;
            }

            // Sort by the canonical key string for deterministic output.
            let mut pairs: Vec<(String, Value, u64)> =
                groups.into_iter().map(|(k, (v, c))| (k, v, c)).collect();
            pairs.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));

            Ok(QueryResult::Grouped(
                pairs.into_iter().map(|(_, v, c)| (v, c)).collect(),
            ))
        }

        QueryPlan::BlastRadius { origin, max_depth } => {
            let origin_entities = execute_inner(origin, graph, ctx)?.into_entities()?;
            let origin_id = origin_entities
                .first()
                .ok_or(ExecError::NoMatchingEntity { side: "origin" })?
                .id;

            let result = graph
                .blast_radius(origin_id)
                .default_rules()
                .max_depth(*max_depth)
                .analyze();
            ctx.add_edges(result.total_impacted() as u64)?;

            // Return impacted entities as Traversals.
            Ok(QueryResult::Scalar(result.total_impacted() as u64))
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn build_traversal<'snap>(
    graph: &GraphReader<'snap>,
    id: parallax_core::entity::EntityId,
    step: &PlannedTraversal,
) -> parallax_graph::TraversalBuilder<'snap> {
    let mut builder = graph.traverse(id).direction(step.direction).max_depth(1);
    if let Some(ref cls) = step.edge_class {
        builder = builder.edge_classes(&[cls.as_str()]);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::planner::{plan, IndexStats};
    use compact_str::CompactString;
    use parallax_core::{
        entity::{Entity, EntityClass, EntityId, EntityType},
        relationship::{Relationship, RelationshipClass, RelationshipId},
        source::SourceTag,
        timestamp::Timestamp,
    };
    use parallax_graph::GraphReader;
    use parallax_store::{StorageEngine, StoreConfig, WriteBatch};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn make_engine() -> (StorageEngine, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = StoreConfig::new(dir.path());
        let engine = StorageEngine::open(config).expect("open engine");
        (engine, dir)
    }

    fn add_entity(batch: &mut WriteBatch, account: &str, typ: &str, class: &str, key: &str) {
        let id = EntityId::derive(account, typ, key);
        batch.upsert_entity(Entity {
            id,
            _type: EntityType::new_unchecked(typ),
            _class: EntityClass::new_unchecked(class),
            display_name: CompactString::new(key),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        });
    }

    fn add_rel(
        batch: &mut WriteBatch,
        account: &str,
        from_t: &str,
        from_k: &str,
        cls: &str,
        to_t: &str,
        to_k: &str,
    ) {
        let from_id = EntityId::derive(account, from_t, from_k);
        let to_id = EntityId::derive(account, to_t, to_k);
        let rel_id = RelationshipId::derive(account, from_t, from_k, cls, to_t, to_k);
        batch.upsert_relationship(Relationship {
            id: rel_id,
            from_id,
            to_id,
            _class: RelationshipClass::new_unchecked(cls),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        });
    }

    fn stats_for(types: &[(&str, usize)], classes: &[(&str, usize)]) -> IndexStats {
        let type_counts = types.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let class_counts = classes.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let total = types.iter().map(|(_, v)| v).sum();
        IndexStats::new(type_counts, class_counts, total, 0)
    }

    fn run_pql<'snap>(
        pql: &str,
        graph: &GraphReader<'snap>,
        stats: &IndexStats,
    ) -> QueryResult<'snap> {
        let q = parse(pql).expect("parse");
        let p = plan(q, stats).expect("plan");
        execute(&p, graph, QueryLimits::default()).expect("execute")
    }

    #[test]
    fn execute_find_by_type() {
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();
        add_entity(&mut batch, "a", "host", "Host", "h1");
        add_entity(&mut batch, "a", "host", "Host", "h2");
        add_entity(&mut batch, "a", "service", "Service", "s1");
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(
            &[("host", 2), ("service", 1)],
            &[("Host", 2), ("Service", 1)],
        );

        let result = run_pql("FIND host", &graph, &stats);
        assert_eq!(result.count(), 2);
    }

    #[test]
    fn execute_find_with_property_filter() {
        use parallax_core::property::Value;
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();

        let mut props = BTreeMap::new();
        props.insert(CompactString::new("state"), Value::from("running"));
        let id = EntityId::derive("a", "host", "h1");
        batch.upsert_entity(Entity {
            id,
            _type: EntityType::new_unchecked("host"),
            _class: EntityClass::new_unchecked("Host"),
            display_name: CompactString::new("h1"),
            properties: props,
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        });
        add_entity(&mut batch, "a", "host", "Host", "h2");
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(&[("host", 2)], &[("Host", 2)]);

        let result = run_pql("FIND host WITH state = 'running'", &graph, &stats);
        assert_eq!(result.count(), 1);
    }

    #[test]
    fn execute_traversal() {
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();
        add_entity(&mut batch, "a", "host", "Host", "h1");
        add_entity(&mut batch, "a", "service", "Service", "s1");
        add_rel(&mut batch, "a", "host", "h1", "CONNECTS", "service", "s1");
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(
            &[("host", 1), ("service", 1)],
            &[("Host", 1), ("Service", 1)],
        );

        let result = run_pql("FIND host THAT CONNECTS service", &graph, &stats);
        assert_eq!(result.count(), 1);
    }

    #[test]
    fn execute_negated_traversal() {
        // h1 has PROTECTS edge, h2 does not.
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();
        add_entity(&mut batch, "a", "host", "Host", "h1");
        add_entity(&mut batch, "a", "host", "Host", "h2");
        add_entity(&mut batch, "a", "edr_agent", "SecurityTool", "e1");
        add_rel(&mut batch, "a", "host", "h1", "PROTECTS", "edr_agent", "e1");
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(
            &[("host", 2), ("edr_agent", 1)],
            &[("Host", 2), ("SecurityTool", 1)],
        );

        let result = run_pql("FIND host THAT !PROTECTS edr_agent", &graph, &stats);
        assert_eq!(result.count(), 1);
    }

    #[test]
    fn execute_count() {
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();
        add_entity(&mut batch, "a", "host", "Host", "h1");
        add_entity(&mut batch, "a", "host", "Host", "h2");
        add_entity(&mut batch, "a", "host", "Host", "h3");
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(&[("host", 3)], &[("Host", 3)]);

        let result = run_pql("FIND host RETURN COUNT", &graph, &stats);
        assert!(matches!(result, QueryResult::Scalar(3)));
    }

    #[test]
    fn execute_group_by() {
        use parallax_core::property::Value;
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();

        let make_host_with_region = |key: &str, region: &str| {
            let id = EntityId::derive("a", "host", key);
            let mut props = BTreeMap::new();
            props.insert(CompactString::new("region"), Value::from(region));
            Entity {
                id,
                _type: EntityType::new_unchecked("host"),
                _class: EntityClass::new_unchecked("Host"),
                display_name: CompactString::new(key),
                properties: props,
                source: SourceTag::default(),
                created_at: Timestamp::default(),
                updated_at: Timestamp::default(),
                _deleted: false,
            }
        };

        batch.upsert_entity(make_host_with_region("h1", "us-east-1"));
        batch.upsert_entity(make_host_with_region("h2", "us-east-1"));
        batch.upsert_entity(make_host_with_region("h3", "eu-west-1"));
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(&[("host", 3)], &[("Host", 3)]);

        let result = run_pql("FIND host GROUP BY region", &graph, &stats);
        if let QueryResult::Grouped(groups) = result {
            // 2 groups: us-east-1 (2), eu-west-1 (1).
            assert_eq!(groups.len(), 2);
            let total: u64 = groups.iter().map(|(_, c)| c).sum();
            assert_eq!(total, 3);
            // Both regions present.
            assert!(groups
                .iter()
                .any(|(v, c)| v.as_str() == Some("us-east-1") && *c == 2));
            assert!(groups
                .iter()
                .any(|(v, c)| v.as_str() == Some("eu-west-1") && *c == 1));
        } else {
            panic!("expected Grouped result");
        }
    }

    #[test]
    fn execute_limit() {
        let (mut engine, _dir) = make_engine();
        let mut batch = WriteBatch::new();
        for i in 0..5 {
            add_entity(&mut batch, "a", "host", "Host", &format!("h{i}"));
        }
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let stats = stats_for(&[("host", 5)], &[("Host", 5)]);

        let result = run_pql("FIND host LIMIT 3", &graph, &stats);
        assert_eq!(result.count(), 3);
    }
}
