//! # parallax-ingest
//!
//! Source-scoped sync protocol: diff, commit, and referential integrity.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.6

pub mod error;
pub mod sync;
pub mod validate;

pub use error::SyncError;
pub use sync::{commit_sync_exclusive, SyncEngine, SyncResult, SyncStats};
pub use validate::validate_sync_batch;
