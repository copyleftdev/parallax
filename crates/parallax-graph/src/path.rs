//! Shortest path — bidirectional BFS between two entities.
//!
//! Bidirectional BFS explores from both ends simultaneously. When the two
//! frontiers meet, the path is reconstructed from parent pointers.
//!
//! Complexity: O(b^(d/2)) vs O(b^d) for unidirectional BFS, where b is the
//! branching factor and d is the shortest path length.
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.6
//!
//! INV-G03: Returns the actual shortest path (by hop count), or `None`.

use std::collections::{hash_map::Entry, HashMap, VecDeque};

use parallax_core::{
    entity::EntityId,
    relationship::{Direction, RelationshipClass},
};
use parallax_store::Snapshot;

use crate::traversal::{GraphPath, PathSegment};

/// Builder for bidirectional shortest-path search.
pub struct ShortestPathBuilder<'snap> {
    snapshot: &'snap Snapshot,
    from: EntityId,
    to: EntityId,
    edge_classes: Option<Vec<RelationshipClass>>,
    max_depth: u32,
}

impl<'snap> ShortestPathBuilder<'snap> {
    pub(crate) fn new(snapshot: &'snap Snapshot, from: EntityId, to: EntityId) -> Self {
        ShortestPathBuilder {
            snapshot,
            from,
            to,
            edge_classes: None,
            max_depth: 10,
        }
    }

    /// Restrict to edges with these classes.
    pub fn edge_classes(mut self, classes: &[&str]) -> Self {
        self.edge_classes = Some(
            classes
                .iter()
                .map(|c| RelationshipClass::new_unchecked(c))
                .collect(),
        );
        self
    }

    /// Maximum path length in hops (default: 10).
    pub fn max_depth(mut self, depth: u32) -> Self {
        self.max_depth = depth;
        self
    }

    /// Find the shortest path. Returns `None` if no path exists within `max_depth`.
    ///
    /// INV-G03: The returned path has the minimum hop count of any path
    /// between `from` and `to`.
    ///
    /// Uses strict alternating bidirectional BFS: one level from the forward
    /// frontier, check for meeting, one level from the backward frontier, check
    /// again. Iterates up to `max_depth` times, so paths of length 1..=max_depth
    /// are all reachable.
    pub fn find(self) -> Option<GraphPath<'snap>> {
        if self.from == self.to {
            // Trivial: same entity, zero-length path.
            return Some(GraphPath { segments: vec![] });
        }

        // Parent maps: entity → (came_from, via_relationship)
        // Forward: expanding from `from`
        // Backward: expanding from `to`
        type Parent = (EntityId, parallax_core::relationship::RelationshipId);
        let mut fwd_visited: HashMap<EntityId, Option<Parent>> = HashMap::new();
        let mut bwd_visited: HashMap<EntityId, Option<Parent>> = HashMap::new();

        fwd_visited.insert(self.from, None);
        bwd_visited.insert(self.to, None);

        let mut fwd_frontier: VecDeque<EntityId> = VecDeque::from([self.from]);
        let mut bwd_frontier: VecDeque<EntityId> = VecDeque::from([self.to]);

        // Alternate one level per direction per iteration. Each frontier can
        // explore up to ceil(max_depth/2) hops, so the total reachable path
        // length is max_depth. Check for a meeting point after every expansion.
        for depth in 0..self.max_depth {
            if depth % 2 == 0 {
                if fwd_frontier.is_empty() {
                    break;
                }
                expand_frontier(
                    self.snapshot,
                    &mut fwd_frontier,
                    &mut fwd_visited,
                    &self.edge_classes,
                    Direction::Outgoing,
                );
            } else {
                if bwd_frontier.is_empty() {
                    break;
                }
                expand_frontier(
                    self.snapshot,
                    &mut bwd_frontier,
                    &mut bwd_visited,
                    &self.edge_classes,
                    Direction::Incoming,
                );
            }

            // Check for a meeting point after every expansion.
            let meeting = fwd_visited
                .keys()
                .find(|id| bwd_visited.contains_key(id))
                .copied();

            if let Some(mid) = meeting {
                return Some(reconstruct_path(
                    self.snapshot,
                    self.from,
                    self.to,
                    mid,
                    &fwd_visited,
                    &bwd_visited,
                ));
            }
        }

        None
    }
}

