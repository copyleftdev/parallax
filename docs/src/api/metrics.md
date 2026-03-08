# Prometheus Metrics

## GET /metrics

Returns engine metrics in Prometheus text exposition format.
Use this endpoint to integrate with Prometheus, Grafana, or any compatible
metrics system.

### Response (200 OK)

```
# HELP parallax_entities_total Total number of entities in the graph
# TYPE parallax_entities_total gauge
parallax_entities_total 12543

# HELP parallax_relationships_total Total number of relationships in the graph
# TYPE parallax_relationships_total gauge
parallax_relationships_total 45210

# HELP parallax_writes_total Total number of write batches committed
# TYPE parallax_writes_total counter
parallax_writes_total 1024

# HELP parallax_reads_total Total number of snapshot reads
# TYPE parallax_reads_total counter
parallax_reads_total 98432

# HELP parallax_uptime_seconds Server uptime in seconds
# TYPE parallax_uptime_seconds gauge
parallax_uptime_seconds 3601
```

### Content Type

```
Content-Type: text/plain; version=0.0.4; charset=utf-8
```

### Prometheus Scrape Configuration

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'parallax'
    static_configs:
      - targets: ['localhost:7700']
    metrics_path: '/metrics'
    # Add auth if PARALLAX_API_KEY is set:
    authorization:
      credentials: 'your-api-key'
```

### Available Metrics

| Metric | Type | Description |
|---|---|---|
| `parallax_entities_total` | Gauge | Current entity count (includes soft-deleted) |
| `parallax_relationships_total` | Gauge | Current relationship count |
| `parallax_writes_total` | Counter | Total committed write batches |
| `parallax_reads_total` | Counter | Total snapshot reads |
| `parallax_uptime_seconds` | Gauge | Server uptime in seconds |

### Future Metrics (v0.2)

Planned additions:
- `parallax_query_duration_seconds` — histogram of query latencies
- `parallax_wal_bytes_total` — WAL bytes written
- `parallax_segment_count` — number of on-disk segments
- `parallax_sync_duration_seconds` — histogram of sync durations per connector
- `parallax_sync_errors_total` — count of sync errors by connector

### Example

```bash
curl http://localhost:7700/metrics
```

Or with authentication:

```bash
curl -H 'Authorization: Bearer your-key' http://localhost:7700/metrics
```
