//! Entity and relationship builders for connector authors.
//!
//! Connectors describe what they found using these fluent builders.
//! The SDK handles ID derivation, timestamps, and source tagging.
//!
//! **Spec reference:** `specs/05-integration-sdk.md` §5.4

use std::collections::BTreeMap;

use compact_str::CompactString;
use parallax_core::{
    entity::{Entity, EntityClass, EntityId, EntityType},
    property::Value,
    relationship::{Relationship, RelationshipClass, RelationshipId},
    source::SourceTag,
    timestamp::Timestamp,
};

// ─── Entity builder ───────────────────────────────────────────────────────────

/// Fluent entity builder. Connector authors call `entity(type, key)` to start.
///
/// Turon: "Make illegal states unrepresentable." The builder can't be used
/// without a type and key — the two components required for ID derivation.
#[derive(Debug, Clone)]
pub struct EntityBuilder {
    pub(crate) entity_type: String,
    pub(crate) entity_key: String,
    pub(crate) entity_class: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) properties: Vec<(String, Value)>,
}

/// Start building an entity.
///
/// `entity_type` is the Parallax type (e.g. `"aws_ec2_instance"`).
/// `entity_key` is the source system's unique identifier for this entity.
pub fn entity(entity_type: &str, entity_key: &str) -> EntityBuilder {
    EntityBuilder {
        entity_type: entity_type.to_owned(),
        entity_key: entity_key.to_owned(),
        entity_class: None,
        display_name: None,
        properties: Vec::new(),
    }
}

impl EntityBuilder {
    /// Set the entity class (e.g. `"Host"`, `"User"`, `"DataStore"`).
    pub fn class(mut self, class: &str) -> Self {
        self.entity_class = Some(class.to_owned());
        self
    }

    /// Set the human-readable display name.
    pub fn display_name(mut self, name: &str) -> Self {
        self.display_name = Some(name.to_owned());
        self
    }

    /// Add a property. `value` can be any type that converts to `Value`.
    pub fn property(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.properties.push((key.to_owned(), value.into()));
        self
    }

    /// Materialize into an `Entity` with a derived ID and source tag.
    ///
    /// `account_id` scopes the entity to a specific account.
    pub fn build(self, account_id: &str, source: SourceTag) -> Entity {
        let id = EntityId::derive(account_id, &self.entity_type, &self.entity_key);
        let now = Timestamp::now();

        let class = self.entity_class.unwrap_or_else(|| {
            // Default class: PascalCase from snake_case type (heuristic).
            // e.g. "aws_ec2_instance" → "AwsEc2Instance"
            // Connector authors should always set .class() explicitly.
            self.entity_type
                .split('_')
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        });

        let mut properties = BTreeMap::new();
        for (k, v) in self.properties {
            properties.insert(CompactString::new(&k), v);
        }

        Entity {
            id,
            _type: EntityType::new_unchecked(&self.entity_type),
            _class: EntityClass::new_unchecked(&class),
            display_name: CompactString::new(
                self.display_name.as_deref().unwrap_or(&self.entity_key),
            ),
            properties,
            source,
            created_at: now,
            updated_at: now,
            _deleted: false,
        }
    }
}

// ─── Relationship builder ─────────────────────────────────────────────────────

/// Fluent relationship builder.
#[derive(Debug, Clone)]
pub struct RelationshipBuilder {
    pub(crate) from_type: Option<String>,
    pub(crate) from_key: String,
    pub(crate) verb: String,
    pub(crate) to_type: Option<String>,
    pub(crate) to_key: String,
    pub(crate) properties: Vec<(String, Value)>,
}

/// Start building a relationship.
///
/// `from_key` and `to_key` are the entity keys in the source system.
/// `verb` is the relationship class (e.g. `"HAS"`, `"USES"`, `"ASSIGNS"`).
pub fn relationship(from_key: &str, verb: &str, to_key: &str) -> RelationshipBuilder {
    RelationshipBuilder {
        from_type: None,
        from_key: from_key.to_owned(),
        verb: verb.to_owned(),
        to_type: None,
        to_key: to_key.to_owned(),
        properties: Vec::new(),
    }
}

impl RelationshipBuilder {
    /// Set the source entity's type.
    pub fn from_type(mut self, t: &str) -> Self {
        self.from_type = Some(t.to_owned());
        self
    }

    /// Set the target entity's type.
    pub fn to_type(mut self, t: &str) -> Self {
        self.to_type = Some(t.to_owned());
        self
    }

    /// Add a property to the relationship.
    pub fn property(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.properties.push((key.to_owned(), value.into()));
        self
    }

    /// Materialize into a `Relationship` with derived IDs and source tag.
    pub fn build(self, account_id: &str, source: SourceTag) -> Option<Relationship> {
        let from_type = self.from_type.as_deref()?;
        let to_type = self.to_type.as_deref()?;

        let from_id = EntityId::derive(account_id, from_type, &self.from_key);
        let to_id = EntityId::derive(account_id, to_type, &self.to_key);
        let rel_id = RelationshipId::derive(
            account_id,
            from_type,
            &self.from_key,
            &self.verb,
            to_type,
            &self.to_key,
        );
        let now = Timestamp::now();

        let mut properties = BTreeMap::new();
        for (k, v) in self.properties {
            properties.insert(CompactString::new(&k), v);
        }

        Some(Relationship {
            id: rel_id,
            from_id,
            to_id,
            _class: RelationshipClass::new_unchecked(&self.verb),
            properties,
            source,
            created_at: now,
            updated_at: now,
            _deleted: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_source() -> SourceTag {
        SourceTag {
            connector_id: CompactString::new("test"),
            sync_id: CompactString::new("s1"),
            sync_timestamp: Timestamp::default(),
        }
    }

    #[test]
    fn entity_builder_derives_id() {
        let e = entity("host", "h1").class("Host").build("acme", test_source());
        assert_eq!(e.id, EntityId::derive("acme", "host", "h1"));
        assert_eq!(e._type.as_str(), "host");
        assert_eq!(e._class.as_str(), "Host");
    }

    #[test]
    fn entity_default_class_from_type() {
        let e = entity("aws_ec2_instance", "i-123").build("acme", test_source());
        // Default class derived from type: "AwsEc2Instance"
        assert_eq!(e._class.as_str(), "AwsEc2Instance");
    }

    #[test]
    fn entity_builder_properties() {
        let e = entity("host", "h1")
            .property("state", "running")
            .property("active", true)
            .build("acme", test_source());
        use parallax_core::property::Value;
        assert_eq!(e.properties.get("state"), Some(&Value::from("running")));
        assert_eq!(e.properties.get("active"), Some(&Value::Bool(true)));
    }

    #[test]
    fn relationship_builder_derives_ids() {
        let r = relationship("h1", "CONNECTS", "s1")
            .from_type("host")
            .to_type("service")
            .build("acme", test_source())
            .expect("build");
        assert_eq!(r.from_id, EntityId::derive("acme", "host", "h1"));
        assert_eq!(r.to_id, EntityId::derive("acme", "service", "s1"));
        assert_eq!(r._class.as_str(), "CONNECTS");
    }

    #[test]
    fn relationship_builder_without_types_returns_none() {
        // Missing from_type and to_type → can't derive IDs.
        let r = relationship("h1", "CONNECTS", "s1").build("acme", test_source());
        assert!(r.is_none());
    }
}
