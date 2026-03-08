//! Graph traversal — BFS/DFS walks from a starting entity.
//!
//! `TraversalBuilder` is a fluent builder. `.collect()` or `for ... in`
//! executes the traversal lazily via `TraversalIter`.
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.4
//!
//! INV-G01: All returned entities exist in the snapshot.
//! INV-G02: With cycle_detection=true (the default), never visits the same
//!          entity twice and always terminates.

use std::collections::{HashMap, HashSet, VecDeque};

use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    relationship::{Direction, Relationship, RelationshipClass, RelationshipId},
};
use parallax_store::Snapshot;

use crate::finder::PropertyFilter;

/// Whether to expand the frontier breadth-first or depth-first.
#[derive(Debug, Clone, Copy, Default)]
pub enum TraversalStrategy {
    /// Expand all neighbors at current depth before going deeper.
    /// Best for "find all entities within N hops." (default)
    #[default]
    BreadthFirst,
    /// Follow a path as deep as possible before backtracking.
    /// Best for "find any path" or deep exploration.
    DepthFirst,
}

/// A single entity reached during traversal, with its hop depth.
pub struct TraversalResult<'snap> {
    /// The entity at this point in the traversal.
    pub entity: &'snap Entity,
    /// Hop depth from the starting entity (1 = direct neighbor).
    pub depth: u32,
    /// Path from start to this entity, reconstructed via BFS parent pointers.
    pub path: Option<GraphPath<'snap>>,
}

/// An ordered sequence of (edge, destination-entity) pairs.
#[derive(Clone)]
pub struct GraphPath<'snap> {
    pub segments: Vec<PathSegment<'snap>>,
}

/// One hop in a path: the relationship traversed and the entity reached.
#[derive(Clone)]
pub struct PathSegment<'snap> {
    pub relationship: &'snap Relationship,
    pub entity: &'snap Entity,
}

/// Fluent builder for graph traversals.
pub struct TraversalBuilder<'snap> {
    pub(crate) snapshot: &'snap Snapshot,
    pub(crate) start: parallax_core::entity::EntityId,
    pub(crate) direction: Direction,
    pub(crate) edge_classes: Option<Vec<RelationshipClass>>,
    pub(crate) node_type_filter: Option<EntityType>,
    pub(crate) node_class_filter: Option<EntityClass>,
    pub(crate) node_property_filters: Vec<PropertyFilter>,
    pub(crate) max_depth: u32,
    pub(crate) strategy: TraversalStrategy,
    pub(crate) cycle_detection: bool,
}

impl<'snap> TraversalBuilder<'snap> {
    pub(crate) fn new(snapshot: &'snap Snapshot, start: parallax_core::entity::EntityId) -> Self {
        TraversalBuilder {
            snapshot,
            start,
            direction: Direction::Outgoing,
            edge_classes: None,
            node_type_filter: None,
            node_class_filter: None,
            node_property_filters: Vec::new(),
            max_depth: 10,
            strategy: TraversalStrategy::BreadthFirst,
            cycle_detection: true,
        }
    }

    /// Set traversal direction.
    pub fn direction(mut self, dir: Direction) -> Self {
        self.direction = dir;
        self
    }

    /// Only follow edges whose class (verb) is in `classes`.
    pub fn edge_classes(mut self, classes: &[&str]) -> Self {
        self.edge_classes = Some(
            classes
                .iter()
                .map(|c| RelationshipClass::new_unchecked(c))
                .collect(),
        );
        self
    }

    /// Only *return* nodes matching this class (still traverses through others).
    pub fn filter_node_class(mut self, class: &str) -> Self {
        self.node_class_filter = Some(EntityClass::new_unchecked(class));
        self
    }

    /// Only *return* nodes matching this type (still traverses through others).
    pub fn filter_node_type(mut self, t: &str) -> Self {
        self.node_type_filter = Some(EntityType::new_unchecked(t));
        self
    }

    /// Only *return* nodes matching this property filter.
    pub fn filter_node_property(mut self, f: PropertyFilter) -> Self {
        self.node_property_filters.push(f);
        self
    }

    /// Maximum hop depth (default: 10).
    pub fn max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// BFS or DFS traversal order (default: BFS).
    pub fn strategy(mut self, s: TraversalStrategy) -> Self {
        self.strategy = s;
        self
    }

