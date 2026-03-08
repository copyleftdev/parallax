//! Entity finder — filtered lookup over a snapshot.
//!
//! `EntityFinder` is a fluent builder that narrows a result set using
//! the storage-layer indices (type, class) and post-index property filters.
//!
//! **Spec reference:** `specs/03-graph-engine.md` §3.3

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityType},
    property::Value,
};
use parallax_store::Snapshot;
use tracing::warn;

/// Comparison operator for numeric/ordered property filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Lte,
    Gt,
    Gte,
    Ne,
}

impl CmpOp {
    fn compare(self, actual: &Value, expected: &Value) -> bool {
        match (self, actual, expected) {
            (CmpOp::Ne, a, b) => a != b,
            (CmpOp::Lt, Value::Int(a), Value::Int(b)) => a < b,
            (CmpOp::Lte, Value::Int(a), Value::Int(b)) => a <= b,
            (CmpOp::Gt, Value::Int(a), Value::Int(b)) => a > b,
            (CmpOp::Gte, Value::Int(a), Value::Int(b)) => a >= b,
            (CmpOp::Lt, Value::Float(a), Value::Float(b)) => a < b,
            (CmpOp::Lte, Value::Float(a), Value::Float(b)) => a <= b,
            (CmpOp::Gt, Value::Float(a), Value::Float(b)) => a > b,
            (CmpOp::Gte, Value::Float(a), Value::Float(b)) => a >= b,
            _ => false,
        }
    }
}

/// A predicate applied to an entity's property map.
#[derive(Debug, Clone)]
pub enum PropertyFilter {
    /// Property equals the given value.
    Eq(CompactString, Value),
    /// Property key exists (any value including Null).
    Exists(CompactString),
    /// Property satisfies an ordered comparison.
    Cmp(CompactString, CmpOp, Value),
    /// String property contains the given substring.
    Contains(CompactString, CompactString),
    /// String property starts with the given prefix.
    StartsWith(CompactString, CompactString),
    /// Property value is in the given set.
    In(CompactString, Vec<Value>),
}

impl PropertyFilter {
    /// Returns `true` if the entity satisfies this filter.
    pub fn matches(&self, entity: &Entity) -> bool {
        match self {
            PropertyFilter::Eq(key, expected) => {
                entity.properties.get(key.as_str()) == Some(expected)
            }
            PropertyFilter::Exists(key) => entity.properties.contains_key(key.as_str()),
            PropertyFilter::Cmp(key, op, expected) => entity
                .properties
                .get(key.as_str())
                .map(|v| op.compare(v, expected))
                .unwrap_or(false),
            PropertyFilter::Contains(key, substr) => entity
                .properties
                .get(key.as_str())
                .and_then(|v| v.as_str())
                .map(|s| s.contains(substr.as_str()))
                .unwrap_or(false),
            PropertyFilter::StartsWith(key, prefix) => entity
                .properties
                .get(key.as_str())
                .and_then(|v| v.as_str())
                .map(|s| s.starts_with(prefix.as_str()))
                .unwrap_or(false),
            PropertyFilter::In(key, values) => entity
                .properties
                .get(key.as_str())
                .map(|v| values.contains(v))
                .unwrap_or(false),
        }
    }
}

/// Fluent builder for finding entities in a snapshot.
///
/// Call `.collect()` or iterate with `for ... in finder.into_iter()`.
///
/// Access strategy (in priority order):
/// 1. entity_type set → type index (O(|type|))
/// 2. entity_class set → class index (O(|class|))
/// 3. neither → full scan (logged at WARN)
///
/// INV-G01: Results only contain entities that exist in the snapshot.
pub struct EntityFinder<'snap> {
    snapshot: &'snap Snapshot,
    pub(crate) entity_type: Option<EntityType>,
    pub(crate) entity_class: Option<EntityClass>,
    pub(crate) filters: Vec<PropertyFilter>,
    limit: Option<usize>,
}

