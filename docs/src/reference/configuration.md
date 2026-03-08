# Configuration

## Server Configuration

Configuration via environment variables (recommended) or CLI flags.

| Env Var | CLI Flag | Default | Description |
|---|---|---|---|
| `PARALLAX_HOST` | `--host` | `127.0.0.1` | Bind address |
| `PARALLAX_PORT` | `--port` | `7700` | HTTP port |
| `PARALLAX_DATA_DIR` | `--data-dir` | `./parallax-data` | Data directory |
| `PARALLAX_API_KEY` | — | *(empty)* | API key (empty = open mode) |
| `RUST_LOG` | — | `info` | Log level (trace/debug/info/warn/error) |

## Storage Engine Configuration

`StoreConfig` controls engine behavior. Set via code when embedding,
or via future config file support (v0.2).

```rust
pub struct StoreConfig {
    /// Root directory for WAL, segments, and index files.
    pub data_dir: PathBuf,

    /// Flush MemTable to segment when in-memory size exceeds this threshold.
    /// Default: 64MB. Larger = fewer flushes, more memory usage.
    pub memtable_flush_size: usize,

    /// Maximum WAL segment file size before rotation.
    /// Default: 64MB. After rotation, a new wal-{n}.pxw file starts.
    pub wal_segment_max_size: u64,
}
```

## Data Directory Layout

After starting the server, the data directory has this structure:

```
parallax-data/
├── wal/
│   ├── wal-00000000.pxw    ← WAL segments (binary, PXWA format)
│   ├── wal-00000001.pxw
│   └── ...
└── segments/
    ├── seg-00000000.pxs    ← Immutable segment files (binary, PXSG format)
    ├── seg-00000001.pxs
    └── ...
```

Both WAL and segment files are binary and not human-readable. Use
`parallax stats` to inspect the current state.

## Query Limits

Default limits applied to all queries:

| Limit | Default | Description |
|---|---|---|
| `max_results` | 10,000 | Maximum entities returned per query |
| `timeout` | 30s | Query execution timeout |
| `max_traversal_depth` | 10 | Maximum traversal hops |

These are currently hardcoded. Per-query overrides and global configuration
are planned for v0.2.

## Logging

Parallax uses `tracing` for structured logging. Configure with `RUST_LOG`:

```bash
# Common patterns
RUST_LOG=info parallax serve           # Default
RUST_LOG=debug parallax serve          # Verbose
RUST_LOG=parallax_store=trace parallax serve  # Trace storage only
RUST_LOG=parallax=debug,tower_http=warn parallax serve  # App debug, HTTP quiet
```

Log format (JSON) is planned for v0.2. Current format is human-readable text.

## Production Recommendations

```bash
# Full production configuration
export PARALLAX_HOST=0.0.0.0
export PARALLAX_PORT=7700
export PARALLAX_DATA_DIR=/var/lib/parallax
export PARALLAX_API_KEY=$(openssl rand -hex 32)
export RUST_LOG=info

parallax serve
```

**Storage:**
- Mount `PARALLAX_DATA_DIR` on a fast SSD
- Ensure at least 10GB free space for WAL and segments
- Set up log rotation for WAL files (automatic in v0.2)

**Security:**
- Generate a cryptographically random API key
- Terminate TLS at the load balancer or reverse proxy
- Restrict network access to the parallax port
- Never log the `PARALLAX_API_KEY`

**Monitoring:**
- Scrape `/metrics` with Prometheus
- Alert on `parallax_errors_total` increases
- Monitor `parallax_entities_total` for unexpected spikes or drops