/// Expand one BFS frontier by one level.
fn expand_frontier(
    snapshot: &Snapshot,
    frontier: &mut VecDeque<EntityId>,
    visited: &mut HashMap<
        EntityId,
        Option<(EntityId, parallax_core::relationship::RelationshipId)>,
    >,
    edge_classes: &Option<Vec<RelationshipClass>>,
    direction: Direction,
) {
    let count = frontier.len();
    for _ in 0..count {
        let current = match frontier.pop_front() {
            Some(id) => id,
            None => break,
        };
        for adj in snapshot.adjacency(current) {
            if !direction.matches(adj.direction) {
                continue;
            }
            if let Some(ref classes) = edge_classes {
                let rel = match snapshot.get_relationship(adj.relationship_id) {
                    Some(r) => r,
                    None => continue,
                };
                if !classes.contains(&rel._class) {
                    continue;
                }
            }
            if let Entry::Vacant(e) = visited.entry(adj.neighbor_id) {
                e.insert(Some((current, adj.relationship_id)));
                frontier.push_back(adj.neighbor_id);
            }
        }
    }
}

/// Reconstruct the full path from `from` to `to` via meeting point `mid`.
fn reconstruct_path<'snap>(
    snapshot: &'snap Snapshot,
    from: EntityId,
    _to: EntityId,
    mid: EntityId,
    fwd: &HashMap<EntityId, Option<(EntityId, parallax_core::relationship::RelationshipId)>>,
    bwd: &HashMap<EntityId, Option<(EntityId, parallax_core::relationship::RelationshipId)>>,
) -> GraphPath<'snap> {
    let mut segments: Vec<PathSegment<'snap>> = Vec::new();

    // Build forward half: from → mid
    let mut fwd_chain: Vec<(EntityId, parallax_core::relationship::RelationshipId)> = Vec::new();
    let mut cur = mid;
    while cur != from {
        match fwd.get(&cur).and_then(|p| *p) {
            Some((parent, rel_id)) => {
                fwd_chain.push((cur, rel_id));
                cur = parent;
            }
            None => break,
        }
    }
    fwd_chain.reverse();
    for (entity_id, rel_id) in fwd_chain {
        if let (Some(rel), Some(entity)) = (
            snapshot.get_relationship(rel_id),
            snapshot.get_entity(entity_id),
        ) {
            segments.push(PathSegment {
                relationship: rel,
                entity,
            });
        }
    }

    // Build backward half: mid → to
    let mut cur = mid;
    while let Some((parent, rel_id)) = bwd.get(&cur).and_then(|p| *p) {
        if let (Some(rel), Some(entity)) = (
            snapshot.get_relationship(rel_id),
            snapshot.get_entity(parent),
        ) {
            segments.push(PathSegment {
                relationship: rel,
                entity,
            });
        }
        cur = parent;
    }

    GraphPath { segments }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;

    #[test]
    fn same_entity_is_zero_length() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1");
        });
        let snap = engine.snapshot();
        let id = EntityId::derive("a", "host", "h1");
        let path = ShortestPathBuilder::new(&snap, id, id).find();
        assert!(path.is_some());
        assert_eq!(path.unwrap().segments.len(), 0);
    }

    #[test]
    fn direct_edge_gives_one_hop() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .rel("a", "host", "A", "CONNECTS", "host", "B");
        });
        let snap = engine.snapshot();
        let a = EntityId::derive("a", "host", "A");
        let b_id = EntityId::derive("a", "host", "B");
        let path = ShortestPathBuilder::new(&snap, a, b_id).find();
        assert!(path.is_some());
    }

    #[test]
    fn no_path_returns_none() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A").host("a", "B"); // no edges
        });
        let snap = engine.snapshot();
        let a = EntityId::derive("a", "host", "A");
        let b_id = EntityId::derive("a", "host", "B");
        let path = ShortestPathBuilder::new(&snap, a, b_id).find();
        assert!(path.is_none());
    }

    /// Regression: the original BFS only explored one frontier per iteration,
    /// so paths longer than max_depth/2 could be missed. This test uses a
    /// 4-hop chain and max_depth = 4; the path must be found.
    #[test]
    fn four_hop_path_found_within_max_depth() {
        // Chain: A → B → C → D → E  (4 hops)
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "A")
                .host("a", "B")
                .host("a", "C")
                .host("a", "D")
                .host("a", "E")
                .rel("a", "host", "A", "CONNECTS", "host", "B")
                .rel("a", "host", "B", "CONNECTS", "host", "C")
                .rel("a", "host", "C", "CONNECTS", "host", "D")
                .rel("a", "host", "D", "CONNECTS", "host", "E");
        });
        let snap = engine.snapshot();
        let a = EntityId::derive("a", "host", "A");
        let e = EntityId::derive("a", "host", "E");
        let path = ShortestPathBuilder::new(&snap, a, e).max_depth(4).find();
        assert!(path.is_some(), "4-hop path must be found with max_depth=4");
    }
}
