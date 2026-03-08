# Design Principles

Parallax's architecture is shaped by seven principles drawn from the systems
thinkers whose work directly influenced each layer.

## Principle 1: Interfaces Are Forever

> *Lampson: "Implementation can change; interfaces cannot."*

The entity/relationship schema, the query language (PQL), and the integration
SDK are **public contracts**. They are versioned, documented, and maintained
with the assumption that downstream consumers depend on them.

Internal storage format, memory layout, concurrency strategy — these are
implementation secrets that can change every release without notice.

**What this means in practice:**
- `EntityId`, `Entity`, `Relationship`, `Value`, and `PropertyMap` in
  `parallax-core` are stable API. Breaking changes require a major version bump.
- The WAL on-disk format (`PXWA` magic + postcard framing) can change between
  releases as long as the storage engine handles migration transparently.
- PQL syntax is a public contract. Adding new keywords is allowed; removing or
  changing semantics of existing keywords requires a deprecation cycle.

## Principle 2: Ownership Is Architecture

> *Matsakis: "Good abstractions hide complexity."*

Every datum in Parallax has exactly one owner at any point in time:

- The **graph store** owns entities and relationships.
- A **transaction** borrows the store mutably for writes; a snapshot borrows
  it immutably for reads.
- A **snapshot** is a frozen, immutable view that readers hold cheaply via
  `Arc::clone`.
- A **connector** owns its ingestion state. The engine never reaches into it.

If you cannot draw the ownership graph of a component on a whiteboard, the
component is too complex.

**In code:** `GraphReader<'snap>` ties every returned reference to the
snapshot's lifetime via Rust's borrow checker. No use-after-free. No stale
reads. No cloning on the read path.

## Principle 3: Single Writer, Many Readers

> *Bos: "Less sharing means fewer bugs."*

All mutations flow through a single serialized write path. This eliminates
write-write conflicts, deadlocks, and lock ordering bugs.

Readers operate on MVCC snapshots with zero coordination. A reader never
blocks a writer. A writer never waits for a reader.

**The numbers:** A single writer on modern NVMe processes 500K+ key-value
mutations per second after WAL batching. The largest enterprise asset graphs
ingest at ~10K entities/sec. We have 50× headroom.

## Principle 4: Separate Normal and Worst Case

> *Lampson: "Optimize for the common case; handle edge cases separately."*

**Normal case:** Ingest a batch of 50–500 entity upserts from a connector.
Execute a graph query over 10K–100K entities. Evaluate a policy rule. This
path is optimized for latency and throughput.

**Worst case:** Full re-sync of 2M assets from a new connector. WAL recovery
after crash. Compaction of stale segment files. These paths are optimized for
correctness and progress, not speed.

The normal and worst case paths may share zero code. That is acceptable.

## Principle 5: Correctness Is Non-Negotiable

> *Lamport: "If you can't specify it precisely, you don't understand it."*

Every state machine — transaction lifecycle, sync protocol, compaction —
has written invariants before implementation. See the
[Invariant Reference](./invariants.md) for the full list.

**Safety properties (must never happen):**
- A committed write is never lost.
- A read snapshot never observes a partial write.
- An entity ID is never reused for a different logical entity.
- A relationship never references a non-existent entity in a committed state.

**Liveness properties (must eventually happen):**
- A submitted write batch is eventually committed or rejected.
- A compaction cycle eventually reclaims space from deleted versions.
- A connector sync eventually converges to the source-of-truth state.

## Principle 6: Make Illegal States Unrepresentable

> *Turon: "The best API is one where the obvious thing to do is the right thing."*

If an entity cannot exist without a `_type` and `_class`, the Rust type
system enforces that — not a runtime validator.

If a relationship requires two valid entity references, the write path
enforces referential integrity at commit time (INV-03, INV-04).

If a query cursor cannot outlive its snapshot, the borrow checker enforces it.

**In practice:**
- `EntityClass::new(s)` returns `Result` — unknown classes are rejected at
  the API boundary, not silently accepted.
- `EntityId::derive()` takes `(account_id, entity_type, entity_key)` and
  produces a deterministic ID. There is no `EntityId::random()`.
- `WriteBatch` is an opaque builder — you cannot construct a batch that
  references a non-existent entity. Validation runs at ingest time.

## Principle 7: Observability as a Requirement

> *Not optional. Not "later." Now.*

Every subsystem exposes:

- **Metrics:** operation counts, latencies (p50/p99/max), queue depths —
  served at `GET /metrics` in Prometheus text format.
- **Structured logs:** every write batch commit, every sync cycle, every
  error — via `tracing` with configurable log levels.
- **Request IDs:** every HTTP request gets a `X-Request-Id` header (UUID v4)
  that is propagated through the stack for distributed tracing.

The engine must be debuggable in production without recompilation.
