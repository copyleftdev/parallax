//! # parallax-policy
//!
//! Continuous compliance evaluation against the Parallax graph.
//!
//! Rules are defined in PQL. Each rule's query finds violations.
//! Zero results → PASS. Non-zero results → FAIL.
//!
//! **Spec reference:** `specs/08-policy-engine.md`

pub mod evaluator;
pub mod posture;
pub mod rule;

pub use evaluator::{PolicyEvaluator, RuleResult, RuleStatus, Violation};
pub use posture::{compute_posture, ControlPosture, ControlStatus, FrameworkPosture};
pub use rule::{load_rules_from_yaml, FrameworkMapping, PolicyError, PolicyRule, Schedule, Severity};
