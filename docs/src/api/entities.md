# Entity & Relationship Endpoints

## GET /v1/entities/:id

Fetch a single entity by its `EntityId`.

### URL Parameters

| Parameter | Description |
|---|---|
| `:id` | Hex-encoded `EntityId` (32 hex characters = 16 bytes) |

### Computing an EntityId

Entity IDs are deterministic from `(account_id, entity_type, entity_key)`.
You can compute them client-side using the same blake3 derivation:

```rust
use parallax_core::entity::EntityId;

// account_id is "default" when using the REST API without multi-tenancy
let id = EntityId::derive("default", "host", "web-01");
let hex = format!("{id}");  // 32 hex characters
```

Or derive the hex string from the list response (`/v1/query`).

### Response (200 OK)

```json
{
  "id": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6",
  "type": "host",
  "class": "Host",
  "display_name": "Web Server 01",
  "properties": {
    "state": "running",
    "region": "us-east-1",
    "cpu_count": 8
  },
  "source": {
    "connector_id": "my-connector",
    "sync_id": "sync-001"
  },
  "created_at": "2024-01-15T10:00:00Z",
  "updated_at": "2024-01-15T10:05:00Z"
}
```

### Response (404 Not Found)

```json
{
  "error": "NotFound",
  "message": "entity a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6 not found"
}
```

### Example

```bash
# Derive entity ID
HOST_ID=$(printf '%s' 'default:host:web-01' | sha256sum | cut -c1-32)

curl http://localhost:7700/v1/entities/a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6
```

---

## GET /v1/relationships/:id

Fetch a single relationship by its `RelationshipId`.

### Response (200 OK)

```json
{
  "id": "d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9",
  "class": "RUNS",
  "from_id": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6",
  "to_id": "b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7",
  "properties": {
    "port": 443
  },
  "source": {
    "connector_id": "my-connector",
    "sync_id": "sync-001"
  }
}
```

### Response (404 Not Found)

```json
{
  "error": "NotFound",
  "message": "relationship d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9 not found"
}
```
