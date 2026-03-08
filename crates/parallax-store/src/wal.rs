//! Write-ahead log (WAL).
//!
//! The WAL is the durability backbone. Every `WriteBatch` is serialized and
//! appended to the WAL before being applied to the MemTable. After `append()`
//! returns `Ok`, the batch survives crashes.
//!
//! **Spec reference:** `specs/02-storage-engine.md` §2.4
//!
//! # On-disk entry format (little-endian)
//!
//! ```text
//! ┌──────────┬──────────┬──────────┬────────────┬──────────┐
//! │ magic(4) │ len(4)   │ seq(8)   │ payload(N) │ crc32(4) │
//! └──────────┴──────────┴──────────┴────────────┴──────────┘
//!
//!   magic:   0x50585741 ("PXWA" — Parallax WAL)
//!   len:     Total entry length (magic + len + seq + payload + crc = N + 20)
//!   seq:     Monotonic sequence number assigned by the WAL
//!   payload: postcard-serialized WriteBatch
//!   crc32:   CRC32C of (seq_le_bytes || payload)
//! ```
//!
//! # Segment files
//!
//! WAL segments are named `wal-{index:08}.pxw` under the WAL directory.
//! A new segment is created when the active segment exceeds `max_segment_size`.
//!
//! INV-S01: A write batch is durable (fsync'd to WAL) before it is visible
//!          in any snapshot.
//! INV-S05: WAL recovery replays all entries after the last checkpoint, in order.
//! INV-S06: Corrupt WAL entries are detected by CRC and recovery stops at the
//!          last valid entry. No silent data corruption.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::error::StoreError;
use crate::write_batch::WriteBatch;

/// WAL entry magic bytes: "PXWA"
const WAL_MAGIC: [u8; 4] = [0x50, 0x58, 0x57, 0x41];

/// Minimum valid entry size (magic + len + seq + empty payload + crc).
const MIN_ENTRY_SIZE: usize = 20;

/// Write-ahead log for durable batch commits.
///
/// Only the writer thread calls `append()`. No synchronization is needed
/// on the write path (INV-S07).
pub struct WriteAheadLog {
    dir: PathBuf,
    active: File,
    active_size: u64,
    next_seq: u64,
    segment_index: u32,
    max_segment_size: u64,
}

impl WriteAheadLog {
    /// Open an existing WAL directory (recovering batches from existing segments)
    /// or create a new one. Returns the WAL and any batches to replay.
    ///
    /// Recovery replays all batches with `seq > checkpoint_seq`. Pass `0` to
    /// replay everything (the v0.1 default, since there is no checkpoint yet).
    pub fn open(
        dir: &Path,
        max_segment_size: u64,
        checkpoint_seq: u64,
    ) -> Result<(Self, Vec<WriteBatch>), StoreError> {
        fs::create_dir_all(dir).map_err(StoreError::DirCreate)?;

        let segments = sorted_segment_paths(dir)?;

        let (batches, last_seq) = if segments.is_empty() {
            (Vec::new(), 0u64)
        } else {
            recover_batches(&segments, checkpoint_seq)?
        };

        let segment_index = segments.len() as u32;
        let next_seq = last_seq + 1;

        // Open the last segment for appending, or create a new one.
        let (active, _active_path) = if let Some(last) = segments.last() {
            let size = last.metadata().map_err(StoreError::WalIo)?.len();
            if size < max_segment_size {
                // Append to existing segment.
                let file = OpenOptions::new()
                    .append(true)
                    .open(last)
                    .map_err(StoreError::WalIo)?;
                info!(path = %last.display(), "WAL: appending to existing segment");
                (file, last.to_path_buf())
            } else {
                // Existing segment is full; start a new one.
                open_new_segment(dir, segment_index)?
            }
        } else {
            open_new_segment(dir, 0)?
        };

        let active_size = active.metadata().map_err(StoreError::WalIo)?.len();

        info!(
            dir = %dir.display(),
            replayed = batches.len(),
            next_seq,
            "WAL opened"
        );

        Ok((
            WriteAheadLog {
                dir: dir.to_path_buf(),
                active,
                active_size,
                next_seq,
                segment_index,
                max_segment_size,
            },
            batches,
        ))
    }

