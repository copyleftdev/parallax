# Summary

[Introduction](./introduction.md)

---

# Architecture

- [Overview](./architecture/overview.md)
- [Design Principles](./architecture/design-principles.md)
- [Crate Dependency Map](./architecture/crate-map.md)
- [Data Model](./architecture/data-model.md)
- [Concurrency Model](./architecture/concurrency.md)
- [Invariant Reference](./architecture/invariants.md)

---

# Storage Engine

- [Overview](./storage/overview.md)
- [Write-Ahead Log (WAL)](./storage/wal.md)
- [MemTable](./storage/memtable.md)
- [Segments](./storage/segments.md)
- [MVCC Snapshots](./storage/snapshots.md)
- [StorageEngine API](./storage/engine-api.md)

---

# Graph Engine

- [Overview](./graph/overview.md)
- [GraphReader](./graph/reader.md)
- [Entity Finder](./graph/finder.md)
- [Traversal](./graph/traversal.md)
- [Shortest Path](./graph/shortest-path.md)
- [Blast Radius](./graph/blast-radius.md)
- [Coverage Gap](./graph/coverage-gap.md)

---

# PQL — Parallax Query Language

- [Introduction](./query/introduction.md)
- [Syntax Reference](./query/syntax.md)
- [Property Filters](./query/filters.md)
- [Traversal Queries](./query/traversal.md)
- [Path Queries](./query/paths.md)
- [Query Execution](./query/execution.md)
- [Examples](./query/examples.md)

---

# Connector SDK

- [Overview](./connectors/overview.md)
- [Writing a Connector](./connectors/writing.md)
- [Step Definitions](./connectors/steps.md)
- [Entity & Relationship Builders](./connectors/builders.md)
- [Sync Protocol](./connectors/sync.md)
- [Observability](./connectors/observability.md)

---

# REST API

- [Overview](./api/overview.md)
- [Authentication](./api/authentication.md)
- [Query Endpoints](./api/query.md)
- [Ingest Endpoints](./api/ingest.md)
- [Entity Endpoints](./api/entities.md)
- [Admin Endpoints](./api/admin.md)
- [Metrics](./api/metrics.md)
- [Error Responses](./api/errors.md)

---

# Policy Engine

- [Overview](./policy/overview.md)
- [Policy Rules](./policy/rules.md)
- [Evaluation](./policy/evaluation.md)
- [Posture Scoring](./policy/posture.md)

---

# Reference

- [Known Entity Classes](./reference/entity-classes.md)
- [Known Relationship Verbs](./reference/relationship-verbs.md)
- [Value Types](./reference/value-types.md)
- [Configuration](./reference/configuration.md)
- [CLI Reference](./reference/cli.md)
- [Performance Targets](./reference/performance.md)
- [v0.2 Roadmap](./reference/roadmap.md)
