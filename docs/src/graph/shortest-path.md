# Shortest Path

Shortest path finds the minimum-hop chain of relationships between two specific
entities. This answers questions like:

- "What is the access path between this user and this S3 bucket?"
- "Is there any connection between these two systems?"
- "What is the shortest privilege escalation route from this role to that secret?"

## Basic Usage

```rust
let graph = GraphReader::new(&snap);

let path = graph
    .shortest_path(user_id, s3_bucket_id)
    .find();

match path {
    Some(path) => {
        println!("Found path with {} hops", path.segments.len());
        for segment in &path.segments {
            println!("  Entity: {:?}", segment.entity_id);
        }
    }
    None => println!("No path exists"),
}
```

## ShortestPathBuilder

```rust
impl<'snap> ShortestPathBuilder<'snap> {
    /// Limit the search to this many hops (default: unlimited).
    pub fn max_depth(self, depth: usize) -> Self;

    /// Only follow edges with these verbs.
    pub fn edge_classes(self, classes: &[&str]) -> Self;

    /// Direction of edges to follow (default: Both).
    pub fn direction(self, dir: Direction) -> Self;

    /// Execute the search. Returns None if no path exists.
    pub fn find(self) -> Option<GraphPath>;
}
```

## Algorithm

Parallax implements **bidirectional BFS** — searching from both the source
and target simultaneously. This dramatically reduces the search space for
sparse graphs:

```
Unidirectional BFS: O(b^d) nodes visited (b = branching factor, d = depth)
Bidirectional BFS:  O(b^(d/2)) nodes visited
```

For a graph with branching factor 10 and path length 6:
- Unidirectional: 10^6 = 1,000,000 nodes
- Bidirectional: 10^3 = 1,000 nodes (1000× fewer)

The two frontiers meet in the middle to form the path.

**INV-Q05:** Shortest path always returns the minimum-hop path or `None`.
It never returns a longer path than the shortest one.

## Handling Cycles

The BFS searches maintain visited sets, so cyclic graphs are handled correctly.
The search terminates when:
- The two frontiers meet (path found)
- Both frontiers are exhausted (no path)
- `max_depth` is reached (INV-Q06)

## Examples

### Privilege escalation path

```rust
let path = graph
    .shortest_path(attacker_id, secret_id)
    .edge_classes(&["ASSIGNED", "ALLOWS", "HAS", "USES"])
    .direction(Direction::Both)
    .max_depth(6)
    .find();
```

### Reachability check

```rust
// Is there any connection between these two networks?
let connected = graph
    .shortest_path(network_a_id, network_b_id)
    .edge_classes(&["CONNECTS", "CONTAINS"])
    .find()
    .is_some();
```

## PQL Equivalent

```sql
-- PQL
FIND SHORTEST PATH FROM user WITH email = 'alice@corp.com'
  TO aws_s3_bucket WITH _key = 'arn:aws:s3:::secrets'

-- Rust equivalent (after resolving entity IDs)
graph.shortest_path(alice_id, bucket_id).find()
```
