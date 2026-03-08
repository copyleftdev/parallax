//! parallax — CLI for the Parallax graph engine.
//!
//! **Spec reference:** `specs/06-api-surface.md` §6.1 (CLI)

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "parallax",
    version = env!("CARGO_PKG_VERSION"),
    about = "Rust-native graph engine for cyber asset intelligence",
    long_about = None,
)]
struct Cli {
    /// Log format: "text" (default) or "json" (structured JSON for log aggregators).
    #[arg(long, global = true, default_value = "text", value_name = "FORMAT")]
    log_format: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the Parallax HTTP server.
    Serve {
        /// Host to listen on.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on.
        #[arg(short, long, default_value_t = 7700)]
        port: u16,

        /// Data directory for WAL and segments.
        #[arg(short, long, default_value = "parallax-data")]
        data_dir: String,
    },

    /// Execute a PQL query against a local data directory (in-process).
    Query {
        /// The PQL query string.
        pql: String,

        /// Data directory to read from.
        #[arg(short, long, default_value = "parallax-data")]
        data_dir: String,

        /// Maximum number of results to return.
        #[arg(short, long, default_value_t = 100)]
        limit: usize,
    },

    /// Print graph statistics from a local data directory (in-process).
    Stats {
        /// Data directory to read from.
        #[arg(short, long, default_value = "parallax-data")]
        data_dir: String,
    },

    /// Inspect Write-Ahead Log contents (debugging).
    Wal {
        #[command(subcommand)]
        wal_cmd: WalCommand,
    },

    /// Print the Parallax version.
    Version,
}

#[derive(Subcommand)]
enum WalCommand {
    /// Dump all WAL entries to stdout.
    Dump {
        /// Data directory to read from.
        #[arg(short, long, default_value = "parallax-data")]
        data_dir: String,

        /// Show individual operation details (default: summary only).
        #[arg(long)]
        verbose: bool,
    },
}

