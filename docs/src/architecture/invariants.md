# Invariant Reference

Parallax formalizes correctness as numbered invariants across all specs.
Every code change should be checked against the invariants it might affect.
Invariants are referenced in commit messages using their codes.

## Data Model Invariants (INV-01..08)

From `specs/01-data-model.md`:

| Code | Invariant |
|---|---|
| **INV-01** | Every entity has a non-empty `_type`, `_class`, and entity key. |
| **INV-02** | `EntityId` is deterministic: same `(account_id, type, key)` always produces the same ID. |
| **INV-03** | Every relationship's `from_id` and `to_id` reference entities that exist in the committed graph. |
| **INV-04** | No two entities in the same account share `(type, key)`. |
| **INV-05** | No two relationships share `(from_id, class, to_id)` unless explicitly keyed with `derive_with_key`. |
| **INV-06** | Timestamps are monotonically increasing per node. |
| **INV-07** | Property types are stable within an entity type across versions (no type changes). |
| **INV-08** | Property values are flat — no nested objects or arrays-of-objects. |

## Storage Engine Invariants (INV-S01..S08)

From `specs/02-storage-engine.md`:

| Code | Invariant |
|---|---|
| **INV-S01** | A committed write is never lost, even after a crash. |
| **INV-S02** | A read snapshot never observes a partial write. |
| **INV-S03** | Snapshots are monotonically increasing — a newer snapshot always supersedes an older one. |
| **INV-S04** | WAL entries are append-only and immutable after write. |
| **INV-S05** | A WAL entry with a CRC mismatch is treated as corrupt; recovery stops at the last valid entry. |
| **INV-S06** | MemTable flush is atomic: either all entries move to a segment or none do. |
| **INV-S07** | Segment files are immutable after creation. Compaction produces new segments, never modifies old ones. |
| **INV-S08** | An entity with `_deleted = true` must never appear in query results. |

## Graph Engine Invariants (INV-G01..G06)

From `specs/03-graph-engine.md`:

| Code | Invariant |
|---|---|
| **INV-G01** | `GraphReader<'snap>` references cannot outlive their snapshot (enforced by the borrow checker). |
| **INV-G02** | Traversal never follows edges to deleted entities. |
| **INV-G03** | BFS traversal visits each entity at most once (no infinite loops in cyclic graphs). |
| **INV-G04** | Shortest path returns `None` when no path exists; it never returns an incorrect path. |
| **INV-G05** | Blast radius computation is bounded by `max_depth` (default: 4). |
| **INV-G06** | Coverage gap analysis only returns entities of `target_type` that have no qualifying neighbor. |

## Query Language Invariants (INV-Q01..Q06)

From `specs/04-query-language.md`:

| Code | Invariant |
|---|---|
| **INV-Q01** | PQL parsing is deterministic: the same query string always produces the same AST. |
| **INV-Q02** | A query never returns results from entities that do not satisfy all specified filters. |
| **INV-Q03** | `LIMIT n` applied to a query returns at most `n` results. |
| **INV-Q04** | A query that times out returns an error, not a partial result. |
| **INV-Q05** | `FIND SHORTEST PATH FROM A TO B` returns the minimum-hop path or `None`; never a longer path. |
| **INV-Q06** | `FIND BLAST RADIUS FROM X DEPTH n` returns only entities reachable within `n` hops. |

## Connector SDK Invariants (INV-C01..C06)

From `specs/05-integration-sdk.md`:

| Code | Invariant |
|---|---|
| **INV-C01** | A sync commit is atomic: either all entities/relationships land or none do. |
| **INV-C02** | Entities from connector A are never deleted by a sync from connector B. |
| **INV-C03** | Entity IDs are deterministic from `(account_id, type, key)` — same as INV-02. |
| **INV-C04** | A relationship in a sync batch whose `from_id` or `to_id` does not exist (in batch or graph) is rejected. |
| **INV-C05** | Step dependencies form a DAG — circular step dependencies are rejected at connector load time. |
| **INV-C06** | A failed step does not prevent independent steps from running. |

## API Surface Invariants (INV-A01..A06)

From `specs/06-api-surface.md`:

| Code | Invariant |
|---|---|
| **INV-A01** | All write endpoints require authentication when an API key is configured. |
| **INV-A02** | API key comparison uses constant-time equality to prevent timing attacks. |
| **INV-A03** | Every committed write is visible to subsequent reads on the same server instance. |
| **INV-A04** | Query responses are paginated; no single response exceeds the configured `max_results` limit. |
| **INV-A05** | Every request has a `X-Request-Id` header (generated or propagated) for tracing. |
| **INV-A06** | The `/v1/health` endpoint is exempt from authentication. |

## Policy Engine Invariants (INV-P01..P06)

From `specs/08-policy-engine.md`:

| Code | Invariant |
|---|---|
| **INV-P01** | A policy rule with an invalid PQL query is rejected at load time, not at evaluation time. |
| **INV-P02** | Policy evaluation never modifies the graph. It is read-only. |
| **INV-P03** | A rule that errors during evaluation is recorded as an error, not as a pass or fail. |
| **INV-P04** | Posture score is computed from all loaded rules, including errored ones (they count as failures). |
| **INV-P05** | Framework mapping (CIS, NIST, PCI-DSS) is metadata on rules; it does not affect evaluation logic. |
| **INV-P06** | Policy rules are validated against the PQL parser before being accepted into the rule set. |

## How to Use This Reference

When modifying code, ask: *Which invariants does this change touch?*

Reference affected invariants in your commit message:

```
fix(ingest): reject dangling relationships at commit time (INV-C04, INV-03)
feat(store): implement WAL CRC32C verification (INV-S05)
refactor(graph): bound traversal depth to prevent unbounded BFS (INV-G03)
```

When adding new behavior, ask: *Does this require a new invariant?* If yes,
add it to the appropriate spec file and to this reference page.
