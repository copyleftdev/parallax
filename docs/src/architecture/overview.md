# Architecture Overview

Parallax is organized as a Cargo workspace of nine crates arranged in a
strictly acyclic dependency chain. Each crate has a single responsibility,
a minimal public API, and explicit dependencies.

## The Nine Crates

| Crate | Responsibility |
|---|---|
| `parallax-core` | Shared types and error definitions. Zero external deps beyond `serde` and `blake3`. |
| `parallax-store` | Durable storage: WAL, MemTable, immutable Segments, MVCC snapshots. |
| `parallax-graph` | Graph reasoning: traversal, pattern matching, shortest path, blast radius. |
| `parallax-query` | PQL parse → plan → execute pipeline. |
| `parallax-policy` | Policy rule evaluation and posture scoring. |
| `parallax-ingest` | Source-scoped sync protocol: diff, validate, atomic commit. |
| `parallax-connect` | Integration SDK: `Connector` trait, step scheduler, entity/relationship builders. |
| `parallax-server` | REST HTTP server with authentication, request-ID middleware, Prometheus metrics. |
| `parallax-cli` | Command-line binary: `serve`, `query`, `stats`, `version`. |

## Dependency Graph

```
parallax-core
    │
    ├──► parallax-store
    │        │
    │        ├──► parallax-graph
    │        │        │
    │        │        ├──► parallax-query
    │        │        └──► parallax-policy
    │        │
    │        └──► parallax-ingest
    │                 │
    │                 └──► parallax-connect
    │
parallax-server  (depends on: core, store, graph, query, policy, ingest, connect)
    │
    └──► parallax-cli
```

**The rule is absolute:** No crate may depend on a crate above it in this
graph. The Rust module system enforces this at compile time.

## Data Flow

A typical request through the full stack:

```
External Source
      │
      ▼
[REST POST /v1/ingest/sync]    ← parallax-server validates auth, parses JSON
      │
      ▼
[validate_sync_batch()]         ← parallax-ingest checks referential integrity
      │                                          and class/verb constraints
      ▼
[SyncEngine::commit_sync()]     ← parallax-ingest diffs against current snapshot,
      │                                           builds WriteBatch
      ▼
[StorageEngine::write(batch)]   ← parallax-store appends WAL entry, updates
      │                                           MemTable, publishes new snapshot
      ▼
[Snapshot published]            ← All subsequent reads see new state

      ...later...

[REST POST /v1/query]           ← parallax-server
      │
      ▼
[parse(pql)]                    ← parallax-query lexer + recursive descent parser
      │
      ▼
[plan(ast, &stats)]             ← parallax-query planner chooses index strategy
      │
      ▼
[execute(plan, &snapshot)]      ← parallax-query executor calls parallax-graph
      │
      ▼
[GraphReader::find() / traverse()]  ← parallax-graph reads from MVCC snapshot
      │
      ▼
[JSON response]
```

## Key Architectural Decisions

### Single-Writer, Multi-Reader

All mutations flow through one serialized write path. This eliminates
write-write conflicts, deadlocks, and lock ordering bugs. Readers operate
on MVCC snapshots with zero coordination — a reader never blocks a writer.

### Owned Storage Engine

No external storage dependency (no Neo4j, no RocksDB, no sled). Parallax
owns its storage format, which means:
- Embeddable in a CLI without a separate daemon
- Deterministic latency without JVM GC pauses
- Full control over the read/write path

### Deterministic Entity Identity

Entity IDs are 128-bit blake3 hashes of `(account_id, entity_type, entity_key)`.
The same logical entity always gets the same ID, across all time and all
connectors. This enables idempotent re-ingestion and conflict-free merging.

### Hybrid Logical Clocks

Even in single-node mode, Parallax uses HLC timestamps rather than wall
clocks. Wall clocks can go backwards (NTP adjustments); two events in the
same millisecond need deterministic ordering; and HLC extends naturally
when clustering is added.
