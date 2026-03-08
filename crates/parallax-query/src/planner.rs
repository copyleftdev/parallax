//! PQL query planner — transforms an AST into an executable `QueryPlan`.
//!
//! **Spec reference:** `specs/04-query-language.md` §4.5
//!
//! INV-Q01: Every parseable query produces a valid QueryPlan.
//! INV-Q02: The planner never produces a FullScan plan without logging a warning.

use std::collections::HashMap;

use parallax_core::{
    entity::{EntityClass, EntityType},
    relationship::RelationshipClass,
};
use tracing::warn;

use crate::ast::{
    BlastQuery, EntityFilter, EntityFilterKind, FindQuery, PathQuery, PropertyCondition, Query,
    ReturnClause, TraversalStep,
};
use crate::error::PlanError;

// ─── Index statistics ─────────────────────────────────────────────────────────

/// Cardinality statistics used by the planner to choose access strategies.
#[derive(Debug, Default, Clone)]
pub struct IndexStats {
    /// Number of live entities per type name.
    pub type_counts: HashMap<String, usize>,
    /// Number of live entities per class name.
    pub class_counts: HashMap<String, usize>,
    pub total_entities: usize,
    pub total_relationships: usize,
}

impl IndexStats {
    pub fn new(
        type_counts: HashMap<String, usize>,
        class_counts: HashMap<String, usize>,
        total_entities: usize,
        total_relationships: usize,
    ) -> Self {
        IndexStats { type_counts, class_counts, total_entities, total_relationships }
    }
}

// ─── Query plan ───────────────────────────────────────────────────────────────

/// An executable query plan. Produced by the planner, consumed by the executor.
#[derive(Debug)]
pub enum QueryPlan {
    /// Scan entities using an index.
    IndexScan { index: IndexAccess, filters: Vec<PropertyCondition> },
    /// One-hop traversal from all source entities.
    Traverse { source: Box<QueryPlan>, step: PlannedTraversal },
    /// Negated traversal: keep source entities that do NOT have the expected relationship.
    NegatedTraverse { source: Box<QueryPlan>, step: PlannedTraversal },
    /// Count the results of a sub-plan.
    Count { source: Box<QueryPlan> },
    /// Project specific fields from entity results.
    Project { source: Box<QueryPlan>, fields: Vec<String> },
    /// Limit results.
    Limit { source: Box<QueryPlan>, n: usize },
    /// Bidirectional BFS between two entity sets.
    ShortestPath { from: Box<QueryPlan>, to: Box<QueryPlan>, max_depth: u32 },
    /// Blast radius from a set of origin entities.
    BlastRadius { origin: Box<QueryPlan>, max_depth: u32 },
    /// Group entities by a property field and count per group.
    GroupBy { source: Box<QueryPlan>, field: String },
}

/// Which index to use for an entity scan.
#[derive(Debug)]
pub enum IndexAccess {
    TypeIndex(EntityType),
    ClassIndex(EntityClass),
    /// Last resort — always logs a warning (INV-Q02).
    FullScan,
}

/// A single traversal hop in the plan.
#[derive(Debug)]
pub struct PlannedTraversal {
    pub direction: parallax_core::relationship::Direction,
    /// `None` = any relationship class (RELATES TO).
    pub edge_class: Option<RelationshipClass>,
    pub target_filter: EntityFilter,
    pub target_property_filters: Vec<PropertyCondition>,
}

// ─── Planner entry point ──────────────────────────────────────────────────────

/// Transform a parsed query into an executable plan.
///
/// INV-Q01: Returns `Ok` for every valid AST.
pub fn plan(query: Query, stats: &IndexStats) -> Result<QueryPlan, PlanError> {
    match query {
        Query::Find(fq) => plan_find(fq, stats),
        Query::ShortestPath(pq) => plan_path(pq, stats),
        Query::BlastRadius(bq) => plan_blast(bq, stats),
    }
}

fn plan_find(fq: FindQuery, stats: &IndexStats) -> Result<QueryPlan, PlanError> {
    let scan = QueryPlan::IndexScan {
        index: resolve_index(&fq.entity, stats),
        filters: fq.property_filters,
    };

    // Build traversal chain.
    let mut plan: QueryPlan = scan;
    for step in fq.traversals {
        plan = plan_traversal_step(plan, step, stats);
    }

    // Wrap in GroupBy before Return/Limit.
    if let Some(gb) = fq.group_by {
        plan = QueryPlan::GroupBy { source: Box::new(plan), field: gb.field };
    }

    // Wrap in Count, Project, or Limit.
    if let Some(rc) = fq.return_clause {
        plan = match rc {
            ReturnClause::Count => QueryPlan::Count { source: Box::new(plan) },
            ReturnClause::Fields(fields) => QueryPlan::Project { source: Box::new(plan), fields },
        };
    }
    if let Some(n) = fq.limit {
        plan = QueryPlan::Limit { source: Box::new(plan), n };
    }

    Ok(plan)
}

