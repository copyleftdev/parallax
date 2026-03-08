//! Sync event types for observability.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.10

use std::time::Duration;

use crate::error::ConnectorError;

/// Events emitted during a sync cycle for observability.
#[derive(Debug)]
pub enum SyncEvent {
    Started { connector_id: String, sync_id: String },
    StepStarted { step_id: String },
    StepCompleted { step_id: String, entities: u64, relationships: u64, duration: Duration },
    StepFailed { step_id: String, error: ConnectorError },
    SyncCompleted { sync_id: String, entities_created: u64, entities_deleted: u64 },
    SyncFailed { sync_id: String, error: String },
}
