//! GraphReader — the entry point for all graph operations.
//!
//! `GraphReader<'snap>` borrows a snapshot and exposes every graph-aware
//! operation as a fluent builder. The lifetime `'snap` ties all returned
//! references to the snapshot — the borrow checker ensures zero stale reads
//! and zero use-after-free (INV-G05).
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.2

use parallax_core::{
    entity::{Entity, EntityId},
    relationship::{Relationship, RelationshipId},
};
use parallax_store::Snapshot;

use crate::blast::BlastRadiusBuilder;
use crate::coverage::CoverageGapBuilder;
use crate::finder::EntityFinder;
use crate::path::ShortestPathBuilder;
use crate::pattern::PatternBuilder;
use crate::traversal::TraversalBuilder;

/// Graph-aware read operations over an MVCC snapshot.
///
/// Lifetime `'snap` ensures no entity reference outlives its snapshot.
///
/// INV-G05: `GraphReader<'snap>` cannot outlive its `Snapshot`.
/// INV-G06: All operations are read-only. No mutation through GraphReader.
pub struct GraphReader<'snap> {
    snapshot: &'snap Snapshot,
}

impl<'snap> GraphReader<'snap> {
    /// Create a GraphReader bound to the given snapshot.
    pub fn new(snapshot: &'snap Snapshot) -> Self {
        GraphReader { snapshot }
    }

    /// Start an entity finder with no type or class constraint (full scan, logged at WARN).
    pub fn find_all(&self) -> EntityFinder<'snap> {
        EntityFinder::new_untyped(self.snapshot)
    }

    /// Start an entity finder scoped to a specific entity class.
    pub fn find_by_class(&self, class: &str) -> EntityFinder<'snap> {
        EntityFinder::new_untyped(self.snapshot).class(class)
    }

    /// Start an entity finder scoped to a specific entity type.
    ///
    /// ```rust,ignore
    /// graph.find("aws_ec2_instance").with("state", "running").collect()
    /// ```
    pub fn find(&self, entity_type: &str) -> EntityFinder<'snap> {
        EntityFinder::new(self.snapshot, entity_type)
    }

    /// Start a traversal from a specific entity.
    ///
    /// ```rust,ignore
    /// graph.traverse(id).direction(Direction::Incoming).max_depth(3).collect()
    /// ```
    pub fn traverse(&self, start: EntityId) -> TraversalBuilder<'snap> {
        TraversalBuilder::new(self.snapshot, start)
    }

    /// Start a structural pattern match.
    ///
    /// ```rust,ignore
    /// graph.pattern()
    ///     .node("u", "User")
    ///     .edge("ASSIGNED")
    ///     .node("r", "AccessRole")
    ///     .execute()
    /// ```
    pub fn pattern(&self) -> PatternBuilder<'snap> {
        PatternBuilder::new(self.snapshot)
    }

    /// Find shortest path between two entities (bidirectional BFS).
    pub fn shortest_path(&self, from: EntityId, to: EntityId) -> ShortestPathBuilder<'snap> {
        ShortestPathBuilder::new(self.snapshot, from, to)
    }

    /// Start a blast radius analysis from a (potentially compromised) entity.
    pub fn blast_radius(&self, origin: EntityId) -> BlastRadiusBuilder<'snap> {
        BlastRadiusBuilder::new(self.snapshot, origin)
    }

    /// Start a coverage gap query: find entities LACKING an expected relationship.
    pub fn coverage_gap(&self, expected_edge_class: &str) -> CoverageGapBuilder<'snap> {
        CoverageGapBuilder::new(self.snapshot, expected_edge_class)
    }

    /// Direct entity lookup by ID.
    pub fn get_entity(&self, id: EntityId) -> Option<&'snap Entity> {
        self.snapshot.get_entity(id)
    }

    /// Direct relationship lookup by ID.
    pub fn get_relationship(&self, id: RelationshipId) -> Option<&'snap Relationship> {
        self.snapshot.get_relationship(id)
    }

    /// Count all live entities of the given type.
    pub fn count_by_type(&self, entity_type: &str) -> usize {
        self.snapshot
            .entities_of_type(&parallax_core::entity::EntityType::new_unchecked(
                entity_type,
            ))
            .len()
    }

    /// Total number of live entities in this snapshot.
    pub fn total_entities(&self) -> usize {
        self.snapshot.entity_count()
    }

    /// Total number of live relationships in this snapshot.
    pub fn total_relationships(&self) -> usize {
        self.snapshot.relationship_count()
    }
}

// GraphReader is explicitly read-only and borrows the snapshot.
// INV-G06: no &mut self methods.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;
    use parallax_core::relationship::Direction;

    #[test]
    fn reader_find_delegates_to_finder() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1").host("a", "h2").service("a", "s1");
        });
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        assert_eq!(graph.find("host").collect().len(), 2);
        assert_eq!(graph.count_by_type("service"), 1);
        assert_eq!(graph.total_entities(), 3);
    }

    #[test]
    fn reader_get_entity() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1");
        });
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let id = EntityId::derive("a", "host", "h1");
        assert!(graph.get_entity(id).is_some());
        assert!(graph.get_entity(EntityId::default()).is_none());
    }

    #[test]
    fn reader_traverse() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .rel("a", "host", "A", "CONNECTS", "host", "B");
        });
        let snap = engine.snapshot();
        let graph = GraphReader::new(&snap);
        let id = EntityId::derive("a", "host", "A");
        let results = graph.traverse(id).direction(Direction::Outgoing).collect();
        assert_eq!(results.len(), 1);
    }
}
