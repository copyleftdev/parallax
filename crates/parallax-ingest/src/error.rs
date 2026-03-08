//! Ingest error types.

use thiserror::Error;

use parallax_store::StoreError;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Storage error during sync commit: {0}")]
    StoreError(#[from] StoreError),

    #[error("Referential integrity violation: relationship references unknown entity {entity_id}")]
    DanglingRelationship { entity_id: String },

    #[error("Sync aborted: {reason}")]
    Aborted { reason: String },
}
