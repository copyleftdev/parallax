//! Connector trait and step definitions.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.3, §5.5

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use compact_str::CompactString;
use parallax_core::source::SourceTag;
use parallax_core::timestamp::Timestamp;

use crate::builder::{EntityBuilder, RelationshipBuilder};
use crate::error::ConnectorError;

// ─── Step definition ──────────────────────────────────────────────────────────

/// A single step in a connector's execution plan.
#[derive(Debug, Clone)]
pub struct StepDefinition {
    /// Unique step identifier (e.g. `"iam-users"`).
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// IDs of steps that must complete before this one runs.
    pub depends_on: Vec<String>,
}

/// Construct a StepDefinition. Used in `Connector::steps()`.
pub fn step(id: &str, description: &str) -> StepDefinitionBuilder {
    StepDefinitionBuilder {
        id: id.to_owned(),
        description: description.to_owned(),
        depends_on: Vec::new(),
    }
}

/// Builder for StepDefinition.
pub struct StepDefinitionBuilder {
    id: String,
    description: String,
    depends_on: Vec<String>,
}

impl StepDefinitionBuilder {
    pub fn depends_on(mut self, deps: &[&str]) -> Self {
        self.depends_on = deps.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn build(self) -> StepDefinition {
        StepDefinition { id: self.id, description: self.description, depends_on: self.depends_on }
    }
}

impl From<StepDefinitionBuilder> for StepDefinition {
    fn from(b: StepDefinitionBuilder) -> Self {
        b.build()
    }
}

// ─── Step context ─────────────────────────────────────────────────────────────

/// Entities and relationships emitted by prior steps (read-only).
#[derive(Debug, Default, Clone)]
pub struct PriorStepData {
    /// key: (entity_type, entity_key) → EntityBuilder snapshot
    pub(crate) entities: HashMap<(String, String), EntityBuilder>,
}

impl PriorStepData {
    pub fn get(&self, entity_type: &str, entity_key: &str) -> Option<&EntityBuilder> {
        self.entities.get(&(entity_type.to_owned(), entity_key.to_owned()))
    }

    pub(crate) fn insert(&mut self, b: &EntityBuilder) {
        self.entities.insert((b.entity_type.clone(), b.entity_key.clone()), b.clone());
    }
}

/// Metrics for a single step execution.
#[derive(Debug, Default, Clone)]
pub struct StepMetrics {
    pub entities_emitted: u64,
    pub relationships_emitted: u64,
}

/// The interface between a connector step and the Parallax engine.
///
/// Lampson: "Leave it to the client." The SDK handles ID derivation,
/// versioning, batching, and sync diffing. The connector just emits
/// what it sees.
pub struct StepContext {
    /// Connector-specific configuration (account ID, etc.)
    pub connector_id: String,
    pub account_id: String,
    pub sync_id: String,
    /// Emitted entities for this step.
    pub(crate) entities: Vec<EntityBuilder>,
    /// Emitted relationships for this step.
    pub(crate) relationships: Vec<RelationshipBuilder>,
    /// Access to entities from prior steps (read-only).
    pub prior_entities: Arc<PriorStepData>,
    /// Per-step metrics.
    pub metrics: StepMetrics,
}

impl StepContext {
    pub(crate) fn new(
        connector_id: &str,
        account_id: &str,
        sync_id: &str,
        prior: Arc<PriorStepData>,
    ) -> Self {
        StepContext {
            connector_id: connector_id.to_owned(),
            account_id: account_id.to_owned(),
            sync_id: sync_id.to_owned(),
            entities: Vec::new(),
            relationships: Vec::new(),
            prior_entities: prior,
            metrics: StepMetrics::default(),
        }
    }

    /// Emit an entity. The SDK validates and queues it for sync.
    pub fn emit_entity(&mut self, builder: EntityBuilder) -> Result<(), ConnectorError> {
        if builder.entity_type.is_empty() {
            return Err(ConnectorError::ValidationFailed {
                reason: "entity_type must not be empty".into(),
            });
        }
        if builder.entity_key.is_empty() {
            return Err(ConnectorError::ValidationFailed {
                reason: "entity_key must not be empty".into(),
            });
        }
        self.metrics.entities_emitted += 1;
        self.entities.push(builder);
        Ok(())
    }

