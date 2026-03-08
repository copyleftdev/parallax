# Property Filters

The `WITH` clause filters entities by their properties. All conditions in a
`WITH` clause are combined with AND.

## Comparison Operators

```sql
-- Equality
FIND host WITH state = 'running'

-- Inequality
FIND host WITH state != 'terminated'

-- Numeric comparisons
FIND host WITH cpu_count > 4
FIND host WITH memory_gb >= 32
FIND host WITH score < 7.5
FIND host WITH version <= 3
```

## String Matching

```sql
-- Exact match (case-sensitive)
FIND user WITH email = 'alice@corp.com'

-- Pattern match with LIKE (% = any sequence, _ = any single char)
FIND user WITH email LIKE '%@corp.com'
FIND host WITH hostname LIKE 'web-%'
```

## Boolean

```sql
FIND host WITH active = true
FIND aws_s3_bucket WITH public = false
FIND user WITH mfa_enabled = false
```

## Null Checks

```sql
-- Field has no value
FIND container WITH cpu_limit = null

-- Field has any value (EXISTS)
FIND host WITH owner EXISTS

-- Field has no value (NOT EXISTS equivalent)
FIND host WITH NOT owner EXISTS
```

## IN List

```sql
-- State is one of the listed values
FIND host WITH state IN ('running', 'pending', 'starting')

-- Region is one of the listed values
FIND host WITH region IN ('us-east-1', 'us-west-2', 'eu-west-1')
```

## NOT

```sql
-- Negate any condition
FIND host WITH NOT state = 'terminated'
FIND user WITH NOT mfa_enabled = true
FIND host WITH NOT region IN ('us-east-1', 'us-west-2')
```

## Multiple Conditions (AND)

Multiple conditions in a single `WITH` clause are combined with AND:

```sql
FIND host WITH state = 'running' AND region = 'us-east-1'
FIND user WITH active = true AND mfa_enabled = false
FIND host WITH env = 'production' AND cpu_count >= 8 AND state = 'running'
```

## OR Conditions

Use `OR` to match any of several values for the same property:

```sql
-- Hosts running linux or windows
FIND host WITH os = 'linux' OR os = 'windows'

-- Three alternatives
FIND host WITH region = 'us-east-1' OR region = 'eu-west-1' OR region = 'ap-southeast-1'
```

Combine OR with AND using the natural precedence — `OR` binds tighter than `AND`:

```sql
-- (os = linux OR os = windows) AND env = prod
FIND host WITH os = 'linux' OR os = 'windows' AND env = 'prod'
```

## GROUP BY

Aggregate query results by a property field:

```sql
-- Count hosts by OS
FIND host GROUP BY os

-- Count hosts by region, filtered to running state
FIND host WITH state = 'running' GROUP BY region
```

`GROUP BY` returns `QueryResult::Grouped` — a list of `(value, count)` pairs,
one per distinct value of the field. Entities missing the field land in their
own group (`Value::Null`).

## Traversal Filters

`WITH` can also appear after a traversal verb to filter the target entity:

```sql
-- Running hosts that run a specific service
FIND host WITH state = 'running' THAT RUNS service WITH name = 'nginx'

-- Users assigned to admin roles (filter both ends)
FIND user WITH active = true THAT ASSIGNED role WITH admin = true
```

## Value Type Reference

| Value Type | Example | Notes |
|---|---|---|
| String | `'running'` | Single quotes. No double quotes. |
| Integer | `42`, `100` | No quotes. |
| Float | `3.14`, `0.5` | Decimal point required. |
| Boolean | `true`, `false` | Lowercase. |
| Null | `null` | Lowercase. |

## Case Sensitivity

- **Property names** are case-sensitive: `state` ≠ `State`
- **String values** are case-sensitive: `'Running'` ≠ `'running'`
- **Keywords** (`WITH`, `AND`, `NOT`, `IN`, `LIKE`, `EXISTS`) are uppercase

## Special Property Names

These properties are always present on every entity:

| Property | Description |
|---|---|
| `display_name` | Human-readable name |
| `_type` | Entity type string (e.g., `"host"`) |
| `_class` | Entity class string (e.g., `"Host"`) |
| `_key` | Source-system key |
| `_deleted` | Soft-delete flag (always `false` in query results) |