    /// Append a `WriteBatch` to the WAL. This is the durability commit point.
    ///
    /// After this returns `Ok(seq)`, the batch is guaranteed to survive crashes.
    /// The batch is NOT yet visible to readers — call `StorageEngine::write()`
    /// which applies to the MemTable and publishes a snapshot after this.
    ///
    /// INV-S01: enforced — WAL fsync before snapshot publish.
    pub fn append(&mut self, batch: &WriteBatch) -> Result<u64, StoreError> {
        let seq = self.next_seq;

        let payload: Vec<u8> = postcard::to_allocvec(batch)?;

        // CRC32C over (seq_le_bytes || payload).
        let crc = crc32c::crc32c_append(crc32c::crc32c(&seq.to_le_bytes()), &payload);

        // Total entry length: magic(4) + len(4) + seq(8) + payload(N) + crc(4)
        let entry_len = (MIN_ENTRY_SIZE + payload.len()) as u32;

        // Rotate segment if the active file would exceed max size.
        if self.active_size + u64::from(entry_len) > self.max_segment_size {
            self.rotate()?;
        }

        // Write the entry in field order.
        self.active
            .write_all(&WAL_MAGIC)
            .map_err(StoreError::WalWrite)?;
        self.active
            .write_all(&entry_len.to_le_bytes())
            .map_err(StoreError::WalWrite)?;
        self.active
            .write_all(&seq.to_le_bytes())
            .map_err(StoreError::WalWrite)?;
        self.active
            .write_all(&payload)
            .map_err(StoreError::WalWrite)?;
        self.active
            .write_all(&crc.to_le_bytes())
            .map_err(StoreError::WalWrite)?;

        // fsync: batch is now durable (INV-S01).
        self.active.sync_data().map_err(StoreError::WalWrite)?;
        self.active_size += u64::from(entry_len);
        self.next_seq += 1;

        Ok(seq)
    }

    /// Append multiple `WriteBatch`es to the WAL with a **single** fsync.
    ///
    /// This is the group-commit path: all batches are serialized and written
    /// contiguously; only one `sync_data()` call is issued at the end.
    /// Throughput improves 10–100× under high write concurrency by amortizing
    /// the ~100 μs NVMe fsync cost across all batches in the group.
    ///
    /// Each batch still gets its own WAL entry (same on-disk format as
    /// `append()`), so recovery remains compatible with the existing reader.
    ///
    /// Returns the sequence numbers assigned to each batch in order.
    ///
    /// INV-S01: enforced — single fsync covers all entries before returning.
    pub fn append_batch(&mut self, batches: &[&WriteBatch]) -> Result<Vec<u64>, StoreError> {
        if batches.is_empty() {
            return Ok(Vec::new());
        }

        let mut seqs = Vec::with_capacity(batches.len());
        // Accumulate serialized bytes between segment rotations.
        let mut pending: Vec<u8> = Vec::new();
        let mut pending_size: u64 = 0;

        for batch in batches {
            let seq = self.next_seq;
            let payload: Vec<u8> = postcard::to_allocvec(batch)?;
            let crc = crc32c::crc32c_append(crc32c::crc32c(&seq.to_le_bytes()), &payload);
            let entry_len = (MIN_ENTRY_SIZE + payload.len()) as u32;

            // If this entry would overflow the active segment, flush pending
            // bytes first, then rotate to a fresh segment.
            let would_overflow =
                self.active_size + pending_size + u64::from(entry_len) > self.max_segment_size;
            if would_overflow {
                if !pending.is_empty() {
                    self.active
                        .write_all(&pending)
                        .map_err(StoreError::WalWrite)?;
                    self.active_size += pending_size;
                    pending.clear();
                    pending_size = 0;
                }
                self.rotate()?;
            }

            pending.extend_from_slice(&WAL_MAGIC);
            pending.extend_from_slice(&entry_len.to_le_bytes());
            pending.extend_from_slice(&seq.to_le_bytes());
            pending.extend_from_slice(&payload);
            pending.extend_from_slice(&crc.to_le_bytes());
            pending_size += u64::from(entry_len);

            seqs.push(seq);
            self.next_seq += 1;
        }

        // Write all remaining entries in one shot, then fsync once (INV-S01).
        if !pending.is_empty() {
            self.active
                .write_all(&pending)
                .map_err(StoreError::WalWrite)?;
            self.active.sync_data().map_err(StoreError::WalWrite)?;
            self.active_size += pending_size;
        }

        Ok(seqs)
    }

    /// Rotate to a new segment file.
    fn rotate(&mut self) -> Result<(), StoreError> {
        self.active.sync_all().map_err(StoreError::WalWrite)?;
        self.segment_index += 1;
        let (new_file, new_path) = open_new_segment(&self.dir, self.segment_index)?;
        info!(path = %new_path.display(), "WAL: rotated to new segment");
        self.active = new_file;
        self.active_size = 0;
        Ok(())
    }
}

