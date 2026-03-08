# Error Responses

All API errors follow a consistent JSON structure.

## Error Response Format

```json
{
  "error": "ErrorType",
  "message": "Human-readable description of what went wrong"
}
```

The `X-Request-Id` header is always present in error responses, making it
easy to correlate errors with server logs.

## Error Types

### 400 Bad Request

| Error | Cause |
|---|---|
| `ParseError` | PQL query has a syntax error |
| `InvalidRequest` | Request body is missing required fields or has invalid types |
| `InvalidEntityClass` | Entity class is not in the known classes list |
| `InvalidRelationshipVerb` | Relationship verb is not in the known verbs list |

```json
{
  "error": "ParseError",
  "message": "unexpected token 'WHERE' at position 12: use 'WITH' instead"
}
```

### 401 Unauthorized

```json
{
  "error": "Unauthorized",
  "message": "missing or invalid API key"
}
```

### 404 Not Found

```json
{
  "error": "NotFound",
  "message": "entity a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6 not found"
}
```

### 500 Internal Server Error

| Error | Cause |
|---|---|
| `StoreError` | Storage engine error (I/O failure, corruption) |
| `DanglingRelationship` | Ingest batch contains a relationship referencing a non-existent entity |
| `ExecutionError` | Query execution failed (timeout, resource limit) |
| `SyncError` | Sync commit failed |

```json
{
  "error": "DanglingRelationship",
  "message": "relationship from host:web-01 RUNS service:ghost — entity not found in batch or graph"
}
```

```json
{
  "error": "StoreError",
  "message": "I/O error writing to WAL: No space left on device"
}
```

## Common Mistakes

### "WHERE" instead of "WITH"

PQL uses `WITH` for property filters, not `WHERE`:

```
# Wrong
FIND host WHERE state = 'running'

# Correct
FIND host WITH state = 'running'
```

### Double quotes in string literals

PQL uses single quotes, not double quotes:

```
# Wrong
FIND host WITH state = "running"

# Correct
FIND host WITH state = 'running'
```

### Dangling relationship in ingest

The `from_key` and `to_key` in relationships must reference entities that
exist in the same batch or the current graph:

```json
{
  "entities": [
    {"entity_type": "host", "entity_key": "web-01", "entity_class": "Host"}
  ],
  "relationships": [
    {
      "from_type": "host", "from_key": "web-01",
      "verb": "RUNS",
      "to_type": "service", "to_key": "ghost-service"
    }
  ]
}
```

This returns a `DanglingRelationship` error because `service:ghost-service`
doesn't exist. Either include the service entity in the batch or remove the
relationship.

## Logging

Server-side errors are logged with the request ID and full error context:

```
ERROR parallax_server::routes: query execution failed
  request_id = "550e8400-e29b-41d4-a716-446655440000"
  error = "ParseError: unexpected token '=' at position 18"
  pql = "FIND host WITH state = = 'running'"
```

Use the `X-Request-Id` from the response to search server logs.
