//! Policy rule types — the interface between rule authors and the evaluator.
//!
//! **Spec reference:** `specs/08-policy-engine.md` §8.3, §8.7, §8.8
//!
//! ## YAML format
//!
//! ```yaml
//! rules:
//!   - id: edr-coverage-001
//!     name: EDR coverage gap
//!     severity: high
//!     description: Hosts without EDR protection.
//!     query: "FIND host THAT !PROTECTS edr_agent"
//!     frameworks:
//!       - framework: CIS-Controls-v8
//!         control: "10.1"
//!     schedule: "manual"          # or "every:5m", "on_sync:aws,gcp"
//!     remediation: "Deploy EDR."
//!     enabled: true
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// A policy rule. Parsed from YAML or constructed in code.
///
/// Lampson: "Define interfaces before implementation."
/// INV-P06: PQL is validated at rule load time, not at evaluation time.
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyRule {
    /// Unique identifier (e.g. `edr-coverage-001`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    pub severity: Severity,
    /// Long-form description.
    pub description: String,
    /// The PQL query that finds violations.
    /// 0 results → PASS. >0 results → FAIL.
    pub query: String,
    /// Compliance framework mappings.
    pub frameworks: Vec<FrameworkMapping>,
    /// Evaluation schedule.
    pub schedule: Schedule,
    /// Remediation guidance (markdown).
    pub remediation: String,
    pub enabled: bool,
}

impl PolicyRule {
    /// Construct a minimal rule with defaults.
    pub fn new(id: impl Into<String>, name: impl Into<String>, query: impl Into<String>) -> Self {
        PolicyRule {
            id: id.into(),
            name: name.into(),
            severity: Severity::Medium,
            description: String::new(),
            query: query.into(),
            frameworks: Vec::new(),
            schedule: Schedule::Manual,
            remediation: String::new(),
            enabled: true,
        }
    }
}

/// Rule severity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Critical => write!(f, "critical"),
            Severity::High => write!(f, "high"),
            Severity::Medium => write!(f, "medium"),
            Severity::Low => write!(f, "low"),
            Severity::Info => write!(f, "info"),
        }
    }
}

/// A compliance framework control mapping.
#[derive(Debug, Clone, Deserialize)]
pub struct FrameworkMapping {
    /// Framework identifier: `"CIS-Controls-v8"`, `"NIST-CSF"`, `"SOC2"`, etc.
    pub framework: String,
    /// Control identifier within the framework: `"10.1"`, `"DE.CM-4"`, etc.
    pub control: String,
}

/// When a rule should be evaluated.
///
/// Deserializes from a string:
/// - `"manual"` → `Manual`
/// - `"every:5m"` / `"every:1h"` / `"every:30s"` → `Every(Duration)`
/// - `"on_sync:aws,gcp"` → `OnSync(vec!["aws", "gcp"])`
#[derive(Debug, Clone)]
pub enum Schedule {
    /// Re-evaluate every `Duration`.
    Every(Duration),
    /// Re-evaluate after every sync commit from the named connectors.
    OnSync(Vec<String>),
    /// Only evaluate on explicit request.
    Manual,
}

impl<'de> Deserialize<'de> for Schedule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        parse_schedule(&s).map_err(serde::de::Error::custom)
    }
}

fn parse_schedule(s: &str) -> Result<Schedule, String> {
    if s == "manual" {
        return Ok(Schedule::Manual);
    }
    if let Some(rest) = s.strip_prefix("every:") {
        let dur = parse_duration(rest)
            .ok_or_else(|| format!("invalid duration '{rest}' in schedule '{s}'"))?;
        return Ok(Schedule::Every(dur));
    }
    if let Some(rest) = s.strip_prefix("on_sync:") {
        let connectors = rest.split(',').map(|c| c.trim().to_owned()).collect();
        return Ok(Schedule::OnSync(connectors));
    }
    Err(format!(
        "unrecognised schedule '{s}' — expected 'manual', 'every:<dur>', or 'on_sync:<ids>'"
    ))
}

/// Parse simple duration strings: "30s", "5m", "2h", "1d".
fn parse_duration(s: &str) -> Option<Duration> {
    let (num, unit) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86400)
    } else {
        return None;
    };
    let secs: u64 = num.parse().ok()?;
    Some(Duration::from_secs(secs * unit))
}

// ─── YAML rule file loading ───────────────────────────────────────────────────

/// Container for a YAML rule file.
#[derive(Deserialize)]
struct RuleFile {
    rules: Vec<PolicyRule>,
}

