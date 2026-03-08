//! Pattern matching — find subgraphs that match a structural template.
//!
//! A pattern is an alternating sequence of (Node, Edge, Node, Edge, ..., Node)
//! steps. The matcher starts from the most selective node in the pattern
//! (estimated by entity count) and expands using nested-loop join.
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.5
//!
//! INV-G04: All returned paths satisfy ALL node and edge constraints.

use compact_str::CompactString;
use parallax_core::{
    entity::{EntityClass, EntityType},
    property::Value,
    relationship::{Direction, RelationshipClass},
};
use parallax_store::Snapshot;

use crate::finder::PropertyFilter;
use crate::traversal::{GraphPath, PathSegment};

/// One step in a pattern — either a node constraint or an edge constraint.
#[derive(Debug, Clone)]
pub enum PatternStep {
    /// A node (entity) to match.
    Node {
        alias: Option<CompactString>,
        type_filter: Option<EntityType>,
        class_filter: Option<EntityClass>,
        property_filters: Vec<PropertyFilter>,
    },
    /// An edge (relationship) to traverse.
    Edge {
        class_filter: Option<RelationshipClass>,
        direction: Direction,
        property_filters: Vec<PropertyFilter>,
    },
}

impl PatternStep {
    fn node_matches_entity(&self, entity: &parallax_core::entity::Entity) -> bool {
        if let PatternStep::Node {
            type_filter,
            class_filter,
            property_filters,
            ..
        } = self
        {
            if let Some(t) = type_filter {
                if &entity._type != t {
                    return false;
                }
            }
            if let Some(c) = class_filter {
                if &entity._class != c {
                    return false;
                }
            }
            property_filters.iter().all(|f| f.matches(entity))
        } else {
            false
        }
    }
}

/// Fluent builder for structural pattern matching.
///
/// Pattern syntax: node → edge → node → edge → ... → node
///
/// ```rust,ignore
/// graph.pattern()
///     .node("u", "User")
///     .edge("ASSIGNED")
///     .node("r", "AccessRole")
///     .edge("ALLOWS")
///     .node("b", "aws_s3_bucket")
///     .with("b", "public", true)
///     .execute()
/// ```
pub struct PatternBuilder<'snap> {
    snapshot: &'snap Snapshot,
    steps: Vec<PatternStep>,
}

impl<'snap> PatternBuilder<'snap> {
    pub(crate) fn new(snapshot: &'snap Snapshot) -> Self {
        PatternBuilder {
            snapshot,
            steps: Vec::new(),
        }
    }

    /// Add a node constraint to the pattern.
    ///
    /// `class_or_type`: if it contains `_`, treated as entity type;
    /// otherwise treated as entity class.
    pub fn node(mut self, alias: &str, class_or_type: &str) -> Self {
        let step = if class_or_type.contains('_') {
            PatternStep::Node {
                alias: Some(alias.into()),
                type_filter: Some(EntityType::new_unchecked(class_or_type)),
                class_filter: None,
                property_filters: vec![],
            }
        } else {
            PatternStep::Node {
                alias: Some(alias.into()),
                type_filter: None,
                class_filter: Some(EntityClass::new_unchecked(class_or_type)),
                property_filters: vec![],
            }
        };
        self.steps.push(step);
        self
    }

    /// Add a node constraint with explicit type.
    pub fn node_type(mut self, alias: &str, entity_type: &str) -> Self {
        self.steps.push(PatternStep::Node {
            alias: Some(alias.into()),
            type_filter: Some(EntityType::new_unchecked(entity_type)),
            class_filter: None,
            property_filters: vec![],
        });
        self
    }

    /// Add a node constraint with explicit class.
    pub fn node_class(mut self, alias: &str, class: &str) -> Self {
        self.steps.push(PatternStep::Node {
            alias: Some(alias.into()),
            type_filter: None,
            class_filter: Some(EntityClass::new_unchecked(class)),
            property_filters: vec![],
        });
        self
    }

    /// Add an outgoing edge constraint.
    pub fn edge(mut self, class: &str) -> Self {
        self.steps.push(PatternStep::Edge {
            class_filter: Some(RelationshipClass::new_unchecked(class)),
            direction: Direction::Outgoing,
            property_filters: vec![],
        });
        self
    }

    /// Add an incoming edge constraint.
    pub fn edge_incoming(mut self, class: &str) -> Self {
        self.steps.push(PatternStep::Edge {
            class_filter: Some(RelationshipClass::new_unchecked(class)),
            direction: Direction::Incoming,
            property_filters: vec![],
        });
        self
    }

    /// Add a property filter to the most recently named node.
    pub fn with(mut self, alias: &str, key: &str, value: impl Into<Value>) -> Self {
        let filter = PropertyFilter::Eq(key.into(), value.into());
        for step in self.steps.iter_mut().rev() {
            if let PatternStep::Node {
                alias: Some(a),
                property_filters,
                ..
            } = step
            {
                if a.as_str() == alias {
                    property_filters.push(filter);
                    return self;
                }
            }
        }
        self
    }