// ─── WAL dump (4B) ───────────────────────────────────────────────────────────

/// A single decoded WAL entry for inspection / debugging.
pub struct WalDumpEntry {
    pub seq: u64,
    pub batch: WriteBatch,
    pub segment: String,
}

/// Read all WAL entries from `data_dir/wal/` without applying them.
///
/// Returns entries in write order (oldest segment first, ascending seq).
/// Corrupt entries terminate reading for the affected segment (stops there,
/// continues to next segment — same as crash recovery).
///
/// Intended for `parallax wal dump` debugging (INV-S06: readable post-crash).
pub fn dump_wal(data_dir: &Path) -> Result<Vec<WalDumpEntry>, StoreError> {
    let wal_dir = data_dir.join("wal");
    let segments = sorted_segment_paths(&wal_dir)?;
    let mut out = Vec::new();

    for seg_path in &segments {
        let seg_name = seg_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let file = File::open(seg_path).map_err(StoreError::WalIo)?;
        let mut reader = BufReader::new(file);
        loop {
            match read_entry(&mut reader) {
                Ok(None) => break,
                Ok(Some(entry)) => {
                    match postcard::from_bytes::<WriteBatch>(&entry.payload) {
                        Ok(batch) => out.push(WalDumpEntry {
                            seq: entry.seq,
                            batch,
                            segment: seg_name.clone(),
                        }),
                        Err(_) => break, // deserialize error → skip rest of segment
                    }
                }
                Err(_) => break, // corrupt magic/CRC → skip rest of segment
            }
        }
    }

    Ok(out)
}

// --- File helpers ---

fn segment_path(dir: &Path, index: u32) -> PathBuf {
    dir.join(format!("wal-{index:08}.pxw"))
}

fn open_new_segment(dir: &Path, index: u32) -> Result<(File, PathBuf), StoreError> {
    let path = segment_path(dir, index);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(StoreError::WalIo)?;
    Ok((file, path))
}

/// Return all WAL segment paths in ascending order (oldest first).
fn sorted_segment_paths(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(StoreError::WalIo)?
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("pxw")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("wal-"))
                    .unwrap_or(false)
        })
        .collect();

    // Lexicographic sort is correct because of zero-padded naming.
    paths.sort();
    Ok(paths)
}

// --- Recovery ---

/// Scan WAL segments in order and collect all batches with `seq > checkpoint_seq`.
///
/// Stops at the first corrupt entry (CRC mismatch) and logs a warning.
/// Returns `(batches, last_valid_seq)`.
///
/// INV-S05, INV-S06.
fn recover_batches(
    segments: &[PathBuf],
    checkpoint_seq: u64,
) -> Result<(Vec<WriteBatch>, u64), StoreError> {
    let mut batches = Vec::new();
    let mut last_seq = checkpoint_seq;
    let mut corrupt = false;

    'outer: for seg_path in segments {
        let file = File::open(seg_path).map_err(StoreError::WalIo)?;
        let mut reader = BufReader::new(file);

        loop {
            match read_entry(&mut reader) {
                Ok(None) => break, // EOF — move to next segment.
                Ok(Some(entry)) => {
                    last_seq = last_seq.max(entry.seq);
                    if entry.seq > checkpoint_seq {
                        match postcard::from_bytes::<WriteBatch>(&entry.payload) {
                            Ok(batch) => batches.push(batch),
                            Err(e) => {
                                warn!(
                                    seq = entry.seq,
                                    path = %seg_path.display(),
                                    error = %e,
                                    "WAL: failed to deserialize batch; stopping recovery"
                                );
                                corrupt = true;
                                break 'outer;
                            }
                        }
                    }
                }
                Err(StoreError::WalCorrupt { seq }) => {
                    warn!(
                        seq,
                        path = %seg_path.display(),
                        "WAL: corrupt entry detected; stopping recovery (INV-S06)"
                    );
                    corrupt = true;
                    break 'outer;
                }
                Err(e) => return Err(e),
            }
        }
    }

    if corrupt {
        warn!(
            recovered = batches.len(),
            "WAL: recovery stopped at corrupt entry; some data may be lost"
        );
    } else {
        info!(
            recovered = batches.len(),
            last_seq, "WAL: recovery complete"
        );
    }

    Ok((batches, last_seq))
}

/// A parsed WAL entry (header already verified, CRC already checked).
struct WalEntry {
    seq: u64,
    payload: Vec<u8>,
}

