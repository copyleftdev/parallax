//! # parallax-store
//!
//! Storage engine: WAL, MemTable, MVCC snapshots, on-disk segments, compaction.
//!
//! ## Architecture
//!
//! Single-writer, multi-reader concurrency model:
//!
//! ```text
//! WriteBatch ──► WriteAheadLog (fsync) ──► MemTable ──► Snapshot (Arc)
//!                                                             │
//!                                          SnapshotManager ◄─┘
//!                                          (ArcSwap, lock-free)
//!                                               │
//!                                    Reader 1 ◄─┤─► Reader 2 ◄─► Reader N
//! ```
//!
//! **Spec reference:** `specs/02-storage-engine.md`

pub mod compaction;
pub mod engine;
pub mod error;
pub mod index;
pub mod memtable;
pub mod metrics;
pub mod segment;
pub mod snapshot;
pub mod wal;
pub mod write_batch;

// Convenience re-exports for crates that depend on parallax-store.
pub use engine::{GroupCommitConfig, StoreConfig, StorageEngine};
pub use error::StoreError;
pub use index::{AdjEntry, AdjList};
pub use memtable::MemTable;
pub use metrics::{MetricsSnapshot, StoreMetrics};
pub use snapshot::{Snapshot, SnapshotManager};
pub use write_batch::{WriteBatch, WriteOp};
pub use wal::{dump_wal, WalDumpEntry};
