# PQL — Parallax Query Language

PQL is the read-only query language for Parallax. It is designed for security
practitioners who are not graph database experts: readable in plain English,
learnable in 10 minutes, and predictable in performance.

## Design Goals

| Goal | How |
|---|---|
| Readable by non-engineers | English-like: `FIND`, `THAT`, `WITH`, `ALLOWS` |
| Learnable in 10 minutes | Core syntax is 5 clauses; no joins, no subqueries in v0.1 |
| Predictable performance | Every query maps to a known graph operation |
| Machine-parseable | Clean grammar → easy for AI to generate PQL from natural language |

## Non-Goals

PQL is **read-only**. All writes go through the ingest API. There is no
INSERT, UPDATE, DELETE, or MERGE in PQL.

PQL is not a general-purpose graph query language. No arbitrary pattern
matching with anonymous nodes, no recursive CTEs, no graph algorithms in the
language itself.

## Core Syntax

Every PQL query is one of three forms:

```sql
-- 1. Entity query (most common)
FIND <entity_filter>
  [WITH <property_filters>]
  [THAT <traversal_chain>]
  [RETURN <projection>]
  [LIMIT <n>]

-- 2. Shortest path query
FIND SHORTEST PATH
  FROM <entity_filter> [WITH <property_filters>]
  TO   <entity_filter> [WITH <property_filters>]
  [DEPTH <n>]

-- 3. Blast radius query
FIND BLAST RADIUS
  FROM <entity_filter> [WITH <property_filters>]
  [DEPTH <n>]
```

## The Five Clauses

### FIND

Specifies which entities to start with. The argument is either:
- An entity **type**: specific (e.g., `host`, `aws_ec2_instance`)
- An entity **class**: broad (e.g., `Host`, `User`, `DataStore`)
- `*` for any entity

```sql
FIND host
FIND Host
FIND aws_ec2_instance
FIND *
```

### WITH

Filters entities by property values. Multiple conditions are combined with AND.

```sql
FIND host WITH state = 'running'
FIND host WITH state = 'running' AND region = 'us-east-1'
FIND user WITH active = true AND email LIKE '@corp.com'
```

### THAT

Traverses relationships. Can be chained for multi-hop queries. Supports
negation with `!` to find coverage gaps.

```sql
FIND host THAT RUNS service
FIND user THAT ASSIGNED role THAT ALLOWS s3_bucket
FIND host THAT !PROTECTS edr_agent   -- hosts with no EDR
```

### RETURN

Specifies output format. Defaults to full entity objects.

```sql
FIND host RETURN COUNT              -- count only
FIND host RETURN display_name, state  -- specific properties
```

### LIMIT

Limits the number of results returned.

```sql
FIND host LIMIT 100
FIND host WITH state = 'running' LIMIT 10
```

## Quick Reference

```sql
-- All running hosts
FIND host WITH state = 'running'

-- All services on running hosts
FIND host WITH state = 'running' THAT RUNS service

-- Hosts with no EDR
FIND host THAT !PROTECTS edr_agent

-- Count of all hosts
FIND host RETURN COUNT

-- Shortest path from user to secret
FIND SHORTEST PATH FROM user WITH email = 'alice@corp.com'
  TO secret WITH name = 'prod-db-password'

-- Blast radius from compromised host
FIND BLAST RADIUS FROM host WITH _key = 'web-01' DEPTH 4
```

See [Syntax Reference](./syntax.md) for the complete grammar, and
[Examples](./examples.md) for real-world query patterns.
