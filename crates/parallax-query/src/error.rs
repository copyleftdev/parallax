//! Error types for PQL parsing, planning, and execution.
//!
//! **Spec reference:** `specs/04-query-language.md` §4.7, §4.9

use std::time::Duration;

use thiserror::Error;

/// Errors produced by the PQL lexer and parser (INV-Q05).
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    #[error(
        "Unknown verb '{verb}' at position {pos}. \
             Valid verbs: HAS, IS, ASSIGNED, ALLOWS, USES, CONTAINS, \
             MANAGES, CONNECTS, PROTECTS, EXPLOITS, TRUSTS, SCANS, RELATES TO"
    )]
    UnknownVerb { verb: String, pos: usize },

    #[error("Expected {expected} at position {pos}, got '{got}'")]
    Unexpected {
        expected: String,
        got: String,
        pos: usize,
    },

    #[error("Unterminated string literal at position {pos}")]
    UnterminatedString { pos: usize },

    #[error("Unexpected character '{ch}' at position {pos}")]
    UnexpectedChar { ch: char, pos: usize },

    #[error("Expected integer at position {pos}")]
    ExpectedInteger { pos: usize },
}

/// Errors produced by the query planner.
#[derive(Debug, Clone, Error)]
pub enum PlanError {
    #[error("Cannot plan empty query")]
    EmptyQuery,
}

/// Errors produced during query execution (INV-Q04).
#[derive(Debug, Clone, Error)]
pub enum ExecError {
    #[error("Query exceeded time limit of {limit:?} (elapsed: {elapsed:?})")]
    Timeout { limit: Duration, elapsed: Duration },

    #[error("Query scanned {scanned} entities, exceeding limit of {limit}")]
    ScanLimitExceeded { scanned: u64, limit: u64 },

    #[error("Query traversed {traversed} edges, exceeding limit of {limit}")]
    TraversalLimitExceeded { traversed: u64, limit: u64 },

    #[error("Result set exceeded max_results limit of {limit}")]
    ResultLimitExceeded { limit: usize },

    #[error("Plan node produced unexpected result type")]
    TypeMismatch,

    #[error("Shortest path: no matching entity found for '{side}' filter")]
    NoMatchingEntity { side: &'static str },
}