// ─── Entrypoint ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 4A: JSON or text log format.
    match cli.log_format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(EnvFilter::from_default_env())
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::from_default_env())
                .init();
        }
    }

    match cli.command {
        Command::Serve {
            host,
            port,
            data_dir,
        } => {
            cmd_serve(&host, port, &data_dir).await?;
        }
        Command::Query {
            pql,
            data_dir,
            limit,
        } => {
            cmd_query(&pql, &data_dir, limit)?;
        }
        Command::Stats { data_dir } => {
            cmd_stats(&data_dir)?;
        }
        Command::Wal { wal_cmd } => match wal_cmd {
            WalCommand::Dump { data_dir, verbose } => {
                cmd_wal_dump(&data_dir, verbose)?;
            }
        },
        Command::Version => {
            println!("parallax {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}

// ─── Command implementations ──────────────────────────────────────────────────

async fn cmd_serve(host: &str, port: u16, data_dir: &str) -> Result<()> {
    parallax_server::serve(host, port, data_dir).await
}

fn cmd_query(pql: &str, data_dir: &str, limit: usize) -> Result<()> {
    use parallax_graph::GraphReader;
    use parallax_query::{execute, parse, plan, IndexStats, QueryLimits, QueryResult};
    use parallax_store::{StorageEngine, StoreConfig};
    use std::collections::HashMap;

    let config = StoreConfig::new(data_dir);
    let engine = StorageEngine::open(config)?;
    let snap = engine.snapshot();

    // Build IndexStats for planner.
    let all = snap.all_entities();
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut class_counts: HashMap<String, usize> = HashMap::new();
    for e in &all {
        *type_counts.entry(e._type.as_str().to_owned()).or_insert(0) += 1;
        *class_counts
            .entry(e._class.as_str().to_owned())
            .or_insert(0) += 1;
    }
    let stats = IndexStats::new(
        type_counts,
        class_counts,
        snap.entity_count(),
        snap.relationship_count(),
    );

    let ast = parse(pql).map_err(|e| anyhow::anyhow!("Parse error: {e}"))?;
    let query_plan = plan(ast, &stats).map_err(|e| anyhow::anyhow!("Plan error: {e}"))?;

    let graph = GraphReader::new(&snap);
    let limits = QueryLimits {
        max_results: limit,
        ..QueryLimits::default()
    };
    let result =
        execute(&query_plan, &graph, limits).map_err(|e| anyhow::anyhow!("Exec error: {e}"))?;

    let count = result.count();
    println!("Results: {count}");

    match result {
        QueryResult::Entities(ents) => {
            for e in &ents {
                println!("  [{type}] {name}  (id: {id})",
                    type = e._type.as_str(),
                    name = e.display_name.as_str(),
                    id = e.id,
                );
            }
        }
        QueryResult::Traversals(ts) => {
            for t in &ts {
                println!("  depth={d}  [{type}] {name}  (id: {id})",
                    d = t.depth,
                    type = t.entity._type.as_str(),
                    name = t.entity.display_name.as_str(),
                    id = t.entity.id,
                );
            }
        }
        QueryResult::Scalar(n) => {
            println!("  count = {n}");
        }
        QueryResult::Paths(paths) => {
            for (i, path) in paths.iter().enumerate() {
                println!("  path {}: {} hops", i + 1, path.segments.len());
            }
        }
        QueryResult::Grouped(groups) => {
            for (val, count) in &groups {
                println!("  {val:?}  →  {count}");
            }
        }
    }

    Ok(())
}

fn cmd_stats(data_dir: &str) -> Result<()> {
    use parallax_store::{StorageEngine, StoreConfig};

    let config = StoreConfig::new(data_dir);
    let engine = StorageEngine::open(config)?;
    let snap = engine.snapshot();

    let all = snap.all_entities();
    let mut type_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut class_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for e in &all {
        *type_counts.entry(e._type.as_str().to_owned()).or_insert(0) += 1;
        *class_counts
            .entry(e._class.as_str().to_owned())
            .or_insert(0) += 1;
    }

    println!("Parallax Graph Statistics");
    println!("  Data dir:           {data_dir}");
    println!("  Total entities:     {}", snap.entity_count());
    println!("  Total relationships:{}", snap.relationship_count());
    println!("  Entity types:       {}", type_counts.len());
    println!("  Entity classes:     {}", class_counts.len());

    if !type_counts.is_empty() {
        println!();
        println!("Entity types:");
        let mut types: Vec<_> = type_counts.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1));
        for (t, n) in types {
            println!("  {t:<40} {n}");
        }
    }

    Ok(())
}

/// 4B: Dump WAL entries for debugging.
fn cmd_wal_dump(data_dir: &str, verbose: bool) -> Result<()> {
    use parallax_store::{dump_wal, WriteOp};
    use std::path::Path;

    let entries =
        dump_wal(Path::new(data_dir)).map_err(|e| anyhow::anyhow!("WAL read error: {e}"))?;

    if entries.is_empty() {
        println!("WAL is empty (data_dir: {data_dir})");
        return Ok(());
    }

    println!("WAL dump — data_dir: {data_dir}");
    println!("  {:>8}  {:>10}  segment", "seq", "ops");
    println!("  {}", "-".repeat(50));

    for entry in &entries {
        let ops = entry.batch.len();
        println!("  {:>8}  {:>10}  {}", entry.seq, ops, entry.segment);

        if verbose {
            for op in &entry.batch.operations {
                match op {
                    WriteOp::UpsertEntity(e) => {
                        println!("    + entity  [{type}] {name}  (id: {id})",
                            type = e._type.as_str(),
                            name = e.display_name.as_str(),
                            id = e.id,
                        );
                    }
                    WriteOp::DeleteEntity(id) => {
                        println!("    - entity  id={id}");
                    }
                    WriteOp::UpsertRelationship(r) => {
                        println!(
                            "    + rel     [{cls}] {from} → {to}",
                            cls = r._class.as_str(),
                            from = r.from_id,
                            to = r.to_id,
                        );
                    }
                    WriteOp::DeleteRelationship(id) => {
                        let hex = id.0.iter().map(|b| format!("{b:02x}")).collect::<String>();
                        println!("    - rel     id={hex}");
                    }
                }
            }
        }
    }

    let total_ops: usize = entries.iter().map(|e| e.batch.len()).sum();
    println!();
    println!("  Total: {} batches, {} ops", entries.len(), total_ops);

    Ok(())
}
