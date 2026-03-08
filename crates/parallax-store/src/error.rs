//! Storage error types.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.10

use std::io;
use std::path::PathBuf;

use parallax_core::{entity::EntityId, relationship::RelationshipId};
use thiserror::Error;

/// Errors that can occur in the storage engine.
///
/// Every variant is explicit and typed — no silent failures.
/// (Lampson: "No silent failure.")
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("WAL write failed: {0}")]
    WalWrite(#[source] io::Error),

    #[error("WAL I/O failed: {0}")]
    WalIo(#[source] io::Error),

    #[error("WAL corrupt at sequence {seq}: CRC mismatch")]
    WalCorrupt { seq: u64 },

    #[error("segment read failed for '{path}': {source}")]
    SegmentRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("referential integrity violation: relationship {rel_id} references non-existent entity {missing_id}")]
    DanglingReference {
        rel_id: RelationshipId,
        missing_id: EntityId,
    },

    #[error("write batch rejected: {reason}")]
    ValidationFailed { reason: String },

    #[error("storage capacity exceeded: {details}")]
    CapacityExceeded { details: String },

    #[error("serialization failed: {0}")]
    Serialization(#[source] postcard::Error),

    #[error("failed to create storage directory: {0}")]
    DirCreate(#[source] io::Error),

    #[error("I/O error: {0}")]
    Io(#[source] io::Error),

    #[error("data corruption: {0}")]
    Corruption(String),
}

impl From<postcard::Error> for StoreError {
    fn from(e: postcard::Error) -> Self {
        StoreError::Serialization(e)
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}
