# Policy Rules

A policy rule is a named PQL query that finds security violations. An empty
result means compliant; any results mean violation.

## PolicyRule Structure

```rust
pub struct PolicyRule {
    /// Unique identifier (e.g. `edr-coverage-001`)
    pub id: String,
    /// Human-readable name
    pub name: String,
    pub severity: Severity,
    /// Long-form description
    pub description: String,
    /// The PQL query that finds violations (0 results = PASS, >0 = FAIL)
    pub query: String,
    /// Compliance framework mappings
    pub frameworks: Vec<FrameworkMapping>,
    /// Evaluation schedule
    pub schedule: Schedule,
    /// Remediation guidance (markdown)
    pub remediation: String,
    pub enabled: bool,
}
```

## Severity Levels

```rust
pub enum Severity {
    Critical,  // Immediate action required
    High,      // Resolve within 24 hours
    Medium,    // Resolve within 7 days
    Low,       // Track and resolve at next sprint
    Info,      // Informational, no SLA
}
```

Severities are serialised as lowercase strings in YAML/JSON:
`"critical"`, `"high"`, `"medium"`, `"low"`, `"info"`.

## Schedule

```rust
pub enum Schedule {
    Manual,               // Only on explicit request
    Every(Duration),      // Periodic: "every:5m", "every:2h", "every:1d"
    OnSync(Vec<String>),  // After named connectors sync: "on_sync:aws,gcp"
}
```

## Framework Mapping

```rust
pub struct FrameworkMapping {
    pub framework: String,  // e.g. "CIS-Controls-v8", "NIST-CSF", "SOC2"
    pub control: String,    // e.g. "10.1", "DE.CM-4"
}
```

## YAML Rule Files

Rules are defined in YAML and loaded with `load_rules_from_yaml(path)`:

```yaml
rules:
  - id: edr-coverage-001
    name: EDR coverage gap
    severity: high
    description: Hosts without EDR protection.
    query: "FIND host THAT !PROTECTS edr_agent"
    frameworks:
      - framework: CIS-Controls-v8
        control: "10.1"
    schedule: "manual"          # or "every:5m", "on_sync:aws,gcp"
    remediation: "Deploy EDR agent to all hosts."
    enabled: true

  - id: mfa-enforcement-001
    name: MFA not enforced
    severity: critical
    description: Active users without MFA.
    query: "FIND user WITH mfa_enabled = false AND active = true"
    frameworks:
      - framework: NIST-CSF
        control: "PR.AC-7"
      - framework: CIS-Controls-v8
        control: "6.5"
    schedule: "every:1h"
    remediation: "Enable MFA for all active user accounts."
    enabled: true
```

```rust
use parallax_policy::load_rules_from_yaml;

let rules = load_rules_from_yaml(Path::new("rules/security.yaml"))?;
```

## INV-P06: Validation at Load Time

Policy rules with invalid PQL are rejected when loaded into the evaluator,
not at evaluation time:

```rust
let result = PolicyEvaluator::load(vec![
    PolicyRule::new("bad", "Invalid PQL", "FIND host WHERE state = 'running'"),
], &stats);
// Err(PolicyError::InvalidQuery { rule_id: "bad", parse_error: "..." })
```

This prevents silent failures where a broken rule never catches violations.

## Query Design Patterns

| Pattern | Query |
|---|---|
| Find X that lack Y | `FIND X THAT !VERB Y` |
| Find X with bad property | `FIND X WITH property = 'bad_value'` |
| Find X in bad state OR another bad state | `FIND X WITH state = 'a' OR state = 'b'` |
| Find X connected to dangerous Y | `FIND X THAT VERB Y WITH dangerous = true` |
| Count of X (threshold check) | `FIND X RETURN COUNT` |
| Multi-hop risk path | `FIND X THAT V1 Y THAT V2 Z WITH prop = val` |

## Example Rules

### EDR Coverage

```yaml
- id: edr-coverage-001
  name: EDR coverage gap
  severity: high
  query: "FIND host THAT !PROTECTS edr_agent"
  frameworks:
    - framework: CIS-Controls-v8
      control: "10.1"
  schedule: "on_sync:aws"
  remediation: "Deploy EDR agent."
  enabled: true
```

### MFA Enforcement

```yaml
- id: mfa-all-users
  name: MFA not enforced
  severity: critical
  query: "FIND user WITH mfa_enabled = false AND active = true"
  frameworks:
    - framework: NIST-CSF
      control: "PR.AC-7"
  schedule: "every:1h"
  remediation: "Enable MFA for all active users."
  enabled: true
```

### Public Cloud Storage

```yaml
- id: no-public-buckets
  name: Public S3 buckets
  severity: critical
  query: "FIND aws_s3_bucket WITH public = true"
  frameworks:
    - framework: CIS-Controls-v8
      control: "3.3"
  schedule: "on_sync:aws"
  remediation: "Disable public access on all S3 buckets."
  enabled: true
```
