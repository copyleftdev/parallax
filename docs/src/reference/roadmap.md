# Roadmap

v0.1 delivered the complete vertical slice: ingest → store → graph → query → serve.
v0.2 focused on performance, operability, and the policy engine.

## v0.2 — Completed

### Performance

| Feature | Status | Notes |
|---|---|---|
| **WAL group commit** | ✅ Done | `append_batch()` — single fsync per batch |
| **Segment sparse index** | ✅ Done | Binary-search index, O(log n) lookups |
| **Background compaction** | ✅ Done | `CompactionWorker` thread, merge small segments |
| **Property index** | Deferred | Secondary index deferred to v0.3 |

### Query Language

| Feature | Status | Notes |
|---|---|---|
| **OR in filters** | ✅ Done | `WITH state = 'a' OR state = 'b'` |
| **NOT EXISTS** | ✅ Done | `WITH NOT owner EXISTS` |
| **GROUP BY** | ✅ Done | `FIND host GROUP BY os` → `QueryResult::Grouped` |
| **Field projection** | Deferred | `RETURN field1, field2` parses but returns full entities |
| **Parameterized queries** | Deferred | |

### Connector SDK

| Feature | Status | Notes |
|---|---|---|
| **Parallel step execution** | ✅ Done | `topological_waves()` + `JoinSet` per wave |
| **WASM connector sandbox** | Deferred | |
| **Connector config schema** | Deferred | |

### Policy Engine

| Feature | Status | Notes |
|---|---|---|
| **YAML rule files** | ✅ Done | `load_rules_from_yaml()`, `serde_yaml` |
| **Policy REST API** | ✅ Done | GET/POST `/v1/policies`, POST `/v1/policies/evaluate`, GET `/v1/policies/posture` |
| **Parallel evaluation** | ✅ Done | `par_evaluate_all()` via `std::thread::scope` |
| **Scheduled evaluation** | Deferred | |

### Observability

| Feature | Status | Notes |
|---|---|---|
| **JSON log format** | ✅ Done | `--log-format json` global CLI flag |
| **`parallax wal dump`** | ✅ Done | `parallax wal dump [--verbose]` |
| **Rich Prometheus metrics** | Partial | Basic counters; histograms deferred |
| **OpenTelemetry traces** | Deferred | |

## v0.3 — Planned

- Property secondary index (fast `WITH state=X` without full scan)
- Field projection (`RETURN display_name, state`)
- Parameterized queries (`FIND host WITH state = $1`)
- Scheduled policy evaluation (cron-based)
- gRPC via tonic
- First-party connectors: `connector-aws`, `connector-okta`, `connector-github`

## Known Limitations

1. **Field projection parses but is not enforced:** `FIND host RETURN display_name`
   parses without error but still returns full entity objects. Projection requires
   an architectural change to the entity return type.

2. **Single-node only:** No replication, no clustering. The storage format is
   designed to support replication but it is not implemented.

3. **No gRPC:** Only REST. gRPC is architecturally planned but not implemented.

4. **Soft class/verb enforcement:** Unknown entity classes and relationship verbs
   produce warnings, not hard errors.

## Known Deferred Items

- `crates/parallax-query/src/executor.rs` — field projection (`RETURN` clause returns full entities, not projected fields)

## Versioning Policy

- **v0.x:** Breaking changes between minor versions are allowed.
- **v1.0:** Stable public API. Breaking changes require a major version bump.
- **MSRV:** Latest stable Rust minus 2 releases.

The PQL language syntax and the entity/relationship schema are treated as
public API even before v1.0 — changes go through a deprecation cycle.
