//! Coverage gap detection — find entities missing an expected security control.
//!
//! Example: "Which hosts have NO EDR agent protecting them?"
//!
//! Implementation: set difference using the type/class index + adjacency check.
//! O(|candidates| × avg_degree) — fast because adjacency is O(1) per entity.
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.8

use parallax_core::{
    entity::{Entity, EntityClass, EntityType},
    relationship::{Direction, RelationshipClass},
};
use parallax_store::Snapshot;

use crate::finder::{EntityFinder, PropertyFilter};

/// Builder for coverage gap queries.
///
/// Finds entities of a target type/class that do NOT have an expected
/// relationship to a neighbor of a specific type.
pub struct CoverageGapBuilder<'snap> {
    snapshot: &'snap Snapshot,
    target_type: Option<EntityType>,
    target_class: Option<EntityClass>,
    target_property_filters: Vec<PropertyFilter>,
    expected_edge_class: RelationshipClass,
    expected_neighbor_type: Option<EntityType>,
    expected_neighbor_class: Option<EntityClass>,
    expected_direction: Direction,
}

impl<'snap> CoverageGapBuilder<'snap> {
    pub(crate) fn new(snapshot: &'snap Snapshot, edge_class: &str) -> Self {
        CoverageGapBuilder {
            snapshot,
            target_type: None,
            target_class: None,
            target_property_filters: Vec::new(),
            expected_edge_class: RelationshipClass::new_unchecked(edge_class),
            expected_neighbor_type: None,
            expected_neighbor_class: None,
            expected_direction: Direction::Outgoing,
        }
    }

    /// Target entities of this type (e.g., `"aws_ec2_instance"`).
    pub fn target_type(mut self, t: &str) -> Self {
        self.target_type = Some(EntityType::new_unchecked(t));
        self
    }

    /// Target entities of this class (e.g., `"Host"`).
    pub fn target_class(mut self, class: &str) -> Self {
        self.target_class = Some(EntityClass::new_unchecked(class));
        self
    }

    /// Add a property filter to narrow the target set.
    pub fn target_filter(mut self, f: PropertyFilter) -> Self {
        self.target_property_filters.push(f);
        self
    }

    /// Expected neighbor entity type (e.g., `"edr_agent"`).
    pub fn neighbor_type(mut self, t: &str) -> Self {
        self.expected_neighbor_type = Some(EntityType::new_unchecked(t));
        self
    }

    /// Expected neighbor entity class.
    pub fn neighbor_class(mut self, class: &str) -> Self {
        self.expected_neighbor_class = Some(EntityClass::new_unchecked(class));
        self
    }

    /// Direction of the expected edge (default: Outgoing).
    pub fn direction(mut self, dir: Direction) -> Self {
        self.expected_direction = dir;
        self
    }

    /// Execute and return all entities that LACK the expected relationship.
    pub fn find(self) -> Vec<&'snap Entity> {
        // 1. Gather candidates using type/class index.
        let mut finder = EntityFinder::new_untyped(self.snapshot);
        if let Some(ref t) = self.target_type {
            finder = EntityFinder::new(self.snapshot, t.as_str());
        }
        if let Some(ref c) = self.target_class {
            finder = finder.class(c.as_str());
        }
        for f in self.target_property_filters.iter().cloned() {
            finder = finder.has_filter(f);
        }
        let candidates = finder.collect();

        // 2. For each candidate, check adjacency for the expected relationship.
        candidates
            .into_iter()
            .filter(|entity| !self.has_expected_relationship(entity))
            .collect()
    }

    /// Returns `true` if the entity has at least one edge satisfying the
    /// expected relationship constraints.
    fn has_expected_relationship(&self, entity: &Entity) -> bool {
        for adj in self.snapshot.adjacency(entity.id) {
            // Direction check.
            if !self.expected_direction.matches(adj.direction) {
                continue;
            }
            // Edge class check.
            let rel = match self.snapshot.get_relationship(adj.relationship_id) {
                Some(r) => r,
                None => continue,
            };
            if rel._class != self.expected_edge_class {
                continue;
            }
            // Neighbor type/class check (if specified).
            if self.expected_neighbor_type.is_some() || self.expected_neighbor_class.is_some() {
                let neighbor = match self.snapshot.get_entity(adj.neighbor_id) {
                    Some(e) => e,
                    None => continue,
                };
                if let Some(ref t) = self.expected_neighbor_type {
                    if &neighbor._type != t {
                        continue;
                    }
                }
                if let Some(ref c) = self.expected_neighbor_class {
                    if &neighbor._class != c {
                        continue;
                    }
                }
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;

    #[test]
    fn finds_unprotected_hosts() {
        // h1 has an EDR agent, h2 does not.
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1")
                .host("a", "h2")
                .entity("a", "edr_agent", "SecurityTool", "edr1")
                .rel("a", "host", "h1", "PROTECTS", "edr_agent", "edr1");
        });
        let snap = engine.snapshot();

        let unprotected = CoverageGapBuilder::new(&snap, "PROTECTS")
            .target_type("host")
            .neighbor_type("edr_agent")
            .find();

        assert_eq!(unprotected.len(), 1);
        assert_eq!(
            unprotected[0].id,
            parallax_core::entity::EntityId::derive("a", "host", "h2")
        );
    }

    #[test]
    fn all_protected_returns_empty() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1")
                .entity("a", "edr_agent", "SecurityTool", "edr1")
                .rel("a", "host", "h1", "PROTECTS", "edr_agent", "edr1");
        });
        let snap = engine.snapshot();

        let unprotected = CoverageGapBuilder::new(&snap, "PROTECTS")
            .target_type("host")
            .find();

        assert!(unprotected.is_empty());
    }
}
