<p align="center">
  <img src="media/logo.png" alt="Parallax" width="480">
</p>

<p align="center">
  <strong>Rust-native graph engine for cyber asset intelligence.</strong>
</p>

<p align="center">
  <a href="https://github.com/copyleftdev/parallax/actions/workflows/ci.yml"><img src="https://github.com/copyleftdev/parallax/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://copyleftdev.github.io/parallax/"><img src="https://img.shields.io/badge/docs-pages-blue" alt="Docs"></a>
  <a href="https://github.com/copyleftdev/parallax/blob/main/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2F%20MIT-blue" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-1.76%2B-orange" alt="Rust 1.76+">
</p>

---

Parallax models your infrastructure as a property graph — entities as nodes,
relationships as edges — and lets you query it with PQL, a purpose-built
graph query language. It ships as a Cargo workspace of focused library crates
and a standalone HTTP server, with no external storage engine dependencies.

```
FIND host THAT !PROTECTS edr_agent                  -- hosts without EDR coverage
FIND host WITH os = 'linux' THAT USES aws_iam_role WITH admin = true  -- over-privileged EC2
FIND user WITH mfaActive = false THAT ASSIGNED role -- MFA gap
```

---

## Why Parallax

Security teams manage hundreds of thousands of assets across dozens of tools.
Each tool sees its own slice. No tool sees the relationships between slices.

A graph model — entities connected by typed, directed relationships — is the
right abstraction. Parallax gives you that model as **infrastructure**: open,
embeddable, fast, and correct, without locking you into a SaaS platform or an
external graph database.

---

## Features

- **Custom storage engine** — append-only WAL, in-memory MemTable, immutable
  segments, MVCC snapshots. No RocksDB, no SQLite, no external dependencies.
- **PQL query language** — `FIND`, `WITH`, `THAT`, `GROUP BY`, `SHORTEST PATH`,
  `BLAST RADIUS`. Hand-written lexer and recursive-descent parser.
- **Connector SDK** — implement the `Connector` trait; the scheduler handles
  topological step ordering, parallel wave execution, and source-scoped diffing.
- **Policy engine** — YAML rule files, PQL-backed evaluation, parallel execution
  via `std::thread::scope`, compliance posture scoring per framework.
- **REST API** — Axum-based HTTP server with token auth, Prometheus metrics,
  and a full ingest/query/policy surface.
- **CLI** — `parallax serve`, `parallax query`, `parallax stats`, `parallax wal dump`.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  parallax-cli          parallax-server                  │
│        │                      │                         │
│  parallax-connect    parallax-query    parallax-policy  │
│        │                 │                  │           │
│        └──────── parallax-ingest ───────────┘           │
│                          │                              │
│                  parallax-graph                         │
│                          │                              │
│                  parallax-store                         │
│                          │                              │
│                  parallax-core                          │
└─────────────────────────────────────────────────────────┘
```

Dependency flow is strictly acyclic. Each crate has a single responsibility.

| Crate | Role |
|---|---|
| `parallax-core` | `EntityId`, `Entity`, `Relationship`, `Value`, `Timestamp` (HLC) |
| `parallax-store` | WAL, MemTable, Segment, Snapshot, `StorageEngine` |
| `parallax-graph` | `GraphReader`, traversal, shortest path, blast radius |
| `parallax-query` | PQL lexer, parser, planner, executor |
| `parallax-policy` | YAML rules, `PolicyEvaluator`, posture scoring |
| `parallax-ingest` | `SyncEngine`, source-scoped diffing |
| `parallax-connect` | `Connector` trait, step scheduler, `run_connector()` |
| `parallax-server` | Axum HTTP server, REST API |
| `parallax-cli` | Binary entry point |

---

## Quick Start

### Build from source

```bash
git clone https://github.com/copyleftdev/parallax
cd parallax
cargo build --release
```

### Run the server

```bash
cargo run --package parallax-cli -- serve --data-dir /tmp/parallax-data
```

### Query

```bash
# In another terminal
parallax query "FIND host WITH os = 'linux'"
parallax query "FIND host THAT !PROTECTS edr_agent"
parallax stats
```

---

## PQL — Parallax Query Language

PQL is a declarative graph query language. Every query starts with `FIND`.

### Entity lookup

```pql
FIND host
FIND host WITH os = 'linux'
FIND host WITH os = 'linux' OR os = 'windows'
FIND host WITH state = 'running' AND env = 'prod'
FIND *                                              -- all entities
```

### Relationship traversal

```pql
FIND host THAT USES aws_iam_role
FIND host THAT !PROTECTS edr_agent                  -- negated (missing relationship)
FIND user THAT ASSIGNED role WITH admin = true
```

### Path queries

```pql
SHORTEST PATH FROM 'instance-001' TO 'prod-db-01'
BLAST RADIUS FROM 'compromised-host'
```

### Aggregation

```pql
FIND host GROUP BY os                               -- group by property
FIND host RETURN COUNT                              -- scalar count
FIND host WITH os = 'linux' LIMIT 100
```

---

## Connector SDK

Connectors pull data from external sources and emit entities and relationships
into the graph. Implement the `Connector` trait:

```rust
use parallax_connect::{Connector, StepContext, StepDefinition, ConnectorError};
use parallax_connect::builder::{entity, relationship};
use async_trait::async_trait;

