# Posture Scoring

Posture scoring aggregates rule evaluation results into a per-framework
compliance score and an overall posture score.

## Computing Posture

```rust
let posture = evaluator.compute_posture(&snap)?;

println!("Overall: {:.1}%", posture.overall_score * 100.0);
for (framework, score) in &posture.framework_scores {
    println!("{}: {:.1}%", framework, score.score * 100.0);
}
```

## FrameworkPosture

```rust
pub struct FrameworkPosture {
    /// Overall score: 0.0 (no rules passing) to 1.0 (all rules passing)
    pub overall_score: f64,

    /// Per-framework scores
    pub framework_scores: HashMap<String, ControlStatus>,

    /// Breakdown by severity
    pub by_severity: HashMap<Severity, SeverityBreakdown>,

    /// All rule results (pass, fail, error)
    pub results: Vec<RuleEvaluationResult>,
}

pub struct ControlStatus {
    pub framework: String,
    pub score: f64,            // 0.0 to 1.0
    pub passing_controls: u32,
    pub failing_controls: u32,
    pub error_controls: u32,
}

pub struct SeverityBreakdown {
    pub passing: u32,
    pub failing: u32,
    pub errored: u32,
}
```

## Scoring Algorithm

**Overall score:**

```
score = passing_rules / total_rules
```

Where:
- `passing_rules` = rules with `EvaluationStatus::Pass`
- `total_rules` = all rules (pass + fail + error)

**INV-P04:** Errored rules count as failures for posture scoring. A rule
that can't evaluate is treated as if it found violations.

**Per-framework score:**

For each framework (CIS, NIST, PCI-DSS, etc.), compute the ratio of passing
controls to total controls mapped to that framework:

```
framework_score = controls_passing / controls_mapped_to_framework
```

If a rule maps to multiple frameworks, it contributes to each framework's
score independently.

## Example Report

```
Security Posture Report
=======================
Overall Score: 73.3% (11/15 rules passing)

By Severity:
  Critical: 2 passing, 2 failing    (50.0%)
  High:     4 passing, 1 failing    (80.0%)
  Medium:   5 passing, 0 failing   (100.0%)
  Low:      0 passing, 1 failing    (0.0%)
  Info:     0 passing, 0 failing      (N/A)

By Framework:
  CIS:     8/10 controls passing   (80.0%)
  NIST:    6/8  controls passing   (75.0%)
  PCI-DSS: 4/7  controls passing   (57.1%)

Failing Rules:
  ✗ [CRITICAL] All users must have MFA enabled
    → 47 users without MFA
  ✗ [CRITICAL] No S3 buckets should be publicly accessible
    → 3 public buckets: logs-bucket, public-assets, backup-2023
  ✗ [HIGH] All hosts must have EDR protection
    → 12 unprotected hosts
  ✗ [LOW] Minimize admin role assignments
    → 23 admin assignments (threshold: 10)
```

## Framework Context

| Framework | Focus Area | Common Controls |
|---|---|---|
| **CIS** | Technical security benchmarks | EDR coverage, patch levels, network segmentation |
| **NIST** | Broad cybersecurity framework | Access control, identity, protect/detect/respond |
| **PCI-DSS** | Payment card data security | Network isolation, encryption, access logging |
| **SOC 2** | Service organization controls | Availability, confidentiality, integrity |
| **HIPAA** | Healthcare data protection | PHI access, audit trails, encryption |

The framework mapping in `PolicyRule` is metadata — it doesn't change how
the rule is evaluated, only how it's categorized in the posture report.
