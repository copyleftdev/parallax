# Policy Engine Overview

`parallax-policy` evaluates security policy rules against the live graph.
Policies are PQL-powered rules that identify compliance violations, security
gaps, and posture issues.

## What It Does

1. **Load rules:** Accept `PolicyRule` definitions with PQL queries
2. **Validate queries:** Reject rules whose PQL is invalid at load time (INV-P01)
3. **Evaluate:** Run all rules against the current graph snapshot
4. **Posture scoring:** Compute per-control status and an overall security posture score

## What It Is Not

- **Not a mutation engine.** Policy evaluation is read-only (INV-P02).
- **Not a real-time alerting system.** It evaluates on-demand or on schedule.
- **Not a SIEM.** It doesn't process event streams.

## Core Types

```rust
pub struct PolicyRule {
    /// Unique rule identifier
    pub id: String,

    /// Human-readable description
    pub title: String,

    /// PQL query that finds violating entities
    /// (empty result = compliant; any result = violation)
    pub query: String,

    /// Severity of the violation
    pub severity: Severity,

    /// Framework controls this rule maps to
    pub framework_mapping: Vec<FrameworkMapping>,

    /// Evaluation schedule (future: cron expression)
    pub schedule: Option<String>,
}

pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

pub struct FrameworkMapping {
    pub framework: String,    // "CIS", "NIST", "PCI-DSS", "SOC2"
    pub control_id: String,   // "CIS-1.1", "NIST-AC-2", etc.
}
```

## Quick Example

```rust
use parallax_policy::{PolicyEvaluator, PolicyRule, Severity};

let rules = vec![
    PolicyRule {
        id: "no-unprotected-hosts".to_string(),
        title: "All hosts must have EDR protection".to_string(),
        query: "FIND host THAT !PROTECTS edr_agent".to_string(),
        severity: Severity::High,
        framework_mapping: vec![
            FrameworkMapping {
                framework: "CIS".to_string(),
                control_id: "CIS-7.1".to_string(),
            }
        ],
        schedule: None,
    },
];

let evaluator = PolicyEvaluator::new(rules)?;

let snap = engine.snapshot();
let results = evaluator.evaluate_all(&snap)?;

for result in &results {
    println!("{}: {} violations ({})",
        result.rule_id, result.violations.len(), result.severity);
}

let posture = evaluator.compute_posture(&snap)?;
println!("Overall posture score: {:.1}%", posture.overall_score * 100.0);
```

See [Policy Rules](./rules.md), [Evaluation](./evaluation.md), and
[Posture Scoring](./posture.md) for complete documentation.
