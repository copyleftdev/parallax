//! Write batch — the atomic unit of writes to the storage engine.
//!
//! A `WriteBatch` groups one or more operations that are applied atomically:
//! either all operations are visible in a snapshot, or none are.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.3
//!
//! INV-S02: A snapshot never observes a partially-applied WriteBatch.

use serde::{Deserialize, Serialize};

use parallax_core::{
    entity::{Entity, EntityId},
    relationship::{Relationship, RelationshipId},
};

/// A single write operation within a batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WriteOp {
    /// Insert or update an entity.
    UpsertEntity(Entity),
    /// Delete an entity by ID (inserts a tombstone).
    DeleteEntity(EntityId),
    /// Insert or update a relationship.
    UpsertRelationship(Relationship),
    /// Delete a relationship by ID (inserts a tombstone).
    DeleteRelationship(RelationshipId),
}

/// An atomic, ordered collection of write operations.
///
/// The batch is serialized as a single WAL entry, and applied to the
/// MemTable without interleaving. Readers either see all of a batch's
/// effects or none.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriteBatch {
    /// Ordered list of operations in this batch.
    pub operations: Vec<WriteOp>,
}

impl WriteBatch {
    /// Create an empty batch.
    pub fn new() -> Self {
        WriteBatch::default()
    }

    /// Add an upsert-entity operation.
    pub fn upsert_entity(&mut self, entity: Entity) -> &mut Self {
        self.operations.push(WriteOp::UpsertEntity(entity));
        self
    }

    /// Add a delete-entity operation.
    pub fn delete_entity(&mut self, id: EntityId) -> &mut Self {
        self.operations.push(WriteOp::DeleteEntity(id));
        self
    }

    /// Add an upsert-relationship operation.
    pub fn upsert_relationship(&mut self, rel: Relationship) -> &mut Self {
        self.operations.push(WriteOp::UpsertRelationship(rel));
        self
    }

    /// Add a delete-relationship operation.
    pub fn delete_relationship(&mut self, id: RelationshipId) -> &mut Self {
        self.operations.push(WriteOp::DeleteRelationship(id));
        self
    }

    /// Returns `true` if the batch contains no operations.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Number of operations in the batch.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Approximate serialized size in bytes (for WAL segment rotation heuristics).
    pub fn approx_size(&self) -> usize {
        self.operations.iter().map(|op| match op {
            WriteOp::UpsertEntity(e) => e.approx_size() + 8,
            WriteOp::DeleteEntity(_) => 24,
            WriteOp::UpsertRelationship(r) => r.approx_size() + 8,
            WriteOp::DeleteRelationship(_) => 24,
        }).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_batch_is_empty() {
        let b = WriteBatch::new();
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn batch_len_tracks_ops() {
        let mut b = WriteBatch::new();
        b.delete_entity(EntityId::default());
        b.delete_entity(EntityId::default());
        assert_eq!(b.len(), 2);
        assert!(!b.is_empty());
    }
}
