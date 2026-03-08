# Parallax

> *"Depth perception for your attack surface."*

Parallax is an open-source, Rust-native **typed property graph engine** built
for cyber asset intelligence. It is the infrastructure layer that answers one
question:

> **Given everything we know about our assets, what is connected to what — and what does that imply?**

## What It Is

Parallax is to security asset data what SQLite is to relational data:
embeddable, zero-external-dependency, and correct enough to trust.

It solves the problem that every security team faces: ~400,000 assets spread
across ~70 tools, each tool seeing its own slice with no visibility into
the relationships between them. A graph model — entities as nodes,
relationships as directed edges — is the right abstraction. Parallax makes
that graph infrastructure **open, embeddable, and fast**.

## What It Is Not

- **Not a scanner.** It consumes telemetry; it does not generate it.
- **Not a SIEM.** It does not process event streams or alert on logs.
- **Not a UI product.** It is a library and a server; UIs are built on top.
- **Not Neo4j.** It is a domain-specific graph engine, not a general-purpose
  graph database.

## The Stack

```
parallax-cli          ← Command-line interface
parallax-server       ← REST HTTP server (Axum)
parallax-connect      ← Integration SDK (Connector trait + scheduler)
parallax-ingest       ← Sync protocol (diff + atomic commit)
parallax-policy       ← Policy evaluation engine
parallax-query        ← PQL parser, planner, executor
parallax-graph        ← Graph traversal, path-finding, blast radius
parallax-store        ← Storage engine (WAL + MemTable + Segments + MVCC)
parallax-core         ← Shared types (Entity, Relationship, Value, Timestamp)
```

Dependencies flow strictly downward. No cycles. The compiler enforces this.

## Version

This documentation covers **Parallax v0.1** — the first working vertical slice:

- Ingest via REST API or connector SDK
- Durable storage with WAL + MemTable + Segments
- Graph traversal, shortest path, blast radius, coverage gap analysis
- PQL query language (FIND / WITH / THAT / RETURN / LIMIT)
- Policy evaluation with posture scoring
- REST API with authentication and Prometheus metrics
- CLI for query, stats, and serve

## Quick Start

```bash
# Build
cargo build --release --package parallax-cli

# Start the server (no auth key = open mode)
./target/release/parallax serve --data-dir /var/lib/parallax

# Ingest some entities
curl -X POST http://localhost:7700/v1/ingest/sync \
  -H 'Content-Type: application/json' \
  -d '{
    "connector_id": "my-connector",
    "sync_id": "sync-001",
    "entities": [
      {"entity_type": "host", "entity_key": "web-01", "entity_class": "Host",
       "display_name": "Web Server 01", "properties": {"state": "running"}},
      {"entity_type": "service", "entity_key": "nginx", "entity_class": "Service",
       "display_name": "Nginx"}
    ],
    "relationships": [
      {"from_type": "host", "from_key": "web-01", "verb": "RUNS",
       "to_type": "service", "to_key": "nginx"}
    ]
  }'

# Query with PQL
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND host WITH state = '\''running'\''"}'
```

## Reading This Book

Start with [Design Principles](./architecture/design-principles.md) to
understand the "why" behind every decision. Then read
[Data Model](./architecture/data-model.md) — everything else depends on it.

After that, read in whatever order matches your interest:
- Building connectors → [Connector SDK](./connectors/overview.md)
- Querying the graph → [PQL Reference](./query/introduction.md)
- Storage internals → [Storage Engine](./storage/overview.md)
- REST API usage → [API Reference](./api/overview.md)