    /// Execute the traversal and collect all results.
    pub fn collect(self) -> Vec<TraversalResult<'snap>> {
        self.into_iter().collect()
    }

    /// Execute lazily.
    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(self) -> TraversalIter<'snap> {
        TraversalIter::new(self)
    }
}

/// Iterator produced by `TraversalBuilder::into_iter()`.
///
/// Implements BFS (default) or pseudo-DFS by swapping push_back/push_front.
/// State lives here — the iterator is the BFS/DFS frontier.
pub struct TraversalIter<'snap> {
    snapshot: &'snap Snapshot,
    queue: VecDeque<(EntityId, u32)>,
    visited: HashSet<EntityId>,
    /// Buffered results from the last node expansion, drained before the next pop.
    pending_results: VecDeque<TraversalResult<'snap>>,
    /// Parent tracking for path reconstruction: neighbor_id → (parent_id, relationship_id).
    parents: HashMap<EntityId, (EntityId, RelationshipId)>,
    direction: Direction,
    edge_classes: Option<Vec<RelationshipClass>>,
    node_type_filter: Option<EntityType>,
    node_class_filter: Option<EntityClass>,
    node_property_filters: Vec<PropertyFilter>,
    max_depth: u32,
    strategy: TraversalStrategy,
    cycle_detection: bool,
}

impl<'snap> TraversalIter<'snap> {
    fn new(b: TraversalBuilder<'snap>) -> Self {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        // Start node is in visited to prevent returning to it.
        visited.insert(b.start);
        queue.push_back((b.start, 0u32));

        TraversalIter {
            snapshot: b.snapshot,
            queue,
            visited,
            pending_results: VecDeque::new(),
            parents: HashMap::new(),
            direction: b.direction,
            edge_classes: b.edge_classes,
            node_type_filter: b.node_type_filter,
            node_class_filter: b.node_class_filter,
            node_property_filters: b.node_property_filters,
            max_depth: b.max_depth,
            strategy: b.strategy,
            cycle_detection: b.cycle_detection,
        }
    }

    /// Reconstruct the path from the traversal start to `target` using the
    /// `parents` map built during BFS expansion.
    fn reconstruct_path(&self, target: EntityId) -> GraphPath<'snap> {
        let mut segments = Vec::new();
        let mut cur = target;
        while let Some(&(parent_id, rel_id)) = self.parents.get(&cur) {
            if let (Some(entity), Some(rel)) = (
                self.snapshot.get_entity(cur),
                self.snapshot.get_relationship(rel_id),
            ) {
                segments.push(PathSegment {
                    relationship: rel,
                    entity,
                });
            }
            cur = parent_id;
        }
        segments.reverse();
        GraphPath { segments }
    }

    fn enqueue(&mut self, id: parallax_core::entity::EntityId, depth: u32) {
        match self.strategy {
            TraversalStrategy::BreadthFirst => self.queue.push_back((id, depth)),
            TraversalStrategy::DepthFirst => self.queue.push_front((id, depth)),
        }
    }

    fn node_passes_filter(&self, entity: &Entity) -> bool {
        if let Some(ref t) = self.node_type_filter {
            if &entity._type != t {
                return false;
            }
        }
        if let Some(ref c) = self.node_class_filter {
            if &entity._class != c {
                return false;
            }
        }
        self.node_property_filters.iter().all(|f| f.matches(entity))
    }
}

impl<'snap> Iterator for TraversalIter<'snap> {
    type Item = TraversalResult<'snap>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Drain buffered results from the last node expansion first.
            if let Some(result) = self.pending_results.pop_front() {
                return Some(result);
            }

            let (current_id, depth) = self.queue.pop_front()?;

            if depth >= self.max_depth {
                // At max depth: don't expand neighbors.
                continue;
            }

            let adj = self.snapshot.adjacency(current_id);

