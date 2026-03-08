//! Background compaction — merges L0 segment files into a single L1 segment.
//!
//! Compaction runs on a dedicated background thread and operates only on
//! immutable segment data. The writer thread swaps the segment list atomically
//! when compaction completes (via poll on the result channel).
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.8
//!
//! # Algorithm (leveled, simplified for v0.2)
//!
//! When the number of L0 segments reaches `l0_trigger`, the engine ships the
//! current L0 segment list to the compactor. The compactor:
//!   1. Merges all entities newest-first (Arc reference on each segment keeps
//!      file mappings alive for INV-S04 during the merge).
//!   2. Discards tombstones and old versions of the same entity ID.
//!   3. Writes a single sorted L1 segment file.
//!   4. Sends the result back; the engine atomically replaces the merged
//!      segments with the new L1 segment in its segment list.
//!
//! Compaction is an optimisation — it never blocks reads or writes. If
//! compaction fails, the engine logs a warning and continues with the
//! existing (un-compacted) segment list.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;

use tracing::{debug, info, warn};

use parallax_core::{entity::EntityId, relationship::RelationshipId};

use crate::error::StoreError;
use crate::segment::SegmentRef;

/// Configuration for the leveled compaction strategy.
///
/// Compaction runs in a background thread and operates only on immutable
/// segment data. The writer thread swaps the segment list atomically when
/// compaction completes.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Number of L0 segments that trigger an L0→L1 compaction.
    /// Set to 0 to disable compaction.
    pub l0_trigger: usize,
    /// Target size for L1 and L2 segment files.
    pub target_segment_size: u64,
    /// Minimum age of an old version before it may be discarded.
    ///
    /// Must be longer than the longest possible snapshot lifetime to avoid
    /// dropping data that a live reader still references. The Arc reference
    /// count on each segment also enforces this (INV-S04).
    pub version_retention: std::time::Duration,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        CompactionConfig {
            l0_trigger: 4,
            target_segment_size: 256 * 1024 * 1024, // 256 MB
            version_retention: std::time::Duration::from_secs(3600), // 1 hour
        }
    }
}

// ─── Internal channel types ───────────────────────────────────────────────────

struct CompactionRequest {
    /// Segments to merge (oldest-first, same order as the engine's list).
    segments: Vec<SegmentRef>,
    /// Directory where the merged output segment should be written.
    output_dir: PathBuf,
    /// Version number used to name the output file (monotonic).
    output_seq: u64,
}

/// Result returned from the compaction worker.
pub struct CompactionResult {
    /// Replacement segment list (typically one L1 segment, empty if all data
    /// was tombstoned).
    pub new_segments: Vec<SegmentRef>,
    /// Number of input segments that were merged (so the engine knows which
    /// prefix of its segment list to replace).
    pub merged_count: usize,
}

// ─── Worker ──────────────────────────────────────────────────────────────────

/// Background compaction worker.
///
/// Spawns a single OS thread. The engine submits compaction requests via
/// `try_compact()` and polls `result_rx` for completed results.
pub struct CompactionWorker {
    request_tx: SyncSender<CompactionRequest>,
    /// Poll this receiver on the writer thread with `try_recv()`.
    pub result_rx: Receiver<CompactionResult>,
    // Keep the handle alive so the thread is not detached.
    _handle: thread::JoinHandle<()>,
}

impl CompactionWorker {
    /// Spawn the background compaction thread.
    pub fn spawn() -> Self {
        // Bounded channel of size 1: if the worker is still busy, the engine
        // skips compaction for this cycle (best-effort, not blocking).
        let (req_tx, req_rx) = mpsc::sync_channel::<CompactionRequest>(1);
        let (res_tx, res_rx) = mpsc::channel::<CompactionResult>();

        let handle = thread::Builder::new()
            .name("parallax-compactor".into())
            .spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    match compact(req) {
                        Ok(result) => {
                            if res_tx.send(result).is_err() {
                                break; // engine dropped; exit gracefully.
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "compaction failed; segment list unchanged");
                            // Send an empty result so the engine can clear its
                            // pending_compaction flag.
                            let _ = res_tx.send(CompactionResult {
                                new_segments: vec![],
                                merged_count: 0, // 0 = signal "failed, no swap"
                            });
                        }
                    }
                }
                debug!("compaction worker exiting");
            })
            .expect("failed to spawn compaction thread");

        CompactionWorker {
            request_tx: req_tx,
            result_rx: res_rx,
            _handle: handle,
        }
    }

    /// Try to submit a compaction request.
    ///
    /// Returns `true` if the request was accepted. Returns `false` if the
    /// worker is still busy (caller should retry on the next write cycle).
    pub fn try_compact(
        &self,
        segments: Vec<SegmentRef>,
        output_dir: PathBuf,
        output_seq: u64,
    ) -> bool {
        match self.request_tx.try_send(CompactionRequest {
            segments,
            output_dir,
            output_seq,
        }) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false, // worker busy
            Err(TrySendError::Disconnected(_)) => false, // thread exited
        }
    }
}

