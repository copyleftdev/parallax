//! Core error types shared across Parallax crates.

use thiserror::Error;

use crate::entity::EntityId;
use crate::relationship::RelationshipId;

/// Errors that can occur in the core data model.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid entity type '{value}': {reason}")]
    InvalidEntityType { value: String, reason: String },

    #[error("invalid entity class '{value}': {reason}")]
    InvalidEntityClass { value: String, reason: String },

    #[error("invalid relationship class '{value}': {reason}")]
    InvalidRelationshipClass { value: String, reason: String },

    #[error("referential integrity violation: relationship {rel_id} references non-existent entity {missing_id}")]
    DanglingReference {
        rel_id: RelationshipId,
        missing_id: EntityId,
    },

    #[error("property '{key}' type mismatch: expected {expected}, got {actual}")]
    PropertyTypeMismatch {
        key: String,
        expected: String,
        actual: String,
    },
}
