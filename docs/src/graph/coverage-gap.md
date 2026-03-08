# Coverage Gap

Coverage gap analysis finds entities that are **missing a qualifying neighbor**.
This answers questions like:

- "Which hosts have no EDR agent protecting them?"
- "Which services have no scanner scanning them?"
- "Which databases have no backup agent connected to them?"

## Basic Usage

```rust
let graph = GraphReader::new(&snap);

// Find all hosts with no EDR agent protecting them
let unprotected = graph
    .coverage_gap("PROTECTS")
    .target_type("host")
    .neighbor_type("edr_agent")
    .find();

println!("{} hosts have no EDR coverage", unprotected.len());
```

## CoverageGapBuilder

```rust
impl<'snap> CoverageGapBuilder<'snap> {
    /// The relationship verb that represents coverage.
    /// e.g., "PROTECTS", "SCANS", "MANAGES"
    /// (This is the first argument to coverage_gap())

    /// The type of entity to check for coverage gaps.
    pub fn target_type(self, t: &str) -> Self;

    /// The class of entity that provides coverage.
    pub fn target_class(self, c: &str) -> Self;

    /// The type of entity that provides coverage (the "covering" entity).
    pub fn neighbor_type(self, t: &str) -> Self;

    /// The class of the covering entity.
    pub fn neighbor_class(self, c: &str) -> Self;

    /// Direction of the coverage edge (default: Incoming — neighbor → target).
    pub fn direction(self, dir: Direction) -> Self;

    /// Execute and return all entities with no qualifying neighbor.
    pub fn find(self) -> Vec<&'snap Entity>;
}
```

## INV-G06

**INV-G06:** Coverage gap only returns entities of `target_type` that have
no qualifying neighbor via the specified verb. Entities that have at least
one qualifying neighbor are excluded.

## Examples

### Scanner coverage

```rust
// Which hosts have never been scanned?
let unscanned = graph
    .coverage_gap("SCANS")
    .target_type("host")
    .neighbor_class("Scanner")
    .direction(Direction::Incoming)  // scanner → host
    .find();
```

### EDR protection

```rust
// Which containers have no security agent?
let unprotected = graph
    .coverage_gap("PROTECTS")
    .target_class("Container")
    .neighbor_class("Agent")
    .find();
```

### Backup coverage

```rust
// Which databases have no backup relationship?
let unbackedup = graph
    .coverage_gap("HAS")
    .target_class("Database")
    .neighbor_type("backup_job")
    .find();
```

## PQL Equivalent

Coverage gap corresponds to PQL's negated traversal:

```sql
-- PQL: find hosts with no EDR protection
FIND host THAT !PROTECTS edr_agent

-- Rust equivalent
graph.coverage_gap("PROTECTS").target_type("host").neighbor_type("edr_agent").find()
```

## Performance

Coverage gap requires:
1. Fetch all entities of `target_type` (index scan)
2. For each entity, check if any `verb` edge exists to a qualifying neighbor
   (adjacency lookup)

The adjacency index makes step 2 O(degree) per entity, not O(n_relationships).
For 10,000 hosts with average degree 5, expect:
- 10,000 adjacency lookups × O(5) = 50,000 operations
- Typically completes in <50ms
