//! # parallax-query
//!
//! PQL (Parallax Query Language) parser, planner, and executor.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use parallax_query::{parse, plan, execute, IndexStats, QueryLimits};
//! use parallax_graph::GraphReader;
//!
//! let query = parse("FIND host WITH state = 'running'")?;
//! let plan = plan(query, &stats)?;
//! let result = execute(&plan, &graph, QueryLimits::default())?;
//! println!("Found {} entities", result.count());
//! ```
//!
//! **Spec reference:** `specs/04-query-language.md`

pub mod ast;
pub mod error;
pub mod executor;
pub mod lexer;
pub mod parser;
pub mod planner;

// Public re-exports.
pub use ast::{
    BlastQuery, EntityFilter, EntityFilterKind, FindQuery, PathQuery, PropertyCondition, Query,
    ReturnClause, TraversalStep, Verb,
};
pub use error::{ExecError, ParseError, PlanError};
pub use executor::{execute, QueryLimits, QueryResult};
pub use parser::parse;
pub use planner::{plan, IndexAccess, IndexStats, PlannedTraversal, QueryPlan};
