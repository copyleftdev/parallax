# Graph Traversal

Traversal walks the graph from a starting entity, following edges in a
specified direction and applying filters at each step.

## Basic Traversal

```rust
let graph = GraphReader::new(&snap);

// Traverse all outgoing edges from start_id, up to depth 3
let results: Vec<TraversalResult> = graph
    .traverse(start_id)
    .direction(Direction::Outgoing)
    .max_depth(3)
    .collect();
```

## TraversalBuilder Options

```rust
pub struct TraversalBuilder<'snap> {
    ...
}

impl<'snap> TraversalBuilder<'snap> {
    /// BFS (default) or DFS traversal order.
    pub fn bfs(self) -> Self;
    pub fn dfs(self) -> Self;

    /// Direction of edges to follow.
    pub fn direction(self, dir: Direction) -> Self;
    // Direction::Outgoing, Direction::Incoming, Direction::Both

    /// Only follow edges with these relationship classes (verbs).
    pub fn edge_classes(self, classes: &[&str]) -> Self;

    /// Only visit entities with this type.
    pub fn filter_node_type(self, t: &str) -> Self;

    /// Only visit entities with this class.
    pub fn filter_node_class(self, c: &str) -> Self;

    /// Only visit entities matching this property filter.
    pub fn filter_node_property(self, key: &str, value: Value) -> Self;

    /// Stop after visiting this many hops from the start.
    pub fn max_depth(self, depth: usize) -> Self;

    /// Collect all results
    pub fn collect(self) -> Vec<TraversalResult<'snap>>;
}
```

## TraversalResult

Each visited entity produces a `TraversalResult`:

```rust
pub struct TraversalResult<'snap> {
    /// The entity reached at this hop.
    pub entity: &'snap Entity,

    /// The relationship that connected the previous hop to this entity.
    pub via: Option<&'snap Relationship>,

    /// How many hops from the start entity.
    pub depth: usize,

    /// The full path from start to this entity.
    pub path: GraphPath,
}
```

## GraphPath

```rust
pub struct GraphPath {
    /// Ordered sequence of (entity_id, relationship_id) pairs.
    pub segments: Vec<PathSegment>,
}

pub struct PathSegment {
    pub entity_id: EntityId,
    pub via_relationship: Option<RelationshipId>,
}
```

Paths are returned for every traversal result. For large traversals, omit
path tracking by using a custom iterator instead of `collect()`.

## Cycle Handling

BFS traversal maintains a `visited` set and never visits an entity twice.
This prevents infinite loops in cyclic graphs (INV-G03):

```rust
// This terminates even in a graph with cycles:
let results = graph
    .traverse(a_id)
    .direction(Direction::Outgoing)
    .max_depth(100)
    .collect();
```

DFS also maintains the visited set, so it is also cycle-safe (but produces
different ordering than BFS).

## Direction

```rust
pub enum Direction {
    /// Follow outgoing edges: entity → neighbor
    Outgoing,
    /// Follow incoming edges: entity ← neighbor
    Incoming,
    /// Follow both directions
    Both,
}
```

## Examples

### Find all services reachable from a host

```rust
let services = graph
    .traverse(host_id)
    .direction(Direction::Outgoing)
    .filter_node_class("Service")
    .max_depth(3)
    .collect();
```

### Find all entities that depend on a database

```rust
let dependents = graph
    .traverse(db_id)
    .direction(Direction::Incoming)
    .edge_classes(&["USES", "CONNECTS", "READS"])
    .max_depth(5)
    .collect();
```

### BFS vs DFS

Use **BFS** (default) when you want results ordered by proximity — closer
entities first. This is best for blast radius and impact analysis.

Use **DFS** when you want to follow a chain as deep as possible before
backtracking. This is better for path exploration.

```rust
// BFS: finds nearest entities first
let bfs = graph.traverse(start).bfs().max_depth(4).collect();

// DFS: explores deep paths first
let dfs = graph.traverse(start).dfs().max_depth(4).collect();
```
