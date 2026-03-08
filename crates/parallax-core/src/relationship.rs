//! Relationship types — the edges in the Parallax graph.
//!
//! **Spec reference:** `specs/01-data-model.md` §1.3

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

use crate::entity::EntityId;
use crate::property::Value;
use crate::source::SourceTag;
use crate::timestamp::Timestamp;

/// 16-byte relationship identity, derived from
/// `(account_id, from_type, from_key, verb, to_type, to_key)` via blake3.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct RelationshipId(pub [u8; 16]);

impl RelationshipId {
    pub fn derive(
        account_id: &str,
        from_type: &str,
        from_key: &str,
        verb: &str,
        to_type: &str,
        to_key: &str,
    ) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(account_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(from_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(from_key.as_bytes());
        hasher.update(b"\0");
        hasher.update(verb.as_bytes());
        hasher.update(b"\0");
        hasher.update(to_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(to_key.as_bytes());
        let hash = hasher.finalize();
        let mut id = [0u8; 16];
        id.copy_from_slice(&hash.as_bytes()[..16]);
        RelationshipId(id)
    }
}

impl fmt::Display for RelationshipId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The curated, closed set of relationship verbs (spec §1.4).
///
/// New verbs require a spec change to ensure query semantics remain
/// consistent across all connectors and entity types.
pub const KNOWN_VERBS: &[&str] = &[
    "HAS", "IS", "ASSIGNED", "ALLOWS", "USES", "CONTAINS", "MANAGES", "CONNECTS", "PROTECTS",
    "EXPLOITS", "TRUSTS", "SCANS",
    // Extended set — approved for v0.1 use by connectors:
    "RUNS", "READS", "WRITES",
];

/// Relationship class — the verb describing the edge (e.g., `ASSIGNED`, `ALLOWS`).
/// SCREAMING_SNAKE by convention. The class is a **closed set** (see `KNOWN_VERBS`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RelationshipClass(CompactString);

impl RelationshipClass {
    /// Validate against the known verb set (spec §1.4).
    ///
    /// Returns `Err` if the verb is not in `KNOWN_VERBS`.
    pub fn new(s: &str) -> Result<Self, crate::error::CoreError> {
        if KNOWN_VERBS.contains(&s) {
            Ok(RelationshipClass(CompactString::new(s)))
        } else {
            Err(crate::error::CoreError::InvalidRelationshipClass {
                value: s.to_owned(),
                reason: "not in the curated verb set (see KNOWN_VERBS)".to_owned(),
            })
        }
    }

    /// Create without validation. Use only in trusted internal paths
    /// (tombstones, deserialization, tests).
    pub fn new_unchecked(s: &str) -> Self {
        RelationshipClass(CompactString::new(s))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for RelationshipClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// A graph relationship (directed edge) between two entities.
///
/// INV-04: from_id and to_id must reference existing entities
///         (enforced at ingest time, not by this type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: RelationshipId,
    pub from_id: EntityId,
    pub to_id: EntityId,
    pub _class: RelationshipClass,
    pub properties: BTreeMap<CompactString, Value>,
    pub source: SourceTag,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub _deleted: bool,
}

impl Relationship {
    /// Create a tombstone marker for deletion tracking.
    pub fn tombstone(id: RelationshipId) -> Self {
        Relationship {
            id,
            from_id: EntityId::default(),
            to_id: EntityId::default(),
            _class: RelationshipClass::new_unchecked("_TOMBSTONE"),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::now(),
            _deleted: true,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        self._deleted
    }

    pub fn approx_size(&self) -> usize {
        32 // from_id + to_id
        + 16 // id
        + self._class.as_str().len()
        + self.properties.iter().map(|(k, v)| k.len() + v.approx_size()).sum::<usize>()
        + 64 // source + timestamps + overhead
    }
}

/// Edge direction for traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

impl Direction {
    pub fn matches(self, edge_dir: Direction) -> bool {
        match self {
            Direction::Both => true,
            other => other == edge_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_id_is_deterministic() {
        let id1 = RelationshipId::derive(
            "acct-1",
            "aws_vpc",
            "vpc-1",
            "HAS",
            "aws_ec2_instance",
            "i-1",
        );
        let id2 = RelationshipId::derive(
            "acct-1",
            "aws_vpc",
            "vpc-1",
            "HAS",
            "aws_ec2_instance",
            "i-1",
        );
        assert_eq!(id1, id2);
    }

    #[test]
    fn relationship_id_differs_for_different_verb() {
        let id1 = RelationshipId::derive(
            "acct-1",
            "aws_vpc",
            "vpc-1",
            "HAS",
            "aws_ec2_instance",
            "i-1",
        );
        let id2 = RelationshipId::derive(
            "acct-1",
            "aws_vpc",
            "vpc-1",
            "CONTAINS",
            "aws_ec2_instance",
            "i-1",
        );
        assert_ne!(id1, id2);
    }

    #[test]
    fn direction_both_matches_everything() {
        assert!(Direction::Both.matches(Direction::Outgoing));
        assert!(Direction::Both.matches(Direction::Incoming));
        assert!(Direction::Both.matches(Direction::Both));
    }
}