    /// Emit a relationship.
    pub fn emit_relationship(
        &mut self,
        builder: RelationshipBuilder,
    ) -> Result<(), ConnectorError> {
        if builder.verb.is_empty() {
            return Err(ConnectorError::ValidationFailed {
                reason: "relationship verb must not be empty".into(),
            });
        }
        self.metrics.relationships_emitted += 1;
        self.relationships.push(builder);
        Ok(())
    }

    /// Look up an entity emitted by a prior step.
    pub fn get_prior_entity(
        &self,
        entity_type: &str,
        entity_key: &str,
    ) -> Option<&EntityBuilder> {
        self.prior_entities.get(entity_type, entity_key)
    }

    /// Build a SourceTag for materialization.
    pub(crate) fn source_tag(&self) -> SourceTag {
        SourceTag {
            connector_id: CompactString::new(&self.connector_id),
            sync_id: CompactString::new(&self.sync_id),
            sync_timestamp: Timestamp::now(),
        }
    }
}

// ─── Connector trait ──────────────────────────────────────────────────────────

/// The trait every connector implements.
///
/// INV-C05: Step dependencies are validated at registration time. Cycles rejected.
/// INV-C06: A failed step doesn't prevent independent steps from running.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Human-readable connector name.
    fn name(&self) -> &str;

    /// The steps this connector executes, in definition order.
    fn steps(&self) -> Vec<StepDefinition>;

    /// Execute a single step. Called by the scheduler in dependency order.
    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError>;
}