fn plan_traversal_step(
    source: QueryPlan,
    step: TraversalStep,
    stats: &IndexStats,
) -> QueryPlan {
    let edge_class = step.verb.edge_class().map(RelationshipClass::new_unchecked);
    let mut target_filter = step.target;
    // Resolve the target entity filter.
    resolve_filter_kind(&mut target_filter, stats);

    let planned_step = PlannedTraversal {
        direction: step.verb.direction(),
        edge_class,
        target_filter,
        target_property_filters: step.property_filters,
    };

    if step.negated {
        QueryPlan::NegatedTraverse { source: Box::new(source), step: planned_step }
    } else {
        QueryPlan::Traverse { source: Box::new(source), step: planned_step }
    }
}

fn plan_path(pq: PathQuery, stats: &IndexStats) -> Result<QueryPlan, PlanError> {
    let from_plan = QueryPlan::IndexScan {
        index: resolve_index(&pq.from, stats),
        filters: pq.from_filters,
    };
    let to_plan = QueryPlan::IndexScan {
        index: resolve_index(&pq.to, stats),
        filters: pq.to_filters,
    };
    Ok(QueryPlan::ShortestPath {
        from: Box::new(from_plan),
        to: Box::new(to_plan),
        max_depth: pq.max_depth.unwrap_or(10),
    })
}

fn plan_blast(bq: BlastQuery, stats: &IndexStats) -> Result<QueryPlan, PlanError> {
    let origin_plan = QueryPlan::IndexScan {
        index: resolve_index(&bq.origin, stats),
        filters: bq.origin_filters,
    };
    Ok(QueryPlan::BlastRadius {
        origin: Box::new(origin_plan),
        max_depth: bq.max_depth.unwrap_or(5),
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn resolve_index(filter: &EntityFilter, stats: &IndexStats) -> IndexAccess {
    match &filter.name {
        None => {
            // Wildcard: must full-scan.
            warn!("PQL planner: wildcard entity filter (*) triggers full entity scan");
            IndexAccess::FullScan
        }
        Some(name) => {
            let in_types = stats.type_counts.contains_key(name.as_str());
            let in_classes = stats.class_counts.contains_key(name.as_str());
            if in_types {
                IndexAccess::TypeIndex(EntityType::new_unchecked(name))
            } else if in_classes {
                IndexAccess::ClassIndex(EntityClass::new_unchecked(name))
            } else {
                // Neither found — full scan with warning (INV-Q02).
                warn!(
                    "PQL planner: entity filter '{}' not found in type or class index — \
                     performing full entity scan",
                    name
                );
                IndexAccess::FullScan
            }
        }
    }
}

fn resolve_filter_kind(filter: &mut EntityFilter, stats: &IndexStats) {
    if let Some(ref name) = filter.name.clone() {
        filter.kind = if stats.type_counts.contains_key(name.as_str()) {
            EntityFilterKind::Type(EntityType::new_unchecked(name))
        } else if stats.class_counts.contains_key(name.as_str()) {
            EntityFilterKind::Class(EntityClass::new_unchecked(name))
        } else {
            EntityFilterKind::Unresolved
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn stats_with_host_service() -> IndexStats {
        let mut type_counts = HashMap::new();
        type_counts.insert("host".into(), 10);
        type_counts.insert("service".into(), 5);
        let mut class_counts = HashMap::new();
        class_counts.insert("Host".into(), 10);
        class_counts.insert("Service".into(), 5);
        IndexStats::new(type_counts, class_counts, 15, 20)
    }

    #[test]
    fn plan_find_host_uses_type_index() {
        let q = parse("FIND host").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::IndexScan { index: IndexAccess::TypeIndex(_), .. }));
    }

    #[test]
    fn plan_find_class_uses_class_index() {
        // "Host" is in class_counts but also in type_counts... set only class.
        let mut stats = IndexStats::default();
        stats.class_counts.insert("Host".into(), 10);
        stats.total_entities = 10;
        let q = parse("FIND Host").unwrap();
        let p = plan(q, &stats).unwrap();
        assert!(matches!(p, QueryPlan::IndexScan { index: IndexAccess::ClassIndex(_), .. }));
    }

    #[test]
    fn plan_traversal_wraps_scan() {
        let q = parse("FIND host THAT CONNECTS service").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::Traverse { .. }));
    }

    #[test]
    fn plan_negated_traversal() {
        let q = parse("FIND host THAT !PROTECTS edr_agent").unwrap();
        let mut stats = stats_with_host_service();
        stats.type_counts.insert("edr_agent".into(), 3);
        let p = plan(q, &stats).unwrap();
        assert!(matches!(p, QueryPlan::NegatedTraverse { .. }));
    }

    #[test]
    fn plan_count_wraps_scan() {
        let q = parse("FIND host RETURN COUNT").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::Count { .. }));
    }

    #[test]
    fn plan_limit_outermost() {
        let q = parse("FIND host LIMIT 50").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::Limit { n: 50, .. }));
    }

    #[test]
    fn plan_shortest_path() {
        let q = parse("FIND SHORTEST PATH FROM host TO service").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::ShortestPath { max_depth: 10, .. }));
    }

    #[test]
    fn plan_blast_radius() {
        let q = parse("FIND BLAST RADIUS FROM host DEPTH 3").unwrap();
        let p = plan(q, &stats_with_host_service()).unwrap();
        assert!(matches!(p, QueryPlan::BlastRadius { max_depth: 3, .. }));
    }
}
