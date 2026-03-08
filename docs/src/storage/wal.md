# Write-Ahead Log (WAL)

The WAL is Parallax's durability guarantee. Every mutation is written to the
WAL and `fsync()`'d before being applied to the MemTable. If the process
crashes, the WAL replays to reconstruct the MemTable.

## On-Disk Format

WAL data lives in `{data_dir}/wal/`. Each segment file is named
`wal-{index:08}.pxw` (e.g., `wal-00000000.pxw`).

Each entry in a WAL file has this layout (little-endian):

```
┌──────────┬──────────┬──────────┬────────────┬──────────┐
│ magic(4) │ len(4)   │ seq(8)   │ payload(N) │ crc32(4) │
└──────────┴──────────┴──────────┴────────────┴──────────┘
```

| Field | Size | Description |
|---|---|---|
| `magic` | 4 bytes | `0x50585741` — ASCII "PXWA" (Parallax WAL) |
| `len` | 4 bytes | Total entry length including all fields |
| `seq` | 8 bytes | Monotonic sequence number |
| `payload` | N bytes | Serialized `WriteBatch` (postcard format) |
| `crc32c` | 4 bytes | CRC32C of `(seq bytes || payload bytes)` |

## Write Protocol

```rust
pub fn append(&mut self, batch: &WriteBatch) -> Result<u64, WalError> {
    let seq = self.next_seq;
    self.next_seq += 1;

    // Serialize the batch with postcard (compact binary format)
    let payload = postcard::to_allocvec(batch)?;

    // Compute checksum over seq + payload
    let crc = crc32c_combine(seq.to_le_bytes(), &payload);

    // Rotate to a new segment if the active file exceeds MAX_SEGMENT_SIZE
    if self.active_size + entry_len > MAX_SEGMENT_SIZE {
        self.rotate()?;
    }

    // Write all fields
    self.active.write_all(&WAL_MAGIC)?;
    self.active.write_all(&(total_len as u32).to_le_bytes())?;
    self.active.write_all(&seq.to_le_bytes())?;
    self.active.write_all(&payload)?;
    self.active.write_all(&crc.to_le_bytes())?;

    // fsync — this is the durability commit point
    self.active.sync_data()?;

    Ok(seq)
}
```

**After `sync_data()` returns, the entry survives any crash.**

## Crash Recovery

On startup, `WriteAheadLog::recover()` replays all WAL segments:

```rust
pub fn recover(&mut self) -> Result<Vec<WriteBatch>, WalError> {
    let mut batches = Vec::new();
    for segment_file in self.sorted_segment_files()? {
        let entries = self.read_segment(&segment_file)?;
        for entry in entries {
            // Verify magic
            if entry.magic != WAL_MAGIC { return Err(WalError::Corruption(...)); }
            // Verify checksum (INV-S05)
            if !verify_crc32c(entry.seq, &entry.payload, entry.crc32c) {
                // Stop at first corrupt entry
                tracing::warn!("WAL corruption detected at seq {}", entry.seq);
                break;
            }
            let batch = postcard::from_bytes(&entry.payload)?;
            batches.push(batch);
        }
    }
    Ok(batches)
}
```

**INV-S05:** A corrupt WAL entry stops recovery at that point. All entries
before it are applied; the corrupt entry and everything after are discarded.
This may result in losing the most recent write if the process crashed during
`fsync()`. This is correct — the write was not acknowledged to the caller yet.

## Segment Rotation

When the active WAL segment exceeds `MAX_SEGMENT_SIZE` (default: 64MB):

1. `sync_data()` the active file
2. Close the active file handle
3. Open a new file: `wal-{next_index:08}.pxw`
4. Set the new file as active

Old WAL segments are retained until after a successful MemTable flush to
a `.pxs` segment. At that point, WAL segments covered by the segment are
safe to delete.

## Group Commit (v0.2)

For high-throughput ingestion, individual `fsync()` calls are expensive
(~100μs on NVMe). Group commit batches multiple `WriteBatch`es into a
single `fsync()`:

```
100 batches × 1 fsync = 100× write throughput
vs.
100 batches × 100 fsyncs = 100× latency overhead
```

Group commit is planned for v0.2. The current v0.1 implementation does one
`fsync()` per `WriteBatch`, which is correct and suitable for all but the
highest-throughput ingestion scenarios.

## Inspecting WAL Files

WAL files are binary and not human-readable directly. Use the CLI to inspect
the current state of the engine rather than reading WAL files:

```bash
parallax stats --data-dir /var/lib/parallax
```

A future `parallax wal dump` command is planned for v0.2 debugging.
