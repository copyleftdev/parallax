# Graph Engine Overview

`parallax-graph` is the reasoning layer between raw storage and the query
language. It takes an MVCC snapshot and exposes graph-aware operations:
traversal, pattern matching, shortest path, blast radius, and coverage gap.

## The GraphReader

All graph operations start with a `GraphReader<'snap>`, which borrows an
immutable snapshot and provides zero-copy access to graph data:

```rust
use parallax_graph::GraphReader;

let snap = engine.snapshot();
let graph = GraphReader::new(&snap);

// Entity finder
let hosts: Vec<&Entity> = graph
    .find("host")
    .with_property("state", Value::from("running"))
    .collect();

// Traversal
let neighbors = graph
    .traverse(start_id)
    .direction(Direction::Outgoing)
    .max_depth(3)
    .collect();

// Shortest path
let path = graph
    .shortest_path(from_id, to_id)
    .find();

// Blast radius
let blast = graph
    .blast_radius(target_id)
    .add_attack_edge("RUNS", Direction::Outgoing)
    .analyze();

// Coverage gap
let uncovered = graph
    .coverage_gap("PROTECTS")
    .target_type("host")
    .neighbor_type("edr_agent")
    .find();
```

## Lifetime Discipline

`GraphReader<'snap>` ties every returned reference to the snapshot's
lifetime via Rust's borrow checker. You cannot accidentally hold a reference
to a entity after the snapshot is dropped.

```rust
let entity: &Entity = {
    let snap = engine.snapshot();
    let graph = GraphReader::new(&snap);
    graph.get_entity(id).unwrap()
    // ERROR: snap dropped here, but entity borrows from it
};
```

This compile-time guarantee eliminates an entire class of use-after-free
and stale-read bugs.

## Operations Summary

| Operation | Builder | Description |
|---|---|---|
| Entity finder | `GraphReader::find()` | Filter entities by type, class, properties |
| All entities | `GraphReader::find_all()` | Return all non-deleted entities |
| By class | `GraphReader::find_by_class()` | Filter by entity class |
| Traversal | `GraphReader::traverse()` | BFS/DFS from a starting entity |
| Shortest path | `GraphReader::shortest_path()` | Minimum-hop path between two entities |
| Blast radius | `GraphReader::blast_radius()` | Attack impact analysis from a target |
| Coverage gap | `GraphReader::coverage_gap()` | Find entities with no qualifying neighbor |
| Direct lookup | `GraphReader::get_entity()` | O(1) entity lookup by ID |

## Performance

The graph engine is designed for interactive query latency:

| Operation | Target p99 |
|---|---|
| Entity lookup by ID (MemTable) | ≤1μs |
| Single-hop traversal (degree ≤100) | ≤500μs |
| Multi-hop traversal (depth 3, degree 5) | ≤5ms |
| Shortest path (1K-node graph) | ≤10ms |
| Blast radius (depth 4) | ≤10ms |

These targets assume data in MemTable. Segment reads add ~100μs per entity
lookup due to linear scan; segment indexing is planned for v0.2.
