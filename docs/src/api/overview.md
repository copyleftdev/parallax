# REST API Overview

`parallax-server` exposes a REST HTTP API over Axum. All responses are JSON.

## Base URL

```
http://localhost:7700   (default)
```

Configure with `--host` and `--port` flags or the `PARALLAX_HOST` /
`PARALLAX_PORT` environment variables.

## Versioning

All endpoints are prefixed with `/v1/`. The version is part of the URL path,
not a header.

## Endpoint Summary

| Method | Path | Description |
|---|---|---|
| `GET` | `/v1/health` | Health check (auth-exempt) |
| `GET` | `/v1/stats` | Entity and relationship counts |
| `POST` | `/v1/query` | Execute a PQL query |
| `GET` | `/v1/entities/:id` | Fetch an entity by ID |
| `GET` | `/v1/relationships/:id` | Fetch a relationship by ID |
| `POST` | `/v1/ingest/sync` | Connector sync batch |
| `POST` | `/v1/ingest/write` | Direct write batch |
| `GET` | `/v1/connectors` | List registered connectors |
| `POST` | `/v1/connectors/:id/sync` | Trigger a connector sync |
| `GET` | `/metrics` | Prometheus metrics exposition |

## Content Type

All request bodies must be `Content-Type: application/json`.
All responses have `Content-Type: application/json` (except `/metrics`).

## Request IDs

Every request receives a `X-Request-Id` header in the response (INV-A05).
The server either generates a UUID v4 or propagates the value from the
incoming request's `X-Request-Id` header. Use this for log correlation.

```bash
curl -v http://localhost:7700/v1/health
# < X-Request-Id: 550e8400-e29b-41d4-a716-446655440000
```

## Common Response Codes

| Status | Meaning |
|---|---|
| `200 OK` | Success |
| `400 Bad Request` | Invalid request body or parameters |
| `401 Unauthorized` | Missing or invalid API key |
| `404 Not Found` | Entity/relationship not found |
| `500 Internal Server Error` | Storage or processing error |

## Starting the Server

```bash
# No auth (development)
parallax serve --data-dir ./data

# With API key
PARALLAX_API_KEY=my-secret-key parallax serve --data-dir ./data

# Custom host and port
parallax serve --host 0.0.0.0 --port 8080 --data-dir /var/lib/parallax
```
