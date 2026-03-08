//! Connector error types.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.9

use std::time::Duration;

use thiserror::Error;

/// Errors from connector execution.
#[derive(Debug, Error)]
pub enum ConnectorError {
    #[error("Authentication failed: {reason}")]
    AuthFailed { reason: String },

    #[error("API request failed: {endpoint} returned {status}: {body}")]
    ApiError { endpoint: String, status: u16, body: String },

    #[error("Rate limited by {service}. Retry after {retry_after:?}")]
    RateLimited { service: String, retry_after: Option<Duration> },

    #[error("Entity validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("Unknown step: {0}")]
    UnknownStep(String),

    #[error("Step dependency cycle detected: {cycle:?}")]
    DependencyCycle { cycle: Vec<String> },

    #[error("Connector timeout after {elapsed:?} (limit: {limit:?})")]
    Timeout { elapsed: Duration, limit: Duration },
}

/// Errors from the sync engine.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Sync commit failed: {0}")]
    Commit(#[from] parallax_ingest::SyncError),

    #[error("Connector failed during step '{step_id}': {error}")]
    StepFailed { step_id: String, error: String },

    #[error("A connector step task panicked: {0}")]
    StepPanic(String),
}