/// Load policy rules from a YAML file on disk.
///
/// The YAML file must contain a top-level `rules:` sequence.
/// Returns `PolicyError::RuleParseError` on YAML syntax errors.
pub fn load_rules_from_yaml(path: &Path) -> Result<Vec<PolicyRule>, PolicyError> {
    let content = std::fs::read_to_string(path).map_err(|e| PolicyError::RuleParseError {
        path: path.to_owned(),
        reason: e.to_string(),
    })?;
    let file: RuleFile =
        serde_yaml::from_str(&content).map_err(|e| PolicyError::RuleParseError {
            path: path.to_owned(),
            reason: e.to_string(),
        })?;
    Ok(file.rules)
}

/// Errors from the policy engine.
#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("Rule '{rule_id}' contains invalid PQL: {parse_error}")]
    InvalidQuery {
        rule_id: String,
        parse_error: String,
    },

    #[error("Rule '{rule_id}' query exceeded execution limits: {details}")]
    QueryLimitExceeded { rule_id: String, details: String },

    #[error("Rule file parse error in {path}: {reason}")]
    RuleParseError { path: PathBuf, reason: String },

    #[error("Unknown framework '{framework}' referenced in rule '{rule_id}'")]
    UnknownFramework { rule_id: String, framework: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_yaml(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_schedule_manual() {
        assert!(matches!(
            parse_schedule("manual").unwrap(),
            Schedule::Manual
        ));
    }

    #[test]
    fn parse_schedule_every_minutes() {
        let s = parse_schedule("every:5m").unwrap();
        assert!(matches!(s, Schedule::Every(d) if d.as_secs() == 300));
    }

    #[test]
    fn parse_schedule_every_hours() {
        let s = parse_schedule("every:2h").unwrap();
        assert!(matches!(s, Schedule::Every(d) if d.as_secs() == 7200));
    }

    #[test]
    fn parse_schedule_on_sync() {
        let s = parse_schedule("on_sync:aws,gcp").unwrap();
        assert!(matches!(s, Schedule::OnSync(v) if v == vec!["aws", "gcp"]));
    }

    #[test]
    fn parse_schedule_invalid_returns_err() {
        assert!(parse_schedule("bogus").is_err());
    }

    #[test]
    fn load_yaml_minimal_rule() {
        let yaml = r#"
rules:
  - id: test-001
    name: Test Rule
    severity: high
    description: "A test rule"
    query: "FIND host"
    frameworks: []
    schedule: "manual"
    remediation: "Fix it."
    enabled: true
"#;
        let f = write_yaml(yaml);
        let rules = load_rules_from_yaml(f.path()).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "test-001");
        assert_eq!(rules[0].severity, Severity::High);
        assert!(matches!(rules[0].schedule, Schedule::Manual));
    }

    #[test]
    fn load_yaml_with_frameworks_and_schedule() {
        let yaml = r#"
rules:
  - id: edr-001
    name: EDR Gap
    severity: critical
    description: "Missing EDR"
    query: "FIND host THAT !PROTECTS edr_agent"
    frameworks:
      - framework: CIS-Controls-v8
        control: "10.1"
    schedule: "every:30m"
    remediation: "Deploy EDR."
    enabled: true
"#;
        let f = write_yaml(yaml);
        let rules = load_rules_from_yaml(f.path()).unwrap();
        assert_eq!(rules[0].frameworks.len(), 1);
        assert_eq!(rules[0].frameworks[0].framework, "CIS-Controls-v8");
        assert!(matches!(rules[0].schedule, Schedule::Every(d) if d.as_secs() == 1800));
    }

    #[test]
    fn load_yaml_multiple_rules() {
        let yaml = r#"
rules:
  - id: r1
    name: Rule 1
    severity: low
    description: ""
    query: "FIND host"
    frameworks: []
    schedule: "manual"
    remediation: ""
    enabled: true
  - id: r2
    name: Rule 2
    severity: medium
    description: ""
    query: "FIND service"
    frameworks: []
    schedule: "on_sync:aws"
    remediation: ""
    enabled: false
"#;
        let f = write_yaml(yaml);
        let rules = load_rules_from_yaml(f.path()).unwrap();
        assert_eq!(rules.len(), 2);
        assert!(!rules[1].enabled);
    }

    #[test]
    fn load_yaml_bad_syntax_returns_err() {
        let f = write_yaml("not: valid: yaml: at: all: [\n");
        assert!(load_rules_from_yaml(f.path()).is_err());
    }
}
