# Query Execution

PQL queries go through a three-stage pipeline: parse → plan → execute.

## Stage 1: Parse

The PQL lexer tokenizes the input string, then the recursive descent parser
produces an AST.

```rust
use parallax_query::parse;

let ast = parse("FIND host WITH state = 'running'")?;
```

The parser is hand-written (no parser generator) for several reasons:
- Precise error messages: "Expected '=' after property name 'state', got '<'"
- Zero additional dependencies
- Full control over error recovery

**INV-Q01:** The same query string always produces the same AST (deterministic).

### Parse Errors

Parse errors include the position of the unexpected token:

```
Error: unexpected token '=' at position 18
  FIND host WITH state = = 'running'
                       ^
  expected: comparison operator (=, !=, <, <=, >, >=)
```

## Stage 2: Plan

The planner transforms the AST into a `QueryPlan` — a concrete execution
strategy. It consults `IndexStats` to choose the most efficient access path.

```rust
use parallax_query::{parse, plan};

let ast = parse("FIND host WITH state = 'running'")?;
let query_plan = plan(&ast, &index_stats)?;
```

### Index Stats

`IndexStats` tracks entity counts by type and class:

```rust
pub struct IndexStats {
    pub type_counts: HashMap<String, u64>,   // "host" → 1234
    pub class_counts: HashMap<String, u64>,  // "Host" → 5678
    pub entity_count: u64,
    pub relationship_count: u64,
}
```

### Access Strategy Selection

| Query Pattern | Chosen Strategy | Reason |
|---|---|---|
| `FIND host` | `TypeIndexScan` | Type index available |
| `FIND Host` | `ClassIndexScan` | Class index available |
| `FIND *` | `FullScan` | No narrowing possible |
| `FIND host WITH state='running'` | `TypeIndexScan` + filter | Type index reduces candidates |

## Stage 3: Execute

The executor runs the query plan against an MVCC snapshot:

```rust
use parallax_query::execute;

let snap = engine.snapshot();
let result = execute(&query_plan, &snap, &limits)?;
```

### QueryLimits

```rust
pub struct QueryLimits {
    pub max_results: usize,      // default: 10_000
    pub timeout: Duration,       // default: 30s
    pub max_traversal_depth: usize,  // default: 10
}
```

**INV-Q03:** `max_results` is a hard cap — the query returns at most this
many results.

**INV-Q04:** If the query takes longer than `timeout`, it returns an error,
not a partial result.

### QueryResult

```rust
pub enum QueryResult {
    Entities(Vec<Entity>),
    Traversals(Vec<TraversalResult>),
    Paths(Vec<GraphPath>),
    Scalar(u64),   // For RETURN COUNT
}
```

## Execution Plan Examples

### `FIND host WITH state = 'running'`

```
QueryPlan::Find {
    access: TypeIndexScan { entity_type: "host" },
    filters: [PropertyFilter { key: "state", op: Eq, value: "running" }],
    return_: ReturnAll,
    limit: None,
}
```

Execution:
1. Load type index entry for `"host"` → `[EntityId1, EntityId2, ...]`
2. For each ID, fetch entity from snapshot
3. Apply property filter `state = 'running'`
4. Return matching entities

### `FIND host THAT RUNS service`

```
QueryPlan::Traversal {
    start: Find { access: TypeIndexScan { "host" }, filters: [] },
    steps: [TraversalStep { verb: "RUNS", target_type: "service", negated: false }],
    return_: ReturnAll,
}
```

Execution:
1. Load all hosts from type index
2. For each host, follow outgoing `RUNS` edges via adjacency index
3. Filter targets to `entity_type == "service"`
4. Return matching host + service pairs

### `FIND host RETURN COUNT`

```
QueryPlan::Find {
    access: TypeIndexScan { "host" },
    filters: [],
    return_: ReturnCount,
    limit: None,
}
```

Execution:
1. Load type index entry for `"host"`
2. Count entries without fetching entity data
3. Return scalar count

## Performance Characteristics

| Query Type | Typical Latency (10K entities) |
|---|---|
| Type scan, no filter | <100μs |
| Type scan + property filter | <1ms |
| Single-hop traversal | <500μs |
| 3-hop traversal | <5ms |
| Shortest path | <10ms |
| Blast radius (depth 4) | <10ms |
| `RETURN COUNT` | <50μs (index only) |
