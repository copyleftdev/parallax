# Traversal Queries

The `THAT` clause traverses relationships from the entities found by `FIND`.

## Basic Traversal

```sql
-- Find hosts that run services
FIND host THAT RUNS service

-- Find users that have been assigned roles
FIND user THAT ASSIGNED role

-- Find security groups that allow internet traffic
FIND security_group THAT ALLOWS internet
```

## Multi-Hop Traversal

Chain multiple `THAT` clauses for multi-hop queries:

```sql
-- user → role → policy → resource (3 hops)
FIND user THAT ASSIGNED role THAT ALLOWS policy THAT USES aws_s3_bucket
```

Each `THAT` clause adds one hop. The entity filter after each verb narrows
which entities at that hop qualify.

## Filtering Traversal Targets

Add `WITH` after a traversal verb to filter the entities at that hop:

```sql
-- Users assigned to admin roles specifically
FIND user THAT ASSIGNED role WITH admin = true

-- Hosts running services that are publicly exposed
FIND host THAT RUNS service WITH public = true

-- Multi-hop with filters at each step
FIND user WITH active = true
  THAT ASSIGNED role WITH admin = true
  THAT ALLOWS aws_s3_bucket WITH public = true
```

## Negated Traversal (Coverage Gaps)

Prefix a verb with `!` to find entities that do **not** have a qualifying
neighbor via that relationship:

```sql
-- Hosts with no EDR agent protecting them
FIND host THAT !PROTECTS edr_agent

-- Services with no firewall
FIND service THAT !PROTECTS firewall

-- Users who have never been assigned any role
FIND user THAT !ASSIGNED role
```

**Note:** Negation (`!`) cannot appear in a chained traversal. It must be
the final `THAT` step.

```sql
-- Invalid: cannot chain after negation
FIND host THAT !PROTECTS edr_agent THAT HAS service   -- syntax error
```

## Available Verbs

PQL supports 15 relationship verbs:

| Verb | Semantic Meaning |
|---|---|
| `HAS` | Ownership or containment (`account HAS bucket`) |
| `IS` | Identity or equivalence (`user IS person`) |
| `ASSIGNED` | Role or permission assignment (`user ASSIGNED role`) |
| `ALLOWS` | Network or access permission (`policy ALLOWS resource`) |
| `USES` | Active dependency (`service USES database`) |
| `CONTAINS` | Logical grouping (`vpc CONTAINS subnet`) |
| `MANAGES` | Administrative control (`team MANAGES repo`) |
| `CONNECTS` | Network connectivity (`vpc CONNECTS vpc`) |
| `PROTECTS` | Security coverage (`edr PROTECTS host`) |
| `EXPLOITS` | Vulnerability relationship (`cve EXPLOITS package`) |
| `TRUSTS` | Trust relationship (`account TRUSTS account`) |
| `SCANS` | Scanner coverage (`scanner SCANS host`) |
| `RUNS` | Process or service execution (`host RUNS service`) |
| `READS` | Data access (read) (`app READS database`) |
| `WRITES` | Data access (write) (`app WRITES database`) |

## Traversal Direction

PQL traversal follows edges in **both directions** by default when using
the entity-type filter form. To control direction, use the Rust API directly
or rely on the verb semantics:

- `FIND host THAT RUNS service` — follows `RUNS` edges **outgoing** from host
- `FIND service THAT RUNS host` — follows `RUNS` edges **incoming** to service
  (i.e., which hosts run this service)

The query executor determines direction based on the verb and entity order.

## How Traversal Maps to Graph Operations

| PQL | Graph Operation |
|---|---|
| `FIND A THAT V B` | Find all A; for each, traverse V edges to B |
| `FIND A THAT !V B` | Find all A that have no V edges to B |
| `FIND A THAT V B THAT W C` | Find A→B→C chains |
| `FIND SHORTEST PATH FROM A TO B` | BFS bidirectional from A and B |
| `FIND BLAST RADIUS FROM A DEPTH n` | BFS from A up to n hops |
