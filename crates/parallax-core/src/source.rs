//! Source tracking — provenance metadata for every entity and relationship.
//!
//! **Spec reference:** `specs/01-data-model.md` §1.7

use compact_str::CompactString;
use serde::{Deserialize, Serialize};

use crate::timestamp::Timestamp;

/// Tracks which connector produced this data and when.
///
/// INV-C02: Entities from connector A are never deleted by a sync from connector B.
/// The SourceTag enables source-scoped diffing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceTag {
    /// Unique identifier for the connector instance.
    pub connector_id: CompactString,
    /// Unique identifier for the specific sync run.
    pub sync_id: CompactString,
    /// When this sync started.
    pub sync_timestamp: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_source_is_empty() {
        let s = SourceTag::default();
        assert!(s.connector_id.is_empty());
        assert!(s.sync_id.is_empty());
    }
}
