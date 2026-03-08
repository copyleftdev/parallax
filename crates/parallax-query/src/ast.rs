//! PQL abstract syntax tree.
//!
//! Every variant is structurally valid. If it parses, it's well-formed.
//! Semantic resolution (type vs class) happens in the planner.
//!
//! **Spec reference:** `specs/04-query-language.md` §4.4

use parallax_core::{
    entity::{Entity, EntityClass, EntityType},
    property::Value,
    relationship::Direction,
};

// ─── Top-level query ─────────────────────────────────────────────────────────

/// A parsed PQL query.
#[derive(Debug, Clone)]
pub enum Query {
    Find(FindQuery),
    ShortestPath(PathQuery),
    BlastRadius(BlastQuery),
}

// ─── FIND query ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FindQuery {
    pub entity: EntityFilter,
    pub property_filters: Vec<PropertyCondition>,
    pub traversals: Vec<TraversalStep>,
    pub group_by: Option<GroupByClause>,
    pub return_clause: Option<ReturnClause>,
    pub limit: Option<usize>,
}

/// `GROUP BY field` — aggregate results by a property value.
#[derive(Debug, Clone)]
pub struct GroupByClause {
    /// The property key to group on.
    pub field: String,
}

// ─── SHORTEST PATH query ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PathQuery {
    pub from: EntityFilter,
    pub from_filters: Vec<PropertyCondition>,
    pub to: EntityFilter,
    pub to_filters: Vec<PropertyCondition>,
    pub max_depth: Option<u32>,
}

// ─── BLAST RADIUS query ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BlastQuery {
    pub origin: EntityFilter,
    pub origin_filters: Vec<PropertyCondition>,
    pub max_depth: Option<u32>,
}

// ─── Entity filter ────────────────────────────────────────────────────────────

/// An entity type or class name from the FIND clause.
#[derive(Debug, Clone)]
pub struct EntityFilter {
    /// `None` = wildcard (`*`).
    pub name: Option<String>,
    /// Resolved by the planner against index statistics.
    pub kind: EntityFilterKind,
}

impl EntityFilter {
    pub fn wildcard() -> Self {
        EntityFilter { name: None, kind: EntityFilterKind::Unresolved }
    }

    pub fn named(name: String) -> Self {
        EntityFilter { name: Some(name), kind: EntityFilterKind::Unresolved }
    }

    /// Returns `true` if the entity matches this filter (by type or class name).
    pub fn matches(&self, entity: &Entity) -> bool {
        match (&self.name, &self.kind) {
            (None, _) => true,
            (Some(_), EntityFilterKind::Type(t)) => entity._type == *t,
            (Some(_), EntityFilterKind::Class(c)) => entity._class == *c,
            (Some(name), EntityFilterKind::Unresolved) => {
                entity._type.as_str() == name || entity._class.as_str() == name
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum EntityFilterKind {
    /// Not yet resolved — set during parsing, replaced by planner.
    Unresolved,
    Type(EntityType),
    Class(EntityClass),
}

// ─── Traversal step ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TraversalStep {
    /// `true` for `THAT !VERB` (coverage gap).
    pub negated: bool,
    pub verb: Verb,
    pub target: EntityFilter,
    pub property_filters: Vec<PropertyCondition>,
}

/// PQL relationship verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    Has,
    Is,
    Assigned,
    Allows,
    Uses,
    Contains,
    Manages,
    Connects,
    Protects,
    Exploits,
    Trusts,
    Scans,
    /// Wildcard: any relationship class.
    RelatesTo,
}

impl Verb {
    /// The relationship class string this verb maps to, or `None` for wildcard.
    pub fn edge_class(&self) -> Option<&'static str> {
        match self {
            Verb::Has => Some("HAS"),
            Verb::Is => Some("IS"),
            Verb::Assigned => Some("ASSIGNED"),
            Verb::Allows => Some("ALLOWS"),
            Verb::Uses => Some("USES"),
            Verb::Contains => Some("CONTAINS"),
            Verb::Manages => Some("MANAGES"),
            Verb::Connects => Some("CONNECTS"),
            Verb::Protects => Some("PROTECTS"),
            Verb::Exploits => Some("EXPLOITS"),
            Verb::Trusts => Some("TRUSTS"),
            Verb::Scans => Some("SCANS"),
            Verb::RelatesTo => None,
        }
    }

    /// The traversal direction for this verb when used in `FIND X THAT VERB Y`.
    ///
    /// Most verbs are directional: the query subject is the "from" end of the
    /// relationship (outgoing). CONNECTS and RELATES TO are bidirectional.
    pub fn direction(&self) -> Direction {
        match self {
            Verb::Connects | Verb::RelatesTo => Direction::Both,
            _ => Direction::Outgoing,
        }
    }
}

// ─── Property conditions ──────────────────────────────────────────────────────

/// A single condition on an entity's property map.
#[derive(Debug, Clone)]
pub enum PropertyCondition {
    Eq(String, Value),
    Ne(String, Value),
    Lt(String, Value),
    Lte(String, Value),
    Gt(String, Value),
    Gte(String, Value),
    In(String, Vec<Value>),
    Like(String, String),
    Exists(String),
    Not(Box<PropertyCondition>),
    /// Disjunction: at least one arm must match.
    Or(Vec<PropertyCondition>),
}

impl PropertyCondition {
    /// Returns `true` if the entity satisfies this condition.
    pub fn matches(&self, entity: &Entity) -> bool {
        match self {
            PropertyCondition::Eq(k, v) => entity.properties.get(k.as_str()) == Some(v),
            PropertyCondition::Ne(k, v) => entity.properties.get(k.as_str()) != Some(v),
            PropertyCondition::Lt(k, v) => {
                entity.properties.get(k.as_str()).map(|a| a < v).unwrap_or(false)
            }
            PropertyCondition::Lte(k, v) => {
                entity.properties.get(k.as_str()).map(|a| a <= v).unwrap_or(false)
            }
            PropertyCondition::Gt(k, v) => {
                entity.properties.get(k.as_str()).map(|a| a > v).unwrap_or(false)
            }
            PropertyCondition::Gte(k, v) => {
                entity.properties.get(k.as_str()).map(|a| a >= v).unwrap_or(false)
            }
            PropertyCondition::In(k, vals) => {
                entity.properties.get(k.as_str()).map(|v| vals.contains(v)).unwrap_or(false)
            }
            PropertyCondition::Like(k, pattern) => entity
                .properties
                .get(k.as_str())
                .and_then(|v| v.as_str())
                .map(|s| s.contains(pattern.as_str()))
                .unwrap_or(false),
            PropertyCondition::Exists(k) => entity.properties.contains_key(k.as_str()),
            PropertyCondition::Not(inner) => !inner.matches(entity),
            PropertyCondition::Or(arms) => arms.iter().any(|c| c.matches(entity)),
        }
    }
}

// ─── Return clause ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ReturnClause {
    Count,
    Fields(Vec<String>),
}
