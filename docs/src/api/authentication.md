# Authentication

Parallax uses API key authentication. When no API key is configured, the
server runs in open mode (all requests accepted).

## Configuration

Set the `PARALLAX_API_KEY` environment variable before starting the server:

```bash
export PARALLAX_API_KEY="your-secret-api-key"
parallax serve --data-dir ./data
```

If `PARALLAX_API_KEY` is not set or is empty, the server starts in open mode
and accepts all requests without authentication.

## Making Authenticated Requests

Include the API key as a Bearer token in the `Authorization` header:

```bash
curl -H "Authorization: Bearer your-secret-api-key" \
     http://localhost:7700/v1/stats
```

Or with an explicit header:

```bash
curl -H "Authorization: Bearer your-secret-api-key" \
     -X POST http://localhost:7700/v1/query \
     -H "Content-Type: application/json" \
     -d '{"pql": "FIND host"}'
```

## Exemptions

The `/v1/health` endpoint is always exempt from authentication (INV-A06).
This allows load balancers and health check systems to verify server liveness
without credentials.

```bash
# Always works, even with auth enabled
curl http://localhost:7700/v1/health
```

The `/metrics` Prometheus endpoint is **not** exempt — protect it from
unauthenticated access in production.

## Security Properties

**Constant-time comparison (INV-A02):** API key verification uses constant-time
string comparison to prevent timing attacks. An attacker cannot determine the
correct key by measuring response time differences:

```rust
fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
```

**Keys are never logged:** The API key is stored in memory as `Arc<String>`.
It is never written to log output, error messages, or response bodies.

## 401 Response

When authentication fails:

```json
{
  "error": "Unauthorized",
  "message": "missing or invalid API key"
}
```

HTTP status: `401 Unauthorized`

## Best Practices

1. Use a random, high-entropy key: `openssl rand -hex 32`
2. Rotate keys regularly in production
3. Use TLS in production (TLS termination planned for v0.2)
4. Restrict network access to the server — treat the API key as a secondary
   defense, not the primary

## Open Mode vs. Protected Mode

| Mode | Config | Use Case |
|---|---|---|
| Open | `PARALLAX_API_KEY` not set | Local development, CLI usage, testing |
| Protected | `PARALLAX_API_KEY=<key>` | Production, shared deployments |

In open mode, all endpoints are accessible without credentials. This is
suitable for local development and single-user CLI usage where network
access is already restricted.