pub struct MyConnector;

#[async_trait]
impl Connector for MyConnector {
    fn name(&self) -> &str { "my-source" }

    fn steps(&self) -> Vec<StepDefinition> {
        vec![
            step("hosts",    "Emit host inventory").build(),
            step("services", "Emit services")
                .depends_on(&["hosts"])
                .build(),
        ]
    }

    async fn execute_step(
        &self,
        step_id: &str,
        ctx: &mut StepContext,
    ) -> Result<(), ConnectorError> {
        match step_id {
            "hosts" => {
                ctx.emit_entity(
                    entity("host", "web-01")
                        .class("Host")
                        .display_name("web-01")
                        .property("os", "linux")
                        .property("active", true),
                )?;
            }
            _ => {}
        }
        Ok(())
    }
}
```

The scheduler automatically computes wave order from step dependencies and
runs each wave's steps concurrently via `tokio::task::JoinSet`.

### Synthetic connectors (for testing)

```rust
use connector_aws_synthetic::AwsSyntheticConnector;

// 100 EC2 instances, 70% EDR coverage (30% gap for policy testing)
let connector = AwsSyntheticConnector::realistic(100);

// Clean baseline — all policies pass
let connector = AwsSyntheticConnector::clean(50);

// Worst case — maximum violations
let connector = AwsSyntheticConnector::worst_case(50);
```

GCP synthetic is available via `connector-gcp-synthetic` with the same API.

---

## Policy Engine

Define security rules in YAML. Each rule is a PQL query — entities returned
by the query are violations.

```yaml
rules:
  - id: edr-coverage-001
    name: Hosts without EDR
    severity: high
    query: "FIND host THAT !PROTECTS edr_agent"
    enabled: true
    schedule: "every:1h"
    frameworks:
      - framework: CIS-Controls-v8
        control: "10.1"
    remediation: Deploy an EDR agent to all active hosts.

  - id: mfa-all-users
    name: Users without MFA
    severity: high
    query: "FIND user WITH mfaActive = false"
    enabled: true
    frameworks:
      - framework: CIS-Controls-v8
        control: "6.5"
```

```rust
use parallax_policy::{PolicyEvaluator, load_rules_from_yaml};

let rules = load_rules_from_yaml(Path::new("rules/security.yaml"))?;
let evaluator = PolicyEvaluator::load(rules, &index_stats)?;

// Parallel evaluation — all rules run concurrently on separate OS threads
let results = evaluator.par_evaluate_all(&graph, QueryLimits::default());

// Compliance posture
let posture = compute_posture("CIS-Controls-v8", &rules, &results);
println!("Score: {:.0}%", posture.overall_score * 100.0);
```

---

## REST API

| Method | Path | Description |
|---|---|---|
| `GET` | `/v1/health` | Health check (no auth) |
| `GET` | `/v1/stats` | Entity and relationship counts |
| `POST` | `/v1/query` | Execute a PQL query |
| `GET` | `/v1/entities/:id` | Fetch entity by ID |
| `POST` | `/v1/ingest/sync` | Submit a sync batch |
| `GET` | `/v1/connectors` | List registered connectors |
| `GET` | `/v1/policies` | List loaded policy rules |
| `POST` | `/v1/policies` | Replace the rule set |
| `POST` | `/v1/policies/evaluate` | Evaluate all rules |
| `GET` | `/v1/policies/posture` | Compliance posture score |
| `GET` | `/v1/metrics` | Prometheus metrics |

Authentication: `Authorization: Bearer <token>` header. Set token via
`PARALLAX_API_TOKEN` environment variable or `--token` flag.

---

## Status

Current version: **v0.2**

| Area | Status |
|---|---|
| Storage engine (WAL + segments + MVCC) | Stable |
| PQL parser + executor | Stable |
| Graph traversal (BFS/DFS, shortest path, blast radius) | Stable |
| Policy engine (YAML rules, parallel evaluation, posture) | Stable |
| Connector SDK (parallel wave execution) | Stable |
| REST API | Stable |
| CLI | Stable |
| Field projection (`RETURN field1, field2`) | Deferred to v0.3 |
| Parameterized queries (`WITH state = $1`) | Deferred to v0.3 |
| gRPC | Deferred to v0.3 |

See [roadmap](docs/src/reference/roadmap.md) for the full plan.

---

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