            // Process ALL neighbors before returning any — fixes the early-return
            // bug that left remaining neighbors of current_id unenqueued.
            for entry in adj {
                // Direction filter.
                if !self.direction.matches(entry.direction) {
                    continue;
                }

                // Edge class filter.
                if let Some(ref classes) = self.edge_classes {
                    let rel = match self.snapshot.get_relationship(entry.relationship_id) {
                        Some(r) => r,
                        None => continue,
                    };
                    if !classes.contains(&rel._class) {
                        continue;
                    }
                }

                let neighbor_id = entry.neighbor_id;
                let rel_id = entry.relationship_id;

                // Cycle detection: skip already-visited neighbors.
                if self.cycle_detection && !self.visited.insert(neighbor_id) {
                    continue;
                }

                let neighbor = match self.snapshot.get_entity(neighbor_id) {
                    Some(e) => e,
                    None => continue,
                };

                let neighbor_depth = depth + 1;

                // Record parent for path reconstruction (first time we reach each node).
                self.parents
                    .entry(neighbor_id)
                    .or_insert((current_id, rel_id));

                // Enqueue for further expansion regardless of node filter.
                self.enqueue(neighbor_id, neighbor_depth);

                // Node filter: buffer matching entities; traverse-through others.
                if self.node_passes_filter(neighbor) {
                    let path = self.reconstruct_path(neighbor_id);
                    self.pending_results.push_back(TraversalResult {
                        entity: neighbor,
                        depth: neighbor_depth,
                        path: Some(path),
                    });
                }
            }
            // Loop back to drain pending_results or pop the next queue entry.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;

    #[test]
    fn single_hop_outgoing() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1")
                .host("a", "h2")
                .rel("a", "host", "h1", "CONNECTS", "host", "h2");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "h1");
        let results = TraversalBuilder::new(&snap, id)
            .direction(Direction::Outgoing)
            .max_depth(1)
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].depth, 1);
    }

    #[test]
    fn multi_hop_bfs() {
        let (engine, _dir) = make_graph(|b| {
            // A → B → C
            b.host("a", "A")
                .host("a", "B")
                .host("a", "C")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "B", "CONNECTS", "host", "C");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id)
            .direction(Direction::Outgoing)
            .max_depth(3)
            .collect();
        assert_eq!(results.len(), 2); // B at depth 1, C at depth 2
        assert_eq!(results[0].depth, 1);
        assert_eq!(results[1].depth, 2);
    }

    #[test]
    fn cycle_detection_terminates() {
        let (engine, _dir) = make_graph(|b| {
            // A → B → A (cycle)
            b.host("a", "A")
                .host("a", "B")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "B", "CONNECTS", "host", "A");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id)
            .direction(Direction::Both)
            .max_depth(10)
            .collect();
        // Only B should be returned; cycle is detected.
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn edge_class_filter() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .host("a", "C")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "A", "TRUSTS", "host", "C");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id)
            .edge_classes(&["CONNECTS"])
            .max_depth(1)
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].entity.id,
            parallax_core::entity::EntityId::derive("a", "host", "B")
        );
    }

    #[test]
    fn path_reconstruction_single_hop() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .rel("a", "host", "A", "CONNECTS", "host", "B");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id)
            .direction(Direction::Outgoing)
            .max_depth(1)
            .collect();
        assert_eq!(results.len(), 1);
        let path = results[0].path.as_ref().expect("path must be Some");
        assert_eq!(path.segments.len(), 1);
        assert_eq!(
            path.segments[0].entity.id,
            parallax_core::entity::EntityId::derive("a", "host", "B")
        );
    }

    #[test]
    fn path_reconstruction_two_hops() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .host("a", "C")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "B", "CONNECTS", "host", "C");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id)
            .direction(Direction::Outgoing)
            .max_depth(3)
            .collect();
        // C is at depth 2 — its path should have 2 segments
        let c_result = results.iter().find(|r| r.depth == 2).expect("C at depth 2");
        let path = c_result.path.as_ref().expect("path must be Some");
        assert_eq!(path.segments.len(), 2);
        assert_eq!(
            path.segments[1].entity.id,
            parallax_core::entity::EntityId::derive("a", "host", "C")
        );
    }

    #[test]
    fn max_depth_respected() {
        let (engine, _dir) = make_graph(|b| {
            // Chain: A → B → C → D
            b.host("a", "A")
                .host("a", "B")
                .host("a", "C")
                .host("a", "D")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "B", "CONNECTS", "host", "C")
                .rel("a", "host", "C", "CONNECTS", "host", "D");
        });
        let snap = engine.snapshot();
        let id = parallax_core::entity::EntityId::derive("a", "host", "A");
        let results = TraversalBuilder::new(&snap, id).max_depth(2).collect();
        assert_eq!(results.len(), 2); // B (depth 1), C (depth 2); D excluded
    }
}
