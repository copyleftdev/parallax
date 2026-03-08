# Ingest Endpoints

## POST /v1/ingest/sync

Submit a connector sync batch. This is the primary ingest endpoint.

The server runs the sync protocol: validates referential integrity,
diffs against the current graph state for this connector, and commits
the delta atomically (INV-C01).

### Request Body

```json
{
  "connector_id": "my-connector",
  "sync_id": "sync-2024-01-15-001",
  "entities": [
    {
      "entity_type": "host",
      "entity_key": "web-01",
      "entity_class": "Host",
      "display_name": "Web Server 01",
      "properties": {
        "state": "running",
        "region": "us-east-1",
        "cpu_count": 8
      }
    },
    {
      "entity_type": "service",
      "entity_key": "nginx-web-01",
      "entity_class": "Service",
      "display_name": "Nginx on web-01"
    }
  ],
  "relationships": [
    {
      "from_type": "host",
      "from_key": "web-01",
      "verb": "RUNS",
      "to_type": "service",
      "to_key": "nginx-web-01"
    }
  ]
}
```

### Request Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `connector_id` | string | Yes | Identifies the connector. Used for source-scoped diff. |
| `sync_id` | string | Yes | Unique ID for this sync run. Included in source tracking. |
| `entities` | array | Yes | Entities to upsert. Can be empty. |
| `relationships` | array | Yes | Relationships to upsert. Can be empty. |

### Entity Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `entity_type` | string | Yes | Type identifier (snake_case, e.g., `"host"`) |
| `entity_key` | string | Yes | Source-system unique key |
| `entity_class` | string | Yes | Class from the [known classes](../reference/entity-classes.md) list |
| `display_name` | string | No | Human-readable name |
| `properties` | object | No | Flat key-value property bag |

### Relationship Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `from_type` | string | Yes | Source entity type |
| `from_key` | string | Yes | Source entity key |
| `verb` | string | Yes | Relationship verb from the [known verbs](../reference/relationship-verbs.md) list |
| `to_type` | string | Yes | Target entity type |
| `to_key` | string | Yes | Target entity key |
| `properties` | object | No | Flat key-value property bag on the edge |

### Response (200 OK)

```json
{
  "sync_id": "sync-2024-01-15-001",
  "entities_created": 2,
  "entities_updated": 0,
  "entities_unchanged": 0,
  "entities_deleted": 0,
  "relationships_created": 1,
  "relationships_updated": 0,
  "relationships_unchanged": 0,
  "relationships_deleted": 0
}
```

### Error: Dangling Relationship (500)

If a relationship references an entity that doesn't exist in the batch or
the current graph:

```json
{
  "error": "DanglingRelationship",
  "message": "relationship from host:web-01 RUNS service:ghost — to_id not found in batch or graph"
}
```

### Example

```bash
curl -X POST http://localhost:7700/v1/ingest/sync \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer your-key' \
  -d '{
    "connector_id": "my-connector",
    "sync_id": "sync-001",
    "entities": [
      {"entity_type": "host", "entity_key": "web-01", "entity_class": "Host",
       "display_name": "Web Server 01", "properties": {"state": "running"}}
    ],
    "relationships": []
  }'
```

---

## POST /v1/ingest/write

Direct write batch — bypass the sync protocol. Use this when you want to
write entities without source-scoped diffing.

Unlike `/v1/ingest/sync`, this endpoint does not:
- Diff against previous state from the same connector
- Delete entities not present in the batch

It simply upserts all provided entities and relationships.

### Request Body

```json
{
  "write_id": "write-001",
  "entities": [
    {
      "entity_type": "host",
      "entity_key": "h1",
      "entity_class": "Host",
      "display_name": "Server 1"
    }
  ],
  "relationships": []
}
```

### Response (200 OK)

```json
{
  "write_id": "write-001",
  "entities_written": 1,
  "relationships_written": 0
}
```

### When to Use Write vs. Sync

| Use Case | Endpoint |
|---|---|
| Connector that owns its entities | `/v1/ingest/sync` |
| One-time bulk import | `/v1/ingest/write` |
| Incremental updates (no deletions) | `/v1/ingest/write` |
| Full re-sync with deletion of departed entities | `/v1/ingest/sync` |
