//! Secondary index types for the storage engine.
//!
//! Defines the compact adjacency list entry used by both the MemTable
//! and the graph engine for traversal.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.9

use parallax_core::{
    entity::EntityId,
    relationship::{Direction, RelationshipId},
};

/// A single entry in an entity's adjacency list.
///
/// Represents one edge from the perspective of a specific entity node.
/// Size: 16 (RelationshipId) + 1 (Direction) + 16 (EntityId) = 33 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjEntry {
    /// The relationship this edge belongs to.
    pub relationship_id: RelationshipId,
    /// `Outgoing`: this entity is the `from` side of the relationship.
    /// `Incoming`: this entity is the `to` side.
    pub direction: Direction,
    /// The other endpoint of this edge.
    pub neighbor_id: EntityId,
}

/// Adjacency list for a single entity node.
///
/// Stored as a flat Vec for cache-friendly traversal. Sorted by
/// `(direction, neighbor_id)` after bulk loads; append-order during
/// incremental updates.
pub type AdjList = Vec<AdjEntry>;
