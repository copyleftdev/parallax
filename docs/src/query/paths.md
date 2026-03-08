# Path Queries

Parallax supports two special query forms for path-based analysis.

## Shortest Path

`FIND SHORTEST PATH` finds the minimum-hop chain of relationships between
two specific entities.

### Syntax

```sql
FIND SHORTEST PATH
  FROM <entity_filter> [WITH <property_filters>]
  TO   <entity_filter> [WITH <property_filters>]
  [DEPTH <max_hops>]
```

### Examples

```sql
-- Is there any connection between a user and a secret?
FIND SHORTEST PATH
  FROM user WITH email = 'alice@corp.com'
  TO secret WITH name = 'prod-db-password'

-- Privilege escalation path from a guest account to admin
FIND SHORTEST PATH
  FROM user WITH role = 'guest'
  TO role WITH admin = true
  DEPTH 6

-- Network path between two VPCs
FIND SHORTEST PATH
  FROM aws_vpc WITH _key = 'vpc-prod'
  TO aws_vpc WITH _key = 'vpc-dev'
  DEPTH 10
```

### Response

If a path exists, the response includes:
- The full sequence of entities in the path
- The relationships connecting them
- The total number of hops

If no path exists within `DEPTH` hops (or at all), the response has an
empty path with `count: 0`.

### Performance

Shortest path uses bidirectional BFS — exploring from both endpoints
simultaneously. This is significantly faster than unidirectional BFS for
deep graphs:

| Graph Size | Path Length | Typical Latency |
|---|---|---|
| 10K entities | 4 hops | <5ms |
| 100K entities | 6 hops | <50ms |
| 1M entities | 8 hops | <500ms |

## Blast Radius

`FIND BLAST RADIUS` computes the set of entities reachable from a starting
point via attacker-relevant relationships.

### Syntax

```sql
FIND BLAST RADIUS
  FROM <entity_filter> [WITH <property_filters>]
  [DEPTH <max_hops>]
```

### Examples

```sql
-- What is at risk if this host is compromised?
FIND BLAST RADIUS FROM host WITH _key = 'web-01' DEPTH 4

-- What can an attacker reach from this credential?
FIND BLAST RADIUS FROM credential WITH name = 'prod-api-key' DEPTH 5

-- Impact of a vulnerable package in production
FIND BLAST RADIUS FROM package WITH cve = 'CVE-2024-1234' DEPTH 3
```

### Response

The blast radius response includes:
- `impacted`: all entities reachable within the depth limit
- `high_value_targets`: entities of high-value classes (DataStore, Secret, etc.)
- `critical_paths`: specific paths to high-value targets
- `count`: total number of impacted entities

### High-Value Target Classes

The following entity classes are always flagged as high-value targets in
blast radius results:

```
DataStore, Secret, Key, Database, Credential, Certificate, Identity, Account
```

### Depth Guidance

| Depth | Coverage | Use Case |
|---|---|---|
| 2 | Immediate neighbors | Quick triage |
| 4 | Typical blast radius | Most analyses |
| 6 | Extended blast radius | Comprehensive analysis |
| 8+ | Near-full graph | Use sparingly — can be slow |

Default depth is 4 if not specified.

## Via REST API

```bash
# Shortest path
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND SHORTEST PATH FROM user WITH email = '\''alice@corp.com'\'' TO secret WITH name = '\''prod-db-password'\''"}'

# Blast radius
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND BLAST RADIUS FROM host WITH _key = '\''web-01'\'' DEPTH 4"}'
```
