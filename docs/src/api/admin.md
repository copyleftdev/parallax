# Admin Endpoints

## GET /v1/health

Health check endpoint. Always returns 200 OK if the server is running.
This endpoint is **exempt from authentication** (INV-A06).

### Response (200 OK)

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

Use this for load balancer health checks and readiness probes.

---

## GET /v1/stats

Engine statistics including entity and relationship counts.

### Response (200 OK)

```json
{
  "total_entities": 12543,
  "total_relationships": 45210,
  "version": "0.1.0",
  "uptime_seconds": 3600,
  "type_counts": {
    "host": 1200,
    "user": 5400,
    "service": 2100,
    "aws_s3_bucket": 450,
    "aws_iam_role": 380
  },
  "class_counts": {
    "Host": 1200,
    "User": 5400,
    "Service": 2100,
    "DataStore": 450
  }
}
```

### Example

```bash
curl http://localhost:7700/v1/stats
```

---

## GET /v1/connectors

List registered connectors.

In v0.1, this returns the list of connector IDs that have submitted at least
one sync batch to this server instance.

### Response (200 OK)

```json
{
  "connectors": [
    {
      "id": "aws-connector",
      "last_sync_id": "sync-2024-01-15-003",
      "entity_count": 8421
    },
    {
      "id": "okta-connector",
      "last_sync_id": "sync-2024-01-15-001",
      "entity_count": 4122
    }
  ]
}
```

---

## POST /v1/connectors/:id/sync

Trigger a sync for a registered connector.

In v0.1, this is a stub endpoint that returns 501 Not Implemented. Full
server-side connector scheduling is planned for v0.2.

### Response (501 Not Implemented)

```json
{
  "error": "NotImplemented",
  "message": "server-side connector scheduling is planned for v0.2"
}
```

For now, run connectors out-of-process and use `POST /v1/ingest/sync` to
submit the results.

---

## GET /v1/policies

List the currently loaded policy rules.

### Response (200 OK)

```json
{
  "count": 2,
  "rules": [
    {
      "id": "edr-coverage-001",
      "name": "EDR coverage gap",
      "severity": "high",
      "description": "Hosts without EDR protection.",
      "query": "FIND host THAT !PROTECTS edr_agent",
      "enabled": true,
      "frameworks": [
        { "framework": "CIS-Controls-v8", "control": "10.1" }
      ]
    }
  ]
}
```

---

## POST /v1/policies

Replace the loaded rule set. PQL in each enabled rule is validated at load
time (INV-P06) — invalid PQL returns 400.

### Request body

```json
{
  "rules": [
    {
      "id": "edr-coverage-001",
      "name": "EDR coverage gap",
      "severity": "high",
      "description": "Hosts without EDR protection.",
      "query": "FIND host THAT !PROTECTS edr_agent",
      "frameworks": [{ "framework": "CIS-Controls-v8", "control": "10.1" }],
      "schedule": "manual",
      "remediation": "Deploy EDR agent.",
      "enabled": true
    }
  ]
}
```

### Response (200 OK)

```json
{ "loaded": 1 }
```

### Response (400 Bad Request) — invalid PQL

```json
{ "error": "policy validation failed: Rule 'edr-coverage-001' contains invalid PQL: ..." }
```

---

## POST /v1/policies/evaluate

Evaluate all enabled rules against the current graph snapshot.

### Response (200 OK)

```json
{
  "total": 2,
  "pass": 1,
  "fail": 1,
  "results": [
    {
      "rule_id": "edr-coverage-001",
      "status": "Fail",
      "violation_count": 3,
      "error": null
    },
    {
      "rule_id": "mfa-all-users",
      "status": "Pass",
      "violation_count": 0,
      "error": null
    }
  ]
}
```

Status values: `"Pass"`, `"Fail"`, `"Error"`, `"Skipped"`.

---

## GET /v1/policies/posture

Compute compliance posture for a framework.

### Query parameters

| Parameter | Default | Description |
|---|---|---|
| `framework` | `CIS-Controls-v8` | Framework name to report on |

### Response (200 OK)

```json
{
  "framework": "CIS-Controls-v8",
  "overall_score": 0.75,
  "controls": [
    {
      "control_id": "10.1",
      "status": "Fail",
      "rule_count": 1,
      "pass_count": 0,
      "fail_count": 1
    },
    {
      "control_id": "6.5",
      "status": "Pass",
      "rule_count": 1,
      "pass_count": 1,
      "fail_count": 0
    }
  ]
}
```

`overall_score` is the fraction of controls with status `Pass` (0.0–1.0).
Controls with no mapped rules are not included.