/// Validate that step dependencies are acyclic (INV-C05).
pub fn validate_steps(steps: &[StepDefinition]) -> Result<(), ConnectorError> {
    let step_ids: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.id.as_str()).collect();

    // Check all declared dependencies exist.
    for step in steps {
        for dep in &step.depends_on {
            if !step_ids.contains(dep.as_str()) {
                return Err(ConnectorError::DependencyCycle {
                    cycle: vec![format!("{} depends on unknown step {}", step.id, dep)],
                });
            }
        }
    }

    // Topological sort (Kahn's algorithm) to detect cycles.
    let mut in_degree: HashMap<&str, usize> = steps.iter().map(|s| (s.id.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for step in steps {
        for dep in &step.depends_on {
            *in_degree.entry(step.id.as_str()).or_insert(0) += 1;
            adj.entry(dep.as_str()).or_default().push(step.id.as_str());
        }
    }

    let mut queue: std::collections::VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut count = 0;
    while let Some(id) = queue.pop_front() {
        count += 1;
        if let Some(dependents) = adj.get(id) {
            for dep in dependents {
                let entry = in_degree.entry(dep).or_default();
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    if count != steps.len() {
        let cycle: Vec<String> = steps
            .iter()
            .filter(|s| *in_degree.get(s.id.as_str()).unwrap_or(&0) > 0)
            .map(|s| s.id.clone())
            .collect();
        return Err(ConnectorError::DependencyCycle { cycle });
    }

    Ok(())
}

/// Group steps into parallel execution waves.
///
/// Steps within a wave have no inter-dependencies and can execute concurrently.
/// Waves are returned in dependency order — wave 0 has no deps, wave N depends
/// on the completion of all prior waves.
///
/// Returns a `Vec<Vec<String>>` where each inner vec holds step IDs for one wave.
pub fn topological_waves(steps: &[StepDefinition]) -> Vec<Vec<String>> {
    if steps.is_empty() {
        return Vec::new();
    }
    let order = topological_order(steps);

    // Assign each step to a level: level = max(dep levels) + 1, or 0 if no deps.
    let mut levels: HashMap<&str, usize> = HashMap::new();
    for step in &order {
        let level = if step.depends_on.is_empty() {
            0
        } else {
            step.depends_on
                .iter()
                .map(|d| levels.get(d.as_str()).copied().unwrap_or(0) + 1)
                .max()
                .unwrap_or(1)
        };
        levels.insert(step.id.as_str(), level);
    }

    let max_level = levels.values().copied().max().unwrap_or(0);
    let mut waves: Vec<Vec<String>> = vec![Vec::new(); max_level + 1];
    for step in steps {
        let lvl = levels.get(step.id.as_str()).copied().unwrap_or(0);
        waves[lvl].push(step.id.clone());
    }
    waves.retain(|w| !w.is_empty());
    waves
}

/// Topological sort of steps for execution order.
pub fn topological_order(steps: &[StepDefinition]) -> Vec<&StepDefinition> {
    let mut in_degree: HashMap<&str, usize> = steps.iter().map(|s| (s.id.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for step in steps {
        for dep in &step.depends_on {
            *in_degree.entry(step.id.as_str()).or_insert(0) += 1;
            adj.entry(dep.as_str()).or_default().push(step.id.as_str());
        }
    }

    let mut queue: std::collections::VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();

    let step_map: HashMap<&str, &StepDefinition> = steps.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut result = Vec::new();

    while let Some(id) = queue.pop_front() {
        if let Some(step) = step_map.get(id) {
            result.push(*step);
        }
        if let Some(dependents) = adj.get(id) {
            for dep in dependents {
                let entry = in_degree.entry(dep).or_default();
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_steps(deps: &[(&str, &[&str])]) -> Vec<StepDefinition> {
        deps.iter()
            .map(|(id, ds)| StepDefinition {
                id: id.to_string(),
                description: id.to_string(),
                depends_on: ds.iter().map(|d| d.to_string()).collect(),
            })
            .collect()
    }

    #[test]
    fn validate_acyclic_steps_ok() {
        let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["a", "b"])]);
        assert!(validate_steps(&steps).is_ok());
    }

    #[test]
    fn validate_cyclic_steps_error() {
        let steps = make_steps(&[("a", &["b"]), ("b", &["a"])]);
        assert!(matches!(validate_steps(&steps), Err(ConnectorError::DependencyCycle { .. })));
    }

    #[test]
    fn topological_order_respects_dependencies() {
        let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let order: Vec<&str> = topological_order(&steps).iter().map(|s| s.id.as_str()).collect();
        let a = order.iter().position(|&s| s == "a").unwrap();
        let b = order.iter().position(|&s| s == "b").unwrap();
        let c = order.iter().position(|&s| s == "c").unwrap();
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn topological_waves_no_deps_single_wave() {
        let steps = make_steps(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let waves = topological_waves(&steps);
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 3);
    }

    #[test]
    fn topological_waves_chain_three_waves() {
        let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let waves = topological_waves(&steps);
        assert_eq!(waves.len(), 3);
        assert!(waves[0].contains(&"a".to_string()));
        assert!(waves[1].contains(&"b".to_string()));
        assert!(waves[2].contains(&"c".to_string()));
    }

    #[test]
    fn topological_waves_diamond_two_middle_parallel() {
        // a → b, a → c, b → d, c → d
        let steps = make_steps(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        let waves = topological_waves(&steps);
        // a: wave 0, b+c: wave 1, d: wave 2
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["a"]);
        assert_eq!(waves[2], vec!["d"]);
        let mid: std::collections::HashSet<_> = waves[1].iter().collect();
        assert!(mid.contains(&"b".to_string()));
        assert!(mid.contains(&"c".to_string()));
    }

    #[test]
    fn topological_waves_empty_returns_empty() {
        assert!(topological_waves(&[]).is_empty());
    }

    #[test]
    fn step_context_emit_entity() {
        let mut ctx = StepContext::new("aws", "acme", "sync-1", Arc::new(PriorStepData::default()));
        let result = ctx.emit_entity(crate::builder::entity("host", "h1").class("Host"));
        assert!(result.is_ok());
        assert_eq!(ctx.metrics.entities_emitted, 1);
        assert_eq!(ctx.entities.len(), 1);
    }

    #[test]
    fn step_context_validation_rejects_empty_type() {
        let mut ctx = StepContext::new("aws", "acme", "sync-1", Arc::new(PriorStepData::default()));
        let result = ctx.emit_entity(crate::builder::entity("", "h1"));
        assert!(result.is_err());
    }
}
