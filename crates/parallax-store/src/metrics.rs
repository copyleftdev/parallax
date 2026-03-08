//! Storage engine metrics — atomic counters for observability.
//!
//! All counters use `Relaxed` ordering: they are advisory and do not
//! participate in any synchronization protocol.
//!
//! > Bos: "Relaxed: No synchronization, only atomicity.
//! > Use for: Counters where exact order doesn't matter."
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.11

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters exported by the storage engine.
///
/// All fields are public so callers can read them directly.
/// Increment with `fetch_add(1, Ordering::Relaxed)`.
#[derive(Debug, Default)]
pub struct StoreMetrics {
    /// Number of `WriteBatch`es appended to the WAL.
    pub wal_appends: AtomicU64,
    /// Total bytes written to the WAL (payload bytes only).
    pub wal_bytes_written: AtomicU64,
    /// Number of fsync calls issued to the WAL.
    pub wal_fsyncs: AtomicU64,
    /// Number of individual write operations applied to the MemTable.
    pub memtable_inserts: AtomicU64,
    /// Approximate byte size of the MemTable (updated on each write).
    pub memtable_bytes: AtomicU64,
    /// Number of snapshots published.
    pub snapshots_published: AtomicU64,
    /// Number of compaction jobs completed.
    pub compactions_completed: AtomicU64,
    /// Bytes reclaimed by compaction.
    pub compaction_bytes_reclaimed: AtomicU64,
    /// Entity lookups satisfied from MemTable (hot path).
    pub read_entity_hits: AtomicU64,
    /// Entity lookups that fell through to segments (cold path).
    pub read_entity_misses: AtomicU64,
}

impl StoreMetrics {
    /// Create a zeroed metrics instance.
    pub fn new() -> Self {
        StoreMetrics::default()
    }

    /// Snapshot all counters as plain `u64` values (for reporting/export).
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            wal_appends: self.wal_appends.load(Ordering::Relaxed),
            wal_bytes_written: self.wal_bytes_written.load(Ordering::Relaxed),
            wal_fsyncs: self.wal_fsyncs.load(Ordering::Relaxed),
            memtable_inserts: self.memtable_inserts.load(Ordering::Relaxed),
            memtable_bytes: self.memtable_bytes.load(Ordering::Relaxed),
            snapshots_published: self.snapshots_published.load(Ordering::Relaxed),
            compactions_completed: self.compactions_completed.load(Ordering::Relaxed),
            compaction_bytes_reclaimed: self.compaction_bytes_reclaimed.load(Ordering::Relaxed),
            read_entity_hits: self.read_entity_hits.load(Ordering::Relaxed),
            read_entity_misses: self.read_entity_misses.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time copy of all metric counters.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricsSnapshot {
    pub wal_appends: u64,
    pub wal_bytes_written: u64,
    pub wal_fsyncs: u64,
    pub memtable_inserts: u64,
    pub memtable_bytes: u64,
    pub snapshots_published: u64,
    pub compactions_completed: u64,
    pub compaction_bytes_reclaimed: u64,
    pub read_entity_hits: u64,
    pub read_entity_misses: u64,
}