/// Read the next entry from a WAL segment reader.
///
/// Returns `Ok(None)` at clean EOF. Returns `Err(StoreError::WalCorrupt)`
/// on magic or CRC mismatch.
fn read_entry(reader: &mut impl Read) -> Result<Option<WalEntry>, StoreError> {
    // Read magic (4 bytes).
    let mut magic = [0u8; 4];
    match reader.read_exact(&mut magic) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(StoreError::WalIo(e)),
    }
    if magic != WAL_MAGIC {
        return Err(StoreError::WalCorrupt { seq: 0 });
    }

    // Read len (4 bytes, u32 LE).
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).map_err(StoreError::WalIo)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len < MIN_ENTRY_SIZE {
        return Err(StoreError::WalCorrupt { seq: 0 });
    }

    // Read seq (8 bytes, u64 LE).
    let mut seq_buf = [0u8; 8];
    reader.read_exact(&mut seq_buf).map_err(StoreError::WalIo)?;
    let seq = u64::from_le_bytes(seq_buf);

    // Read payload (len - 20 bytes).
    let payload_len = len - MIN_ENTRY_SIZE;
    let mut payload = vec![0u8; payload_len];
    reader.read_exact(&mut payload).map_err(StoreError::WalIo)?;

    // Read stored CRC (4 bytes, u32 LE).
    let mut crc_buf = [0u8; 4];
    reader.read_exact(&mut crc_buf).map_err(StoreError::WalIo)?;
    let stored_crc = u32::from_le_bytes(crc_buf);

    // Verify CRC32C over (seq_bytes || payload).
    let computed_crc = crc32c::crc32c_append(crc32c::crc32c(&seq_buf), &payload);
    if computed_crc != stored_crc {
        return Err(StoreError::WalCorrupt { seq });
    }

    Ok(Some(WalEntry { seq, payload }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use parallax_core::entity::EntityId;
    use tempfile::TempDir;

    fn tmp_dir() -> TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    #[test]
    fn round_trip_single_batch() {
        let dir = tmp_dir();
        let (mut wal, recovered) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();
        assert!(recovered.is_empty());

        let mut batch = WriteBatch::new();
        batch.delete_entity(EntityId::derive("a", "host", "h1"));
        let seq = wal.append(&batch).unwrap();
        assert_eq!(seq, 1);

        drop(wal);

        // Re-open and recover.
        let (_wal2, recovered2) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();
        assert_eq!(recovered2.len(), 1);
        assert_eq!(recovered2[0].len(), 1);
    }

    #[test]
    fn multiple_batches_recover_in_order() {
        let dir = tmp_dir();
        let (mut wal, _) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();

        for i in 0u8..5 {
            let mut batch = WriteBatch::new();
            batch.delete_entity(EntityId([i; 16]));
            wal.append(&batch).unwrap();
        }
        drop(wal);

        let (_wal2, recovered) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();
        assert_eq!(recovered.len(), 5);
    }

    #[test]
    fn append_batch_single_fsync_recovers_all() {
        let dir = tmp_dir();
        let (mut wal, _) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();

        // Build 3 batches and commit them with a single fsync.
        let batches: Vec<WriteBatch> = (0u8..3)
            .map(|i| {
                let mut b = WriteBatch::new();
                b.delete_entity(EntityId([i; 16]));
                b
            })
            .collect();
        let refs: Vec<&WriteBatch> = batches.iter().collect();
        let seqs = wal.append_batch(&refs).unwrap();

        // Sequence numbers must be monotonically assigned.
        assert_eq!(seqs, vec![1, 2, 3]);

        drop(wal);

        // Recovery must replay all 3 entries.
        let (_wal2, recovered) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();
        assert_eq!(recovered.len(), 3);
    }

    #[test]
    fn append_batch_checkpoint_filters_old_batches() {
        let dir = tmp_dir();
        let (mut wal, _) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 0).unwrap();

        let batches: Vec<WriteBatch> = (0u8..4)
            .map(|i| {
                let mut b = WriteBatch::new();
                b.delete_entity(EntityId([i; 16]));
                b
            })
            .collect();
        let refs: Vec<&WriteBatch> = batches.iter().collect();
        wal.append_batch(&refs).unwrap();
        drop(wal);

        // Checkpoint at seq=2: only seq 3 and 4 should replay.
        let (_wal2, recovered) = WriteAheadLog::open(dir.path(), 64 * 1024 * 1024, 2).unwrap();
        assert_eq!(recovered.len(), 2);
    }
}
