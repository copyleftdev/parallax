# Performance Targets

## v0.1 Targets

These are the benchmarked targets for v0.1 on modern NVMe hardware:

| Operation | Target p99 | Notes |
|---|---|---|
| Entity lookup by ID (MemTable) | ≤1μs | `BTreeMap::get` |
| Entity lookup by ID (Segment) | ≤100μs | Linear scan; improves with index in v0.2 |
| Type index scan (1K entities) | ≤500μs | Iterate type index, fetch entities |
| Single-hop traversal (degree ≤100) | ≤500μs | Adjacency index lookup |
| Multi-hop traversal (depth 3, degree 5) | ≤5ms | BFS with visited set |
| Shortest path (10K-node graph) | ≤10ms | Bidirectional BFS |
| Blast radius (depth 4, 1K impacted) | ≤10ms | BFS from origin |
| WAL write throughput | ≥500K ops/sec | Batched, single fsync |
| PQL parse + plan | ≤1ms | Hand-written recursive descent |
| Snapshot acquisition | ≤1μs | `Arc::clone` |
| Query execution (count, 100K entities) | ≤50ms | Type index + filter |

## Benchmarking

Run the benchmark suite:

```bash
# Storage engine benchmarks
cargo bench --package parallax-store

# Graph engine benchmarks
cargo bench --package parallax-graph

# Query engine benchmarks
cargo bench --package parallax-query
```

## What Affects Performance

### MemTable vs. Segment Reads

Entities in the MemTable (recently written) are served in ≤1μs. Entities
that have been flushed to segment files require a linear scan of the segment,
adding ~100μs per entity. The segment index (v0.2) will reduce this to ~1μs.

**Implication:** If your workload has mostly recent data (e.g., fresh ingest
followed by queries), performance is excellent. If your graph is heavily
segmented, upgrade to v0.2 for segment indexing.

### Traversal Depth and Degree

Traversal complexity is O(b^d) where b is the average branching factor and
d is the depth. With BFS visited-set deduplication:
- Depth 2, degree 10: 100 entities visited
- Depth 4, degree 10: 10,000 entities visited
- Depth 4, degree 100: 100,000,000 entities — exceeds practical limits

Use `max_depth` to bound traversals. Set `edge_classes` to filter edge types
and reduce the branching factor.

### Lock Contention

The `Arc<Mutex<StorageEngine>>` lock is held for:
- ~5μs per entity during diff computation
- ~10μs for WAL fsync + MemTable update

Multiple concurrent syncs queue at the write step but diff in parallel.
For more than 10 concurrent connectors syncing simultaneously, expect
write latency to increase. WAL group commit (v0.2) will amortize this.

### Memory Consumption

Rule of thumb: ~1KB per entity in MemTable (struct + property strings on heap).
For 100K entities, expect ~100MB MemTable size before flush.

The flush threshold (default: 64MB) triggers a segment flush, freeing MemTable
memory. Tune `memtable_flush_size` based on your available memory.

## Scaling to 1M+ Entities

v0.1 is benchmarked and correct at 100K entities. For 1M+ entities:

1. **Segment indexing** (v0.2): reduces lookup from O(n_segment) to O(log n)
2. **Compaction** (v0.2): reclaims space from deleted entities
3. **WAL group commit** (v0.2): 10-100× write throughput improvement

The architecture supports 10M+ entities with the v0.2 improvements. The
single-writer model remains correct at any scale — it simply processes
writes faster with group commit.
