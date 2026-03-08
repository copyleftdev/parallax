//! Storage engine — top-level coordinator.
//!
//! `StorageEngine` wires together the WAL, MemTable, and SnapshotManager.
//! It is the sole writer: `write()` takes `&mut self`.
//!
//! Readers get a snapshot with `snapshot()`, which is `&self` and
//! lock-free.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.3, §2.12
//!
//! INV-S01: WAL fsync before snapshot publish.
//! INV-S07: Only `write()` mutates the MemTable.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tracing::debug;

use crate::compaction::{CompactionConfig, CompactionWorker};
use crate::error::StoreError;
use crate::memtable::MemTable;
use crate::metrics::StoreMetrics;
use crate::segment::SegmentRef;
use crate::snapshot::{Snapshot, SnapshotManager};
use crate::wal::WriteAheadLog;
use crate::write_batch::WriteBatch;

/// Group-commit configuration.
///
/// For high-throughput ingestion, multiple WriteBatches can be coalesced
/// into a single WAL fsync (amortizing the ~100μs NVMe fsync overhead).
///
/// Use `StorageEngine::write_many()` to submit a group manually. The `delay`
/// and `max_batch` fields are provided for higher-level batching layers that
/// want to coalesce writes across time.
#[derive(Debug, Clone)]
pub struct GroupCommitConfig {
    /// Max time to wait for more batches before flushing.
    pub delay: Duration,
    /// Max batches to coalesce per group.
    pub max_batch: usize,
}

impl Default for GroupCommitConfig {
    fn default() -> Self {
        GroupCommitConfig {
            delay: Duration::from_millis(1),
            max_batch: 100,
        }
    }
}

/// Storage engine configuration.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Root directory for all storage files (`wal/` and `segments/` live here).
    pub data_dir: PathBuf,
    /// Maximum WAL segment file size before rotation.
    pub wal_segment_size: u64,
    /// Group-commit settings.
    pub group_commit: GroupCommitConfig,
    /// MemTable flush threshold: trigger a flush when MemTable exceeds this size.
    pub memtable_flush_size: u64,
    /// Compaction settings.
    pub compaction: CompactionConfig,
}

impl Default for StoreConfig {
    fn default() -> Self {
        StoreConfig {
            data_dir: PathBuf::from("parallax-data"),
            wal_segment_size: 64 * 1024 * 1024,       // 64 MB
            group_commit: GroupCommitConfig::default(),
            memtable_flush_size: 64 * 1024 * 1024,    // 64 MB
            compaction: CompactionConfig::default(),
        }
    }
}

impl StoreConfig {
    /// Create a config with the given data directory and all other settings
    /// at their defaults.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        StoreConfig {
            data_dir: data_dir.into(),
            ..Default::default()
        }
    }
}

/// The storage engine: WAL + MemTable + MVCC snapshots.
///
/// Single writer (`write()` takes `&mut self`), unlimited concurrent
/// readers (`snapshot()` takes `&self`, is lock-free).
pub struct StorageEngine {
    config: StoreConfig,
    wal: WriteAheadLog,
    memtable: MemTable,
    snapshot_mgr: Arc<SnapshotManager>,
    /// Current segment list. `Arc` so it can be shared with snapshots cheaply.
    segments: Arc<Vec<SegmentRef>>,
    /// Monotonically increasing version, incremented on every `write()`.
    version: u64,
    metrics: Arc<StoreMetrics>,
    /// Background compaction worker. Processes L0→L1 merges asynchronously.
    compaction_worker: CompactionWorker,
    /// True while a compaction request is in-flight (not yet received a result).
    pending_compaction: bool,
}

impl StorageEngine {
    /// Open an existing storage engine or create a new one at `config.data_dir`.
    ///
    /// On open, the WAL is scanned and all uncompacted batches are replayed
    /// into a fresh MemTable. This rebuilds the in-memory state deterministically.
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        let wal_dir = config.data_dir.join("wal");

        // Open WAL, recovering all batches written since last checkpoint.
        // v0.1: checkpoint_seq = 0 (no persistent checkpoint yet).
        let (wal, batches) = WriteAheadLog::open(&wal_dir, config.wal_segment_size, 0)?;

        // Rebuild MemTable from recovered batches.
        let mut memtable = MemTable::new();
        for batch in &batches {
            memtable.apply(batch);
        }

