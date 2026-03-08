# Crate Dependency Map

## Workspace Layout

```
parallax/
├── Cargo.toml                     # Workspace root
├── crates/
│   ├── parallax-core/             # Shared types, zero external deps
│   ├── parallax-store/            # Storage engine (WAL + MemTable + Segments)
│   ├── parallax-graph/            # Graph traversal and reasoning
│   ├── parallax-query/            # PQL parse + plan + execute
│   ├── parallax-policy/           # Policy rule evaluation
│   ├── parallax-ingest/           # Sync protocol (diff + commit)
│   ├── parallax-connect/          # Integration SDK
│   ├── parallax-server/           # REST HTTP server
│   └── parallax-cli/              # CLI binary
└── specs/                         # Architectural specifications
```

## Dependency Graph

```
parallax-core          (no workspace deps; blake3, serde, compact_str, thiserror)
       │
       ├──► parallax-store     (core; arc-swap, crc32c, lz4_flex, postcard, tracing)
       │           │
       │           ├──► parallax-graph     (core, store)
       │           │           │
       │           │           ├──► parallax-query      (core, graph)
       │           │           └──► parallax-policy     (core, graph)
       │           │
       │           └──► parallax-ingest    (core, store)
       │                       │
       │                       └──► parallax-connect    (core, ingest)
       │
parallax-server  (core, store, graph, query, policy, ingest, connect;
       │          axum, tower, tower-http, tokio, serde_json, uuid, compact_str)
       │
parallax-cli     (server; clap, anyhow, tokio)
```

## External Dependencies by Crate

### parallax-core

| Crate | Version | Purpose |
|---|---|---|
| `serde` | 1 | Serialization derive macros |
| `compact_str` | 0.8 | Small-string optimization (entity types/classes are short) |
| `blake3` | 1 | Deterministic ID hashing (SIMD-accelerated) |
| `thiserror` | 1 | Error derive macros |

### parallax-store

| Crate | Version | Purpose |
|---|---|---|
| `arc-swap` | 1 | Lock-free atomic Arc swap for snapshot publishing |
| `crc32c` | 0.6 | WAL entry checksums |
| `lz4_flex` | 0.11 | Block compression for segment files |
| `postcard` | 1 | Compact binary serialization for WAL + Segments |
| `tracing` | 0.1 | Structured logging |
| `tempfile` | 3 | (dev) Temporary directories for tests |

### parallax-graph

| Crate | Purpose |
|---|---|
| `tracing` | Structured logging |

### parallax-query

| Crate | Purpose |
|---|---|
| `tracing` | Structured logging |

### parallax-policy

| Crate | Purpose |
|---|---|
| `serde` | Policy rule serialization |
| `tracing` | Structured logging |

### parallax-ingest

| Crate | Purpose |
|---|---|
| `tracing` | Structured logging |
| `tempfile` | (dev) Temporary directories for tests |

### parallax-connect

| Crate | Purpose |
|---|---|
| `async-trait` | Async trait support for the `Connector` trait |
| `serde` | Builder serialization |
| `tokio` | Async runtime |
| `tracing` | Structured logging |
| `compact_str` | Small-string optimization |
| `thiserror` | Error types |

### parallax-server

| Crate | Purpose |
|---|---|
| `axum` | HTTP/REST server framework |
| `tower` | Middleware stack |
| `tower-http` | HTTP middleware (trace, request-id, sensitive-headers) |
| `tokio` | Async runtime |
| `serde_json` | JSON serialization for REST responses |
| `uuid` | UUID v4 for request IDs |
| `compact_str` | Small-string optimization |
| `tracing` | Structured logging |
| `thiserror` | Error types |

### parallax-cli

| Crate | Purpose |
|---|---|
| `clap` | Command-line argument parsing |
| `anyhow` | Error handling in binary context |
| `tokio` | Async runtime |
| `serde_json` | JSON output formatting |

## What We Explicitly Do Not Use

| Crate | Why Not |
|---|---|
| `rocksdb` | We own storage. No C++ FFI dependency. |
| `diesel` / `sqlx` | No SQL database. |
| `neo4j-*` | The whole point is to not depend on Neo4j. |
| `sled` | Correctness concerns in early versions. |
| `tonic` / `prost` | gRPC deferred to v0.2; REST-only for v0.1. |

## Adding a New External Dependency

Before adding a new crate:

1. Is there a `std` equivalent? Prefer `std`.
2. Does an existing approved crate already cover this? Reuse it.
3. Is the crate well-maintained, zero-unsound-unsafe, and Apache/MIT licensed?
4. Does adding it violate the acyclic dep constraint?

Add a justification comment in `Cargo.toml` for non-obvious dependencies.