    /// Execute the pattern match and return all matching paths.
    ///
    /// Algorithm: nested-loop join starting from the first node step.
    /// For v0.1, no selectivity estimation — always starts from step 0.
    ///
    /// INV-G04: Only paths satisfying all constraints are returned.
    pub fn execute(self) -> Vec<GraphPath<'snap>> {
        let mut results = Vec::new();
        if self.steps.is_empty() {
            return results;
        }

        // First step must be a Node.
        let first_node = match &self.steps[0] {
            PatternStep::Node { .. } => &self.steps[0],
            _ => return results,
        };

        // Collect starting candidates matching first node.
        let snapshot = self.snapshot;
        let candidates: Vec<_> = snapshot
            .all_entities()
            .into_iter()
            .filter(|e| first_node.node_matches_entity(e))
            .collect();

        for start in candidates {
            expand_pattern(snapshot, start, &self.steps[1..], vec![], &mut results);
        }

        results
    }
}

/// Recursive nested-loop join for pattern expansion.
///
/// `steps` is the remaining pattern: [Edge, Node, Edge, Node, ...]
/// `path` is the path accumulated so far from the original start entity.
fn expand_pattern<'snap>(
    snapshot: &'snap Snapshot,
    current: &'snap parallax_core::entity::Entity,
    steps: &[PatternStep],
    path: Vec<PathSegment<'snap>>,
    results: &mut Vec<GraphPath<'snap>>,
) {
    if steps.is_empty() {
        // Complete match — add path to results.
        results.push(GraphPath { segments: path });
        return;
    }

    // steps[0] must be Edge, steps[1] must be Node.
    if steps.len() < 2 {
        return;
    }
    let (edge_step, node_step, rest) = match (&steps[0], &steps[1]) {
        (e @ PatternStep::Edge { .. }, n @ PatternStep::Node { .. }) => (e, n, &steps[2..]),
        _ => return,
    };

    let (edge_class_filter, edge_direction) = match edge_step {
        PatternStep::Edge {
            class_filter,
            direction,
            ..
        } => (class_filter, direction),
        _ => return,
    };

    for adj in snapshot.adjacency(current.id) {
        // Direction filter.
        if !edge_direction.matches(adj.direction) {
            continue;
        }

        // Edge class filter.
        let rel = match snapshot.get_relationship(adj.relationship_id) {
            Some(r) => r,
            None => continue,
        };
        if let Some(cls) = edge_class_filter {
            if &rel._class != cls {
                continue;
            }
        }

        // Node filter for the destination.
        let neighbor = match snapshot.get_entity(adj.neighbor_id) {
            Some(e) => e,
            None => continue,
        };
        if !node_step.node_matches_entity(neighbor) {
            continue;
        }

        // Extend path and recurse.
        let mut new_path = path.clone();
        new_path.push(PathSegment {
            relationship: rel,
            entity: neighbor,
        });
        expand_pattern(snapshot, neighbor, rest, new_path, results);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;

    #[test]
    fn two_hop_pattern() {
        // User -[ASSIGNED]-> Role -[ALLOWS]-> Bucket
        let (engine, _dir) = make_graph(|b| {
            b.entity("a", "user", "User", "u1")
                .entity("a", "access_role", "AccessRole", "r1")
                .entity("a", "aws_s3_bucket", "DataStore", "bkt1")
                .rel("a", "user", "u1", "ASSIGNED", "access_role", "r1")
                .rel("a", "access_role", "r1", "ALLOWS", "aws_s3_bucket", "bkt1");
        });
        let snap = engine.snapshot();
        let paths = PatternBuilder::new(&snap)
            .node("u", "User")
            .edge("ASSIGNED")
            .node("r", "AccessRole")
            .edge("ALLOWS")
            .node_type("b", "aws_s3_bucket")
            .execute();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].segments.len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1").host("a", "h2");
        });
        let snap = engine.snapshot();
        let paths = PatternBuilder::new(&snap)
            .node("u", "User")
            .edge("ASSIGNED")
            .node("r", "AccessRole")
            .execute();
        assert!(paths.is_empty());
    }

    #[test]
    fn property_filter_in_pattern() {
        let (engine, _dir) = make_graph(|b| {
            b.entity("a", "user", "User", "u1")
                .entity("a", "aws_s3_bucket", "DataStore", "pub_bkt")
                .entity("a", "aws_s3_bucket", "DataStore", "priv_bkt")
                .rel("a", "user", "u1", "ALLOWS", "aws_s3_bucket", "pub_bkt")
                .rel("a", "user", "u1", "ALLOWS", "aws_s3_bucket", "priv_bkt")
                .prop("a", "aws_s3_bucket", "pub_bkt", "public", true);
        });
        let snap = engine.snapshot();
        let paths = PatternBuilder::new(&snap)
            .node("u", "User")
            .edge("ALLOWS")
            .node_type("b", "aws_s3_bucket")
            .with("b", "public", true)
            .execute();
        assert_eq!(paths.len(), 1);
    }
}
