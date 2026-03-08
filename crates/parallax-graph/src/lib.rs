//! # parallax-graph
//!
//! Graph engine: traversal, pattern matching, shortest path, blast radius,
//! and coverage gap detection. Operates on MVCC snapshots from `parallax-store`.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! let graph = GraphReader::new(&snapshot);
//!
//! // Find all running EC2 instances
//! let instances = graph.find("aws_ec2_instance").with("state", "running").collect();
//!
//! // Who can reach this host (up to 4 hops, incoming)?
//! let accessors = graph.traverse(host_id)
//!     .direction(Direction::Incoming)
//!     .max_depth(4)
//!     .collect();
//!
//! // What's the blast radius if this host is compromised?
//! let impact = graph.blast_radius(host_id).default_rules().analyze();
//! ```
//!
//! **Spec reference:** `specs/03-graph-engine.md`

pub mod blast;
pub mod coverage;
pub mod finder;
pub mod path;
pub mod pattern;
pub mod reader;
pub mod traversal;

#[cfg(test)]
pub(crate) mod test_helpers;

// Public API re-exports.
pub use blast::{BlastRadiusBuilder, BlastRadiusResult};
pub use coverage::CoverageGapBuilder;
pub use finder::{CmpOp, EntityFinder, PropertyFilter};
pub use path::ShortestPathBuilder;
pub use pattern::{PatternBuilder, PatternStep};
pub use reader::GraphReader;
pub use traversal::{GraphPath, PathSegment, TraversalBuilder, TraversalResult, TraversalStrategy};
