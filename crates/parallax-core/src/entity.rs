//! Entity types — the nodes in the Parallax graph.
//!
//! **Spec reference:** `specs/01-data-model.md` §1.2–§1.4

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

use crate::property::Value;
use crate::source::SourceTag;
use crate::timestamp::Timestamp;

/// 16-byte entity identity, derived deterministically from
/// `(account_id, entity_type, entity_key)` via blake3.
///
/// INV-01: EntityId is deterministic — same inputs always produce the same ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct EntityId(pub [u8; 16]);

impl EntityId {
    /// Derive an EntityId from its constituent parts.
    ///
    /// ```text
    /// id = blake3(account_id || '\0' || entity_type || '\0' || entity_key)[..16]
    /// ```
    pub fn derive(account_id: &str, entity_type: &str, entity_key: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(account_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(entity_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(entity_key.as_bytes());
        let hash = hasher.finalize();
        let mut id = [0u8; 16];
        id.copy_from_slice(&hash.as_bytes()[..16]);
        EntityId(id)
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}


/// Entity type — the specific asset kind (e.g., `aws_ec2_instance`).
/// Always lowercase, underscore-separated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityType(CompactString);

impl EntityType {
    /// Create without validation. Use in trusted internal paths only.
    pub fn new_unchecked(s: &str) -> Self {
        EntityType(CompactString::new(s))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// The curated, closed set of entity classes (spec §1.3).
///
/// New classes require a spec change. Connectors must map their assets
/// to one of these classes at ingest time.
pub const KNOWN_CLASSES: &[&str] = &[
    "Host", "User", "DataStore", "CodeRepo", "Firewall", "AccessPolicy",
    "NetworkSegment", "Service", "Certificate", "Secret", "Credential", "Key",
    "Container", "Pod", "Cluster", "Namespace", "Function", "Queue", "Topic",
    "Database", "Application", "Package", "Vulnerability", "Identity", "Process",
    "File", "Registry", "Policy", "Account", "Organization", "Team", "Role",
    "Group", "Device", "Endpoint", "Scanner", "Agent", "Sensor", "Ticket",
    "Event", "Generic",
];

/// Entity class — the broad category (e.g., `Host`, `DataStore`, `User`).
/// PascalCase by convention. The class is a **closed set** (see `KNOWN_CLASSES`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityClass(CompactString);

impl EntityClass {
    /// Validate against the known class set (spec §1.3).
    ///
    /// Returns `Err` if the class is not in `KNOWN_CLASSES`.
    pub fn new(s: &str) -> Result<Self, crate::error::CoreError> {
        if KNOWN_CLASSES.contains(&s) {
            Ok(EntityClass(CompactString::new(s)))
        } else {
            Err(crate::error::CoreError::InvalidEntityClass {
                value: s.to_owned(),
                reason: "not in the curated class set (see KNOWN_CLASSES)".to_owned(),
            })
        }
    }

    /// Create without validation. Use only in trusted internal paths
    /// (tombstones, deserialization, tests).
    pub fn new_unchecked(s: &str) -> Self {
        EntityClass(CompactString::new(s))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for EntityClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// A graph entity (node). Represents a single cyber asset.
///
/// INV-02: Every entity has exactly one _type and one _class.
/// INV-03: Properties are flat — no nested objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub _type: EntityType,
    pub _class: EntityClass,
    pub display_name: CompactString,
    pub properties: BTreeMap<CompactString, Value>,
    pub source: SourceTag,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub _deleted: bool,
}

impl Entity {
    /// Create a tombstone marker for deletion tracking.
    pub fn tombstone(id: EntityId) -> Self {
        Entity {
            id,
            _type: EntityType::new_unchecked("_tombstone"),
            _class: EntityClass::new_unchecked("_Tombstone"),
            display_name: CompactString::default(),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::now(),
            _deleted: true,
        }
    }

    /// Check if this entity is a deletion tombstone.
    pub fn is_tombstone(&self) -> bool {
        self._deleted
    }

    /// Approximate size in bytes (for MemTable flush threshold).
    pub fn approx_size(&self) -> usize {
        16 // id
        + self._type.as_str().len()
        + self._class.as_str().len()
        + self.display_name.len()
        + self.properties.iter().map(|(k, v)| k.len() + v.approx_size()).sum::<usize>()
        + 64 // source + timestamps + overhead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_is_deterministic() {
        let id1 = EntityId::derive("acct-1", "aws_ec2_instance", "i-0abc123");
        let id2 = EntityId::derive("acct-1", "aws_ec2_instance", "i-0abc123");
        assert_eq!(id1, id2);
    }

    #[test]
    fn entity_id_differs_for_different_inputs() {
        let id1 = EntityId::derive("acct-1", "aws_ec2_instance", "i-0abc123");
        let id2 = EntityId::derive("acct-1", "aws_ec2_instance", "i-0abc456");
        assert_ne!(id1, id2);
    }

    #[test]
    fn entity_id_display_is_hex() {
        let id = EntityId([0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(format!("{id}"), "deadbeef000000000000000000000000");
    }

    #[test]
    fn tombstone_is_deleted() {
        let t = Entity::tombstone(EntityId::default());
        assert!(t.is_tombstone());
    }
}