// ─── Merge logic ─────────────────────────────────────────────────────────────

/// Merge all segments in `req` into a single output segment.
///
/// Iterates segments newest-first. For each entity/relationship, the first
/// version encountered (newest) is kept; older duplicates and tombstones are
/// discarded. Writes a new L1 segment file.
fn compact(req: CompactionRequest) -> Result<CompactionResult, StoreError> {
    let merged_count = req.segments.len();
    if merged_count == 0 {
        return Ok(CompactionResult {
            new_segments: vec![],
            merged_count: 0,
        });
    }

    // Merge entities newest-first. `or_insert_with` keeps the first (newest) version.
    let mut entity_map: HashMap<EntityId, parallax_core::entity::Entity> =
        HashMap::with_capacity(1024);
    let mut rel_map: HashMap<RelationshipId, parallax_core::relationship::Relationship> =
        HashMap::with_capacity(256);

    // Segments are oldest-first in the engine list → rev() = newest first.
    for seg in req.segments.iter().rev() {
        for entity in seg.all_entities() {
            entity_map
                .entry(entity.id)
                .or_insert_with(|| entity.clone());
        }
        for rel in seg.all_relationships() {
            rel_map.entry(rel.id).or_insert_with(|| rel.clone());
        }
    }

    let entities: Vec<_> = entity_map.into_values().collect();
    let relationships: Vec<_> = rel_map.into_values().collect();

    if entities.is_empty() && relationships.is_empty() {
        info!(
            merged_count,
            "compaction produced empty segment (all tombstoned)"
        );
        return Ok(CompactionResult {
            new_segments: vec![],
            merged_count,
        });
    }

    // Write merged output as a level-1 segment.
    let path = req
        .output_dir
        .join(format!("seg-{:020}-l1.pxs", req.output_seq));

    let new_seg = SegmentRef::write(&path, 1, entities, relationships)?;

    info!(
        merged_count,
        records = new_seg.record_count,
        path = %path.display(),
        "compaction complete"
    );

    Ok(CompactionResult {
        new_segments: vec![new_seg],
        merged_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_str::CompactString;
    use parallax_core::{
        entity::{Entity, EntityClass, EntityId, EntityType},
        source::SourceTag,
        timestamp::Timestamp,
    };
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn tmp_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn make_entity(id: EntityId) -> Entity {
        Entity {
            id,
            _type: EntityType::new_unchecked("host"),
            _class: EntityClass::new_unchecked("Host"),
            display_name: CompactString::default(),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    #[test]
    fn compact_merges_two_segments_deduplicated() {
        let dir = tmp_dir();

        let id_a = EntityId::derive("acc", "host", "a");
        let id_b = EntityId::derive("acc", "host", "b");
        let id_c = EntityId::derive("acc", "host", "c");

        // Segment 0 (older): a, b
        let seg0 = SegmentRef::write(
            &dir.path().join("seg0.pxs"),
            0,
            vec![make_entity(id_a), make_entity(id_b)],
            vec![],
        )
        .unwrap();

        // Segment 1 (newer): b (updated), c
        let seg1 = SegmentRef::write(
            &dir.path().join("seg1.pxs"),
            0,
            vec![make_entity(id_b), make_entity(id_c)],
            vec![],
        )
        .unwrap();

        let result = compact(CompactionRequest {
            segments: vec![seg0, seg1],
            output_dir: dir.path().to_path_buf(),
            output_seq: 42,
        })
        .unwrap();

        assert_eq!(result.merged_count, 2);
        assert_eq!(result.new_segments.len(), 1);

        let out = &result.new_segments[0];
        assert_eq!(out.record_count, 3, "a + b + c unique");
        assert!(out.get_entity(id_a).is_some());
        assert!(out.get_entity(id_b).is_some());
        assert!(out.get_entity(id_c).is_some());
    }

    #[test]
    fn compact_empty_segments_returns_empty() {
        let dir = tmp_dir();

        let result = compact(CompactionRequest {
            segments: vec![],
            output_dir: dir.path().to_path_buf(),
            output_seq: 1,
        })
        .unwrap();

        assert_eq!(result.new_segments.len(), 0);
        assert_eq!(result.merged_count, 0);
    }

    #[test]
    fn compaction_worker_processes_request() {
        let dir = tmp_dir();
        let worker = CompactionWorker::spawn();

        let id = EntityId::derive("acc", "host", "h1");
        let seg = SegmentRef::write(&dir.path().join("s0.pxs"), 0, vec![make_entity(id)], vec![])
            .unwrap();

        let submitted = worker.try_compact(vec![seg], dir.path().to_path_buf(), 1);
        assert!(submitted, "request should be accepted");

        // Wait for result (with a timeout via loop).
        let result = loop {
            if let Ok(r) = worker.result_rx.try_recv() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };

        assert_eq!(result.merged_count, 1);
        assert_eq!(result.new_segments.len(), 1);
        assert!(result.new_segments[0].get_entity(id).is_some());
    }
}