        let version = batches.len() as u64;
        let segments = Arc::new(Vec::<SegmentRef>::new());
        let metrics = Arc::new(StoreMetrics::new());

        // Publish initial snapshot.
        let initial_snapshot = Snapshot::new(version, memtable.as_arc_snapshot(), Arc::clone(&segments));
        let snapshot_mgr = Arc::new(SnapshotManager::new(initial_snapshot));

        Ok(StorageEngine {
            config,
            wal,
            memtable,
            snapshot_mgr,
            segments,
            version,
            metrics,
            compaction_worker: CompactionWorker::spawn(),
            pending_compaction: false,
        })
    }

    /// Write an atomic batch to the storage engine.
    ///
    /// Steps (in order, per spec):
    /// 1. Append to WAL + fsync (durability commit point — INV-S01).
    /// 2. Apply to MemTable (INV-S07: single writer).
    /// 3. Increment version (INV-S03: monotonic).
    /// 4. Publish new snapshot (INV-S01: after WAL write).
    ///
    /// Returns the WAL sequence number assigned to this batch.
    pub fn write(&mut self, batch: WriteBatch) -> Result<u64, StoreError> {
        // Poll for completed compaction results before processing the new write.
        self.poll_compaction_result();

        if batch.is_empty() {
            return Ok(self.version);
        }

        let batch_len = batch.len() as u64;

        // 1. WAL append + fsync (INV-S01).
        let seq = self.wal.append(&batch)?;
        self.metrics.wal_appends.fetch_add(1, Ordering::Relaxed);

        // 2. Apply to MemTable (INV-S07).
        self.memtable.apply(&batch);
        self.metrics
            .memtable_inserts
            .fetch_add(batch_len, Ordering::Relaxed);
        self.metrics
            .memtable_bytes
            .store(self.memtable.approx_bytes() as u64, Ordering::Relaxed);

        // 3. Increment version (INV-S03).
        self.version += 1;

        // 4. Publish new snapshot (INV-S01: after WAL fsync).
        let snapshot = Snapshot::new(
            self.version,
            self.memtable.as_arc_snapshot(),
            Arc::clone(&self.segments),
        );
        self.snapshot_mgr.publish(snapshot);
        self.metrics
            .snapshots_published
            .fetch_add(1, Ordering::Relaxed);

        debug!(
            seq,
            version = self.version,
            ops = batch_len,
            memtable_bytes = self.memtable.approx_bytes(),
            "write committed"
        );

        // Trigger MemTable flush if the size threshold is exceeded.
        if self.memtable.approx_bytes() as u64 > self.config.memtable_flush_size {
            self.flush_memtable()?;
        }

        Ok(seq)
    }

    /// Write multiple batches to the storage engine with a **single** WAL fsync.
    ///
    /// This is the group-commit path. All non-empty batches are serialized
    /// and appended contiguously to the WAL in one `write_all + sync_data`
    /// call, then applied to the MemTable in order. A single snapshot is
    /// published at the end.
    ///
    /// Throughput gain: amortizes the ~100μs NVMe fsync across N batches.
    /// At batch sizes of 100 the effective cost per batch drops to ~1μs.
    ///
    /// Returns the WAL sequence numbers assigned to each non-empty batch.
    ///
    /// INV-S01: single fsync covers all batches before snapshot publish.
    /// INV-S07: single writer; `&mut self` enforced by the type system.
    pub fn write_many(&mut self, batches: Vec<WriteBatch>) -> Result<Vec<u64>, StoreError> {
        let refs: Vec<&WriteBatch> = batches.iter().filter(|b| !b.is_empty()).collect();
        if refs.is_empty() {
            return Ok(vec![]);
        }

        let total_ops: u64 = refs.iter().map(|b| b.len() as u64).sum();

        // 1. WAL group commit — one fsync for all batches (INV-S01).
        let seqs = self.wal.append_batch(&refs)?;
        self.metrics.wal_appends.fetch_add(1, Ordering::Relaxed);

        // 2. Apply all batches to MemTable in order (INV-S07).
        for batch in &batches {
            if !batch.is_empty() {
                self.memtable.apply(batch);
            }
        }
        self.metrics
            .memtable_inserts
            .fetch_add(total_ops, Ordering::Relaxed);
        self.metrics
            .memtable_bytes
            .store(self.memtable.approx_bytes() as u64, Ordering::Relaxed);

        // 3. Single version increment (INV-S03: monotonic).
        self.version += 1;

        // 4. Publish one snapshot (INV-S01: after WAL fsync).
        let snapshot = Snapshot::new(
            self.version,
            self.memtable.as_arc_snapshot(),
            Arc::clone(&self.segments),
        );
        self.snapshot_mgr.publish(snapshot);
        self.metrics
            .snapshots_published
            .fetch_add(1, Ordering::Relaxed);

        debug!(
            version = self.version,
            batches = refs.len(),
            total_ops,
            "write_many committed (group commit)"
        );

        if self.memtable.approx_bytes() as u64 > self.config.memtable_flush_size {
            self.flush_memtable()?;
        }

        Ok(seqs)
    }

    /// Poll the compaction result channel and, if a result is ready, atomically
    /// replace the compacted segments with the merged output.
    ///
    /// Called from `write()` before the new write is processed so that the
    /// updated segment list is visible in the next snapshot.
    fn poll_compaction_result(&mut self) {
        if !self.pending_compaction {
            return;
        }
        match self.compaction_worker.result_rx.try_recv() {
            Ok(result) if result.merged_count == 0 => {
                // Compaction failed or produced nothing — clear the pending flag.
                self.pending_compaction = false;
            }
            Ok(result) => {
                // Replace the first `merged_count` segments (oldest) with the
                // merged output, keeping any newer segments that arrived while
                // compaction was running.
                let mut new_segs = result.new_segments;
                let tail = self.segments.get(result.merged_count..).unwrap_or(&[]);
                new_segs.extend_from_slice(tail);
                self.segments = Arc::new(new_segs);

                // Publish a snapshot with the new (compacted) segment list.
                let snapshot = Snapshot::new(
                    self.version,
                    self.memtable.as_arc_snapshot(),
                    Arc::clone(&self.segments),
                );
                self.snapshot_mgr.publish(snapshot);
                self.pending_compaction = false;

                debug!(version = self.version, "compaction result applied");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Worker still busy; try again next write.
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Worker exited unexpectedly.
                self.pending_compaction = false;
            }
        }
    }

    /// Flush the current MemTable contents to an immutable segment file.
    ///
    /// After flushing:
    /// - Entity/relationship payload data moves to the new segment.
    /// - The adjacency index is retained in the MemTable for traversal.
    /// - A new snapshot is published with the updated segment list.
    /// - Compaction is triggered if the L0 segment count reaches `l0_trigger`.
    ///
    /// Called only from `write()`, which holds `&mut self` (INV-S07).
    fn flush_memtable(&mut self) -> Result<(), StoreError> {
        let (entities, relationships) = self.memtable.drain_to_flush();

        if entities.is_empty() && relationships.is_empty() {
            return Ok(());
        }

        let segment_dir = self.config.data_dir.join("segments");
        let seg_path = segment_dir.join(format!("seg-{:020}.pxs", self.version));

        let seg = crate::segment::SegmentRef::write(&seg_path, 0, entities, relationships)?;

        // Append new segment (newest last).
        let mut new_segments = (*self.segments).clone();
        new_segments.push(seg);
        self.segments = Arc::new(new_segments);

        // Publish updated snapshot with flushed segment list.
        let snapshot = Snapshot::new(
            self.version,
            self.memtable.as_arc_snapshot(),
            Arc::clone(&self.segments),
        );
        self.snapshot_mgr.publish(snapshot);

        debug!(version = self.version, seg = %seg_path.display(), "MemTable flushed to segment");

        // Trigger compaction if enough L0 segments have accumulated.
        let l0_trigger = self.config.compaction.l0_trigger;
        if l0_trigger > 0 && !self.pending_compaction {
            let l0_count = self.segments.iter().filter(|s| s.level == 0).count();
            if l0_count >= l0_trigger {
                let segments_to_compact: Vec<_> =
                    self.segments.iter().filter(|s| s.level == 0).cloned().collect();
                let submitted = self.compaction_worker.try_compact(
                    segments_to_compact,
                    segment_dir,
                    self.version,
                );
                if submitted {
                    self.pending_compaction = true;
                    debug!(l0_count, "compaction triggered");
                }
            }
        }

        Ok(())
    }

    /// Acquire the current snapshot. Lock-free, wait-free.
    ///
    /// The returned guard keeps the snapshot alive until dropped.
    pub fn snapshot(&self) -> arc_swap::Guard<Arc<Snapshot>> {
        self.snapshot_mgr.snapshot()
    }

    /// A clonable handle to the snapshot manager, for sharing with reader tasks.
    pub fn snapshot_manager(&self) -> Arc<SnapshotManager> {
        Arc::clone(&self.snapshot_mgr)
    }

    /// Current write version (number of committed batches).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Access the metrics counters.
    pub fn metrics(&self) -> &Arc<StoreMetrics> {
        &self.metrics
    }

    /// Engine configuration.
    pub fn config(&self) -> &StoreConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parallax_core::{
        entity::{Entity, EntityClass, EntityId, EntityType},
        relationship::{Relationship, RelationshipClass, RelationshipId},
        source::SourceTag,
        timestamp::Timestamp,
    };
    use compact_str::CompactString;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn tmp_config() -> (StoreConfig, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = StoreConfig::new(dir.path());
        (config, dir)
    }

    fn make_entity(id: EntityId, typ: &str, class: &str) -> Entity {
        Entity {
            id,
            _type: EntityType::new_unchecked(typ),
            _class: EntityClass::new_unchecked(class),
            display_name: CompactString::default(),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    fn make_rel(id: RelationshipId, from: EntityId, to: EntityId, class: &str) -> Relationship {
        Relationship {
            id,
            from_id: from,
            to_id: to,
            _class: RelationshipClass::new_unchecked(class),
            properties: BTreeMap::new(),
            source: SourceTag::default(),
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            _deleted: false,
        }
    }

    #[test]
    fn open_fresh_engine() {
        let (config, _dir) = tmp_config();
        let engine = StorageEngine::open(config).unwrap();
        assert_eq!(engine.version(), 0);
        assert_eq!(engine.snapshot().entity_count(), 0);
    }

    #[test]
    fn write_and_read_entity() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");
        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        assert_eq!(snap.version, 1);
        assert!(snap.get_entity(id).is_some());
    }

    #[test]
    fn delete_entity_not_visible_in_snapshot() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");

        let mut b1 = WriteBatch::new();
        b1.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(b1).unwrap();

        let mut b2 = WriteBatch::new();
        b2.delete_entity(id);
        engine.write(b2).unwrap();

        let snap = engine.snapshot();
        assert!(snap.get_entity(id).is_none());
        assert_eq!(snap.entity_count(), 0);
    }

    #[test]
    fn old_snapshot_survives_new_write() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");
        let mut b1 = WriteBatch::new();
        b1.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(b1).unwrap();

        let old_snap = engine.snapshot(); // holds version 1

        let mut b2 = WriteBatch::new();
        b2.delete_entity(id);
        engine.write(b2).unwrap();

        // Old snapshot still sees the entity.
        assert!(old_snap.get_entity(id).is_some());
        // New snapshot does not.
        assert!(engine.snapshot().get_entity(id).is_none());
    }

    #[test]
    fn adjacency_visible_in_snapshot() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let a = EntityId::derive("acc", "host", "h1");
        let b = EntityId::derive("acc", "host", "h2");
        let rel_id = RelationshipId::derive("acc", "host", "h1", "CONNECTS", "host", "h2");

        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(a, "host", "Host"));
        batch.upsert_entity(make_entity(b, "host", "Host"));
        batch.upsert_relationship(make_rel(rel_id, a, b, "CONNECTS"));
        engine.write(batch).unwrap();

        let snap = engine.snapshot();
        assert_eq!(snap.adjacency(a).len(), 1);
        assert_eq!(snap.adjacency(b).len(), 1);
    }

    #[test]
    fn recovery_restores_state() {
        let (config, _dir) = tmp_config();
        let id = EntityId::derive("acc", "host", "h1");

        // Write, then drop engine (simulating a clean shutdown).
        {
            let mut engine = StorageEngine::open(config.clone()).unwrap();
            let mut batch = WriteBatch::new();
            batch.upsert_entity(make_entity(id, "host", "Host"));
            engine.write(batch).unwrap();
        }

        // Re-open: should recover entity from WAL.
        let engine2 = StorageEngine::open(config).unwrap();
        assert_eq!(engine2.version(), 1);
        assert!(engine2.snapshot().get_entity(id).is_some());
    }

    #[test]
    fn flush_moves_data_to_segment() {
        // Set a tiny flush threshold so a single entity write triggers a flush.
        let dir = tempfile::tempdir().expect("tempdir");
        let config = StoreConfig {
            data_dir: dir.path().to_path_buf(),
            memtable_flush_size: 0, // flush after every write
            ..Default::default()
        };
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");
        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(batch).unwrap();

        // After flush, entity should still be visible via segment read.
        let snap = engine.snapshot();
        assert!(snap.get_entity(id).is_some(), "entity readable after flush");
        // MemTable entity payload should be empty (flushed to segment).
        assert_eq!(engine.memtable.entity_count(), 0, "memtable cleared after flush");
    }

    /// Regression: after flush + re-upsert, entity_count() and entities_of_type()
    /// must NOT double-count the entity that exists in both MemTable and segment.
    #[test]
    fn no_double_count_after_flush_and_upsert() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = StoreConfig {
            data_dir: dir.path().to_path_buf(),
            memtable_flush_size: 0, // flush after every write
            ..Default::default()
        };
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");

        // Write h1 → triggers flush (h1 now in segment, MemTable cleared)
        let mut b1 = WriteBatch::new();
        b1.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(b1).unwrap();

        // Update h1 (now in MemTable AND segment)
        let mut b2 = WriteBatch::new();
        b2.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(b2).unwrap();

        let snap = engine.snapshot();
        // Must be 1 unique entity, not 2.
        assert_eq!(snap.entity_count(), 1, "double-count after flush+upsert");
        assert_eq!(snap.entities_of_type(&parallax_core::entity::EntityType::new_unchecked("host")).len(), 1,
            "entities_of_type double-count after flush+upsert");
    }

    #[test]
    fn write_many_group_commit_all_visible() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let ids: Vec<EntityId> = (0u8..5)
            .map(|i| EntityId::derive("acc", "host", &format!("h{i}")))
            .collect();

        // Submit 5 batches in one group commit.
        let batches: Vec<WriteBatch> = ids
            .iter()
            .map(|id| {
                let mut b = WriteBatch::new();
                b.upsert_entity(make_entity(*id, "host", "Host"));
                b
            })
            .collect();

        let seqs = engine.write_many(batches).unwrap();
        assert_eq!(seqs.len(), 5, "one seq per non-empty batch");

        // All 5 entities must be visible in the single published snapshot.
        let snap = engine.snapshot();
        assert_eq!(snap.entity_count(), 5);
        for id in &ids {
            assert!(snap.get_entity(*id).is_some(), "entity {id:?} missing");
        }

        // One fsync → one wal_appends counter increment.
        let m = engine.metrics().snapshot();
        assert_eq!(m.wal_appends, 1);
    }

    #[test]
    fn write_many_recovery_restores_all() {
        let (config, _dir) = tmp_config();

        let ids: Vec<EntityId> = (0u8..3)
            .map(|i| EntityId::derive("acc", "host", &format!("r{i}")))
            .collect();

        {
            let mut engine = StorageEngine::open(config.clone()).unwrap();
            let batches: Vec<WriteBatch> = ids
                .iter()
                .map(|id| {
                    let mut b = WriteBatch::new();
                    b.upsert_entity(make_entity(*id, "host", "Host"));
                    b
                })
                .collect();
            engine.write_many(batches).unwrap();
        }

        // Re-open: WAL recovery must replay all group-committed entries.
        let engine2 = StorageEngine::open(config).unwrap();
        let snap = engine2.snapshot();
        assert_eq!(snap.entity_count(), 3);
        for id in &ids {
            assert!(snap.get_entity(*id).is_some(), "entity {id:?} missing after recovery");
        }
    }

    #[test]
    fn metrics_incremented_on_write() {
        let (config, _dir) = tmp_config();
        let mut engine = StorageEngine::open(config).unwrap();

        let id = EntityId::derive("acc", "host", "h1");
        let mut batch = WriteBatch::new();
        batch.upsert_entity(make_entity(id, "host", "Host"));
        engine.write(batch).unwrap();

        let m = engine.metrics().snapshot();
        assert_eq!(m.wal_appends, 1);
        assert_eq!(m.snapshots_published, 1);
        assert_eq!(m.memtable_inserts, 1);
    }
}
