# Blast Radius

Blast radius analysis answers: *"If this entity is compromised, what else is
at risk?"*

Given a starting entity (e.g., a compromised host or credential), blast radius
follows attacker-relevant relationships to identify all entities within attack
reach.

## Basic Usage

```rust
let graph = GraphReader::new(&snap);

let result = graph
    .blast_radius(compromised_host_id)
    .add_attack_edge("RUNS", Direction::Outgoing)   // host → services
    .add_attack_edge("CONNECTS", Direction::Outgoing) // host → other hosts
    .max_depth(4)
    .analyze();

println!("Impacted entities: {}", result.impacted.len());
println!("Critical paths: {}", result.critical_paths.len());
println!("High-value targets: {}", result.high_value_targets.len());
```

## BlastRadiusBuilder

```rust
impl<'snap> BlastRadiusBuilder<'snap> {
    /// Add an attack path edge type with direction.
    /// Call multiple times for multiple attack vectors.
    pub fn add_attack_edge(self, verb: &str, direction: Direction) -> Self;

    /// Maximum hops from the origin entity (default: 4).
    pub fn max_depth(self, depth: usize) -> Self;

    /// Run the analysis and return results.
    pub fn analyze(self) -> BlastRadiusResult<'snap>;
}
```

## BlastRadiusResult

```rust
pub struct BlastRadiusResult<'snap> {
    /// The origin entity — the starting point of the attack.
    pub origin: &'snap Entity,

    /// All entities reachable via the specified attack edges.
    pub impacted: Vec<&'snap Entity>,

    /// Paths to entities classified as high-value targets.
    pub critical_paths: Vec<GraphPath>,

    /// High-value targets in the blast radius.
    /// These are entities whose class is in the high-value set.
    pub high_value_targets: Vec<&'snap Entity>,
}
```

## High-Value Target Classes

The following entity classes are automatically identified as high-value targets
when found in the blast radius:

```
DataStore   Secret      Key         Database
Credential  Certificate Identity    Account
```

These classes represent data and access assets that attackers specifically target.

## Default Attack Edges

If no attack edges are specified, the blast radius builder uses a default set
of attacker-relevant relationship verbs:

```
RUNS, CONNECTS, TRUSTS, CONTAINS, HAS, USES, EXPLOITS
```

These cover the most common lateral movement patterns.

## Example: Compromised Credential

```rust
// If an attacker has this credential, what can they access?
let result = graph
    .blast_radius(credential_id)
    .add_attack_edge("ALLOWS", Direction::Outgoing)  // credential → policies
    .add_attack_edge("ASSIGNED", Direction::Incoming) // who was assigned this
    .add_attack_edge("HAS", Direction::Incoming)      // who owns this
    .max_depth(5)
    .analyze();

// Check if any datastores are in the blast radius
let exposed_datastores: Vec<_> = result.high_value_targets.iter()
    .filter(|e| e._class.as_str() == "DataStore")
    .collect();

if !exposed_datastores.is_empty() {
    println!("CRITICAL: {} datastores exposed", exposed_datastores.len());
}
```

## Example: PQL Equivalent

The blast radius builder corresponds to PQL's `FIND BLAST RADIUS FROM` syntax:

```sql
-- PQL
FIND BLAST RADIUS FROM host WITH _key = 'web-01' DEPTH 4

-- Rust equivalent
graph.blast_radius(host_id).max_depth(4).analyze()
```

## Performance

Blast radius uses BFS internally and visits each entity at most once. For
a graph with 100K entities and average degree 10, expect:

- Depth 2: ~100 entities visited, <1ms
- Depth 4: ~10,000 entities visited, ~10ms
- Depth 6: ~100,000 entities visited, ~100ms

Set `max_depth` conservatively — attacker lateral movement rarely exceeds
4-6 hops in practice.
