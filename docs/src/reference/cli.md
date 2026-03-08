# CLI Reference

The `parallax` CLI provides commands for serving, querying, and inspecting
the graph engine.

## Installation

```bash
# From source
cargo install --path crates/parallax-cli

# From release binary (when available)
curl -LO https://releases.parallax.rs/v0.1.0/parallax-linux-amd64
chmod +x parallax-linux-amd64
mv parallax-linux-amd64 /usr/local/bin/parallax
```

## Global Flags

```bash
parallax [--log-format FORMAT] <COMMAND>

Global options:
  --log-format <FORMAT>   Log format: "text" (default) or "json"
```

`--log-format json` emits structured JSON logs compatible with log aggregators
(Datadog, Splunk, CloudWatch Logs).

## Commands

### `parallax serve`

Start the REST HTTP server.

```bash
parallax serve [OPTIONS]

Options:
  --host <HOST>       Bind address [default: 127.0.0.1]
  --port <PORT>       Port number [default: 7700]
  --data-dir <PATH>   Data directory [default: ./parallax-data]

Environment:
  PARALLAX_API_KEY    API key for authentication (empty = open mode)
```

**Examples:**

```bash
# Development (open mode)
parallax serve --data-dir ./data

# Production (with auth, all interfaces)
PARALLAX_API_KEY=$(openssl rand -hex 32) \
parallax serve --host 0.0.0.0 --port 7700 --data-dir /var/lib/parallax
```

---

### `parallax query`

Execute a PQL query against a local data directory (in-process, no server).

```bash
parallax query <PQL> [OPTIONS]

Arguments:
  <PQL>               The PQL query string (quote it)

Options:
  --data-dir <PATH>   Data directory [default: ./parallax-data]
  --limit <N>         Limit results [default: 100]
```

**Examples:**

```bash
# Find all running hosts
parallax query "FIND host WITH state = 'running'"

# Count all hosts
parallax query "FIND host RETURN COUNT"

# Group by OS
parallax query "FIND host GROUP BY os"

# With custom data dir
parallax query "FIND host" --data-dir /var/lib/parallax
```

**Example output:**

```
Results: 2
  [host] Web Server 1  (id: a1b2c3d4...)
  [host] Web Server 2  (id: e5f6a7b8...)
```

---

### `parallax stats`

Display entity and relationship counts.

```bash
parallax stats [OPTIONS]

Options:
  --data-dir <PATH>   Data directory [default: ./parallax-data]
  --json              Output as JSON
```

**Example output:**

```
Parallax Graph Statistics
=========================
Total entities:      12,543
Total relationships: 45,210

By type:
  host              1,200
  user              5,400
  service           2,100
  aws_s3_bucket       450
  aws_iam_role        380
  (36 more types...)

By class:
  User              5,400
  Host              1,200
  Service           2,100
  DataStore           450
```

---

### `parallax wal dump`

Inspect the Write-Ahead Log for debugging and forensics.

```bash
parallax wal dump [OPTIONS]

Options:
  --data-dir <PATH>   Data directory [default: ./parallax-data]
  --verbose           Show individual operation details (default: summary only)
```

**Example output (summary):**

```
WAL dump — data_dir: ./parallax-data
       seq        ops  segment
  --------------------------------------------------
         1         50  wal-00000001.pxw
         2        120  wal-00000002.pxw
         3         30  wal-00000003.pxw

  Total: 3 batches, 200 ops
```

**With `--verbose`** (shows each entity/relationship operation):

```
         1         50  wal-00000001.pxw
    + entity  [host] web-01  (id: a1b2c3...)
    + entity  [host] web-02  (id: d4e5f6...)
    + rel     [RUNS] a1b2c3... → 789abc...
    - entity  id=deadbeef...
```

---

### `parallax version`

Print version information.

```bash
parallax version
```

**Output:**

```
parallax 0.1.0
```

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | General error (engine open failure, query error) |
| `2` | Invalid arguments |

## Logging

Set `RUST_LOG` to control log verbosity, and `--log-format` for the output format:

```bash
# Debug all parallax modules (human-readable)
RUST_LOG=parallax=debug parallax serve

# Only warnings and errors
RUST_LOG=warn parallax serve

# JSON structured logs (for log aggregators)
parallax --log-format json serve

# JSON + custom verbosity
RUST_LOG=info parallax --log-format json serve
```
