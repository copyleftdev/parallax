//! Compliance posture — aggregates rule results per framework and control.
//!
//! **Spec reference:** `specs/08-policy-engine.md` §8.5

use parallax_core::timestamp::Timestamp;

use crate::evaluator::{RuleResult, RuleStatus};
use crate::rule::PolicyRule;

/// Overall compliance posture for a single framework.
#[derive(Debug)]
pub struct FrameworkPosture {
    pub framework: String,
    pub controls: Vec<ControlPosture>,
    /// Fraction of controls passing: 0.0 (all fail) to 1.0 (all pass).
    pub overall_score: f64,
    pub evaluated_at: Timestamp,
}

/// Posture for a single control within a framework.
#[derive(Debug)]
pub struct ControlPosture {
    pub control_id: String,
    pub status: ControlStatus,
    /// Number of rules mapped to this control.
    pub rule_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
}

/// Aggregate status for a control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlStatus {
    /// All mapped rules pass.
    Pass,
    /// At least one mapped rule fails.
    Fail,
    /// Some pass, some fail.
    Partial,
    /// No rules mapped to this control.
    NotMapped,
}

/// Compute a compliance posture for a specific framework from rule results.
///
/// `rules` and `results` must be parallel (same order).
pub fn compute_posture(
    framework: &str,
    rules: &[PolicyRule],
    results: &[RuleResult],
) -> FrameworkPosture {
    use std::collections::BTreeMap;

    // Collect per-control pass/fail counts.
    let mut control_stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    for (rule, result) in rules.iter().zip(results.iter()) {
        for mapping in &rule.frameworks {
            if mapping.framework != framework {
                continue;
            }
            let entry = control_stats.entry(mapping.control.clone()).or_insert((0, 0));
            match result.status {
                RuleStatus::Pass => entry.0 += 1,
                RuleStatus::Fail => entry.1 += 1,
                _ => {}
            }
        }
    }

    let controls: Vec<ControlPosture> = control_stats
        .into_iter()
        .map(|(control_id, (pass, fail))| {
            let status = if pass == 0 && fail == 0 {
                ControlStatus::NotMapped
            } else if fail == 0 {
                ControlStatus::Pass
            } else if pass == 0 {
                ControlStatus::Fail
            } else {
                ControlStatus::Partial
            };
            ControlPosture { control_id, status, rule_count: pass + fail, pass_count: pass, fail_count: fail }
        })
        .collect();

    let total = controls.len();
    let passing = controls.iter().filter(|c| c.status == ControlStatus::Pass).count();
    let overall_score = if total == 0 { 1.0 } else { passing as f64 / total as f64 };

    FrameworkPosture {
        framework: framework.to_owned(),
        controls,
        overall_score,
        evaluated_at: Timestamp::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{FrameworkMapping, PolicyRule};
    use crate::evaluator::{RuleResult, RuleStatus};
    use parallax_core::timestamp::Timestamp;
    use std::time::Duration;

    fn rule_with_framework(id: &str, framework: &str, control: &str) -> PolicyRule {
        let mut r = PolicyRule::new(id, id, "FIND host");
        r.frameworks.push(FrameworkMapping { framework: framework.into(), control: control.into() });
        r
    }

    fn pass_result(rule_id: &str) -> RuleResult {
        RuleResult {
            rule_id: rule_id.into(),
            status: RuleStatus::Pass,
            violations: vec![],
            error: None,
            evaluated_at: Timestamp::default(),
            duration: Duration::ZERO,
        }
    }

    fn fail_result(rule_id: &str) -> RuleResult {
        RuleResult {
            rule_id: rule_id.into(),
            status: RuleStatus::Fail,
            violations: vec![],
            error: None,
            evaluated_at: Timestamp::default(),
            duration: Duration::ZERO,
        }
    }

    #[test]
    fn all_pass_gives_full_score() {
        let rules = vec![
            rule_with_framework("r1", "CIS", "1.1"),
            rule_with_framework("r2", "CIS", "1.2"),
        ];
        let results = vec![pass_result("r1"), pass_result("r2")];
        let posture = compute_posture("CIS", &rules, &results);
        assert!((posture.overall_score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn one_fail_reduces_score() {
        let rules = vec![
            rule_with_framework("r1", "CIS", "1.1"),
            rule_with_framework("r2", "CIS", "1.2"),
        ];
        let results = vec![pass_result("r1"), fail_result("r2")];
        let posture = compute_posture("CIS", &rules, &results);
        // 1 of 2 controls passing = 0.5
        assert!((posture.overall_score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn partial_control_when_mixed() {
        // Two rules map to the same control, one passes, one fails.
        let rules = vec![
            rule_with_framework("r1", "CIS", "1.1"),
            rule_with_framework("r2", "CIS", "1.1"),
        ];
        let results = vec![pass_result("r1"), fail_result("r2")];
        let posture = compute_posture("CIS", &rules, &results);
        assert_eq!(posture.controls[0].status, ControlStatus::Partial);
        // Partial counts as failing for the score.
        assert!((posture.overall_score - 0.0).abs() < 1e-9);
    }

    #[test]
    fn empty_framework_full_score() {
        let posture = compute_posture("CIS", &[], &[]);
        assert!((posture.overall_score - 1.0).abs() < 1e-9);
    }
}
