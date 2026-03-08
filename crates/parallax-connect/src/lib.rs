//! # parallax-connect
//!
//! Integration SDK: connector trait, entity/relationship builders, step scheduler.
//!
//! ## Quick start — writing a connector
//!
//! ```rust,ignore
//! use parallax_connect::prelude::*;
//!
//! pub struct MyConnector;
//!
//! #[async_trait::async_trait]
//! impl Connector for MyConnector {
//!     fn name(&self) -> &str { "my-connector" }
//!
//!     fn steps(&self) -> Vec<StepDefinition> {
//!         vec![
//!             step("hosts", "Collect hosts").build(),
//!             step("services", "Collect services").depends_on(&["hosts"]).build(),
//!         ]
//!     }
//!
//!     async fn execute_step(&self, step_id: &str, ctx: &mut StepContext) -> Result<(), ConnectorError> {
//!         match step_id {
//!             "hosts" => {
//!                 ctx.emit_entity(entity("host", "h1").class("Host").display_name("Server 1"))?;
//!                 Ok(())
//!             }
//!             _ => Err(ConnectorError::UnknownStep(step_id.to_string())),
//!         }
//!     }
//! }
//! ```
//!
//! **Spec reference:** `specs/05-integration-sdk.md`

pub mod builder;
pub mod connector;
pub mod error;
pub mod event;
pub mod scheduler;

// Public re-exports.
pub use builder::{entity, relationship, EntityBuilder, RelationshipBuilder};
pub use connector::{
    step, topological_order, topological_waves, validate_steps, Connector, PriorStepData,
    StepContext, StepDefinition, StepDefinitionBuilder,
};
pub use error::{ConnectorError, SyncError};
pub use event::SyncEvent;
pub use scheduler::{run_connector, ConnectorOutput};

/// Convenience re-exports for connector authors.
pub mod prelude {
    pub use super::{
        builder::{entity, relationship},
        connector::{step, Connector, StepContext, StepDefinition},
        error::ConnectorError,
    };
    pub use async_trait::async_trait;
}