impl<'snap> EntityFinder<'snap> {
    /// Create a finder scoped to a specific entity type.
    pub fn new(snapshot: &'snap Snapshot, entity_type: &str) -> Self {
        EntityFinder {
            snapshot,
            entity_type: Some(EntityType::new_unchecked(entity_type)),
            entity_class: None,
            filters: Vec::new(),
            limit: None,
        }
    }

    /// Create a finder with no initial type constraint (full scan).
    pub fn new_untyped(snapshot: &'snap Snapshot) -> Self {
        EntityFinder {
            snapshot,
            entity_type: None,
            entity_class: None,
            filters: Vec::new(),
            limit: None,
        }
    }

    /// Narrow to a specific entity class.
    pub fn class(mut self, class: &str) -> Self {
        self.entity_class = Some(EntityClass::new_unchecked(class));
        self
    }

    /// Filter by exact property value.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.filters
            .push(PropertyFilter::Eq(key.into(), value.into()));
        self
    }

    /// Filter by property existence.
    pub fn has(mut self, key: &str) -> Self {
        self.filters.push(PropertyFilter::Exists(key.into()));
        self
    }

    /// Add a pre-built property filter directly.
    pub fn has_filter(mut self, f: PropertyFilter) -> Self {
        self.filters.push(f);
        self
    }

    /// Filter by ordered comparison.
    pub fn with_cmp(mut self, key: &str, op: CmpOp, value: impl Into<Value>) -> Self {
        self.filters
            .push(PropertyFilter::Cmp(key.into(), op, value.into()));
        self
    }

    /// Limit the number of results.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Execute and collect all matching entities.
    pub fn collect(self) -> Vec<&'snap Entity> {
        self.into_iter().collect()
    }

    /// Execute lazily — returns an iterator.
    #[allow(clippy::should_implement_trait)]
    pub fn into_iter(self) -> impl Iterator<Item = &'snap Entity> + 'snap {
        let candidates: Vec<&'snap Entity> = if let Some(ref et) = self.entity_type {
            self.snapshot.entities_of_type(et)
        } else if let Some(ref ec) = self.entity_class {
            self.snapshot.entities_of_class(ec.as_str())
        } else {
            warn!("EntityFinder: no type or class constraint — performing full entity scan");
            self.snapshot.all_entities()
        };

        // Apply optional class narrowing when type was the primary key.
        let class_filter = self.entity_class.clone();
        let filters = self.filters;
        let limit = self.limit.unwrap_or(usize::MAX);

        candidates
            .into_iter()
            .filter(move |e| {
                // Secondary class filter (when type was used as primary).
                if let Some(ref cls) = class_filter {
                    if &e._class != cls {
                        return false;
                    }
                }
                // Property filters.
                filters.iter().all(|f| f.matches(e))
            })
            .take(limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::make_graph;
    use parallax_core::property::Value;

    #[test]
    fn find_by_type() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1").host("a", "h2").service("a", "s1");
        });
        let snap = engine.snapshot();
        let found = EntityFinder::new(&snap, "host").collect();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn find_by_type_and_property() {
        let (engine, _dir) = make_graph(|b| {
            b.host_with("a", "h1", "state", "running")
                .host_with("a", "h2", "state", "stopped");
        });
        let snap = engine.snapshot();
        let found = EntityFinder::new(&snap, "host")
            .with("state", "running")
            .collect();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn find_with_limit() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1").host("a", "h2").host("a", "h3");
        });
        let snap = engine.snapshot();
        let found = EntityFinder::new(&snap, "host").limit(2).collect();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn find_by_class() {
        let (engine, _dir) = make_graph(|b| {
            b.host("a", "h1").service("a", "s1");
        });
        let snap = engine.snapshot();
        let found = EntityFinder::new_untyped(&snap).class("Host").collect();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn cmp_op_lt() {
        let (engine, _dir) = make_graph(|b| {
            b.host_with("a", "h1", "port", 80i64)
                .host_with("a", "h2", "port", 443i64);
        });
        let snap = engine.snapshot();
        let found = EntityFinder::new(&snap, "host")
            .with_cmp("port", CmpOp::Lt, Value::Int(443))
            .collect();
        assert_eq!(found.len(), 1);
    }
}
