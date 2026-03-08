# Query Endpoints

## POST /v1/query

Execute a PQL query and return results.

### Request Body

```json
{
  "pql": "FIND host WITH state = 'running'"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `pql` | string | Yes | The PQL query to execute |

### Response: Entity Query (200 OK)

```json
{
  "count": 2,
  "entities": [
    {
      "id": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6",
      "entity_type": "host",
      "entity_class": "Host",
      "display_name": "Web Server 01",
      "properties": {
        "state": "running",
        "region": "us-east-1"
      },
      "source": {
        "connector_id": "my-connector",
        "sync_id": "sync-001"
      }
    },
    {
      "id": "b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7",
      "entity_type": "host",
      "entity_class": "Host",
      "display_name": "Database Server 01",
      "properties": {
        "state": "running",
        "region": "us-west-2"
      }
    }
  ]
}
```

### Response: Count Query (200 OK)

For `FIND ... RETURN COUNT`:

```json
{
  "count": 42
}
```

### Response: Traversal Query (200 OK)

For `FIND A THAT VERB B`:

```json
{
  "count": 1,
  "entities": [
    {
      "id": "...",
      "entity_type": "host",
      "entity_class": "Host",
      "display_name": "Web Server 01"
    }
  ]
}
```

### Response: Path Query (200 OK)

For `FIND SHORTEST PATH FROM A TO B`:

```json
{
  "count": 1,
  "path": [
    {"entity_id": "user-alice-id"},
    {"entity_id": "role-admin-id", "via_relationship": "rel-assigned-id"},
    {"entity_id": "secret-prod-id", "via_relationship": "rel-allows-id"}
  ]
}
```

If no path exists:
```json
{
  "count": 0,
  "path": null
}
```

### Error: Parse Error (400)

```json
{
  "error": "ParseError",
  "message": "unexpected token '=' at position 18: expected comparison operator"
}
```

### Error: Execution Error (500)

```json
{
  "error": "ExecutionError",
  "message": "query timed out after 30000ms"
}
```

### Examples

```bash
# Find all running hosts
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND host WITH state = '\''running'\''"}'

# Count all services
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND Service RETURN COUNT"}'

# Find hosts with no EDR
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND host THAT !PROTECTS edr_agent"}'

# Blast radius
curl -X POST http://localhost:7700/v1/query \
  -H 'Content-Type: application/json' \
  -d '{"pql": "FIND BLAST RADIUS FROM host WITH _key = '\''web-01'\'' DEPTH 4"}'
```
