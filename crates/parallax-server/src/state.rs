//! Server application state — shared across all request handlers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use parallax_ingest::sync::SyncEngine;
use parallax_policy::PolicyRule;
use parallax_query::IndexStats;
use parallax_store::StorageEngine;

/// Shared application state wrapped in `Arc` for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    /// The storage engine. Protected by a Mutex for single-writer safety.
    pub engine: Arc<Mutex<StorageEngine>>,
    /// Sync engine for connector ingest.
    pub sync: SyncEngine,
    /// Index statistics, recomputed after each write.
    pub stats: Arc<Mutex<IndexStats>>,
    /// Loaded policy rules. Updated via `POST /v1/policies`.
    pub policies: Arc<Mutex<Vec<PolicyRule>>>,
    /// Server version string.
    pub version: &'static str,
    /// Server start time (for uptime reporting).
    pub started_at: std::time::Instant,
    /// Expected API key (bearer token). Empty string = open/dev mode.
    /// INV-A02: never log or expose this field.
    pub api_key: Arc<String>,
}

impl AppState {
    pub fn new(engine: StorageEngine) -> Self {
        Self::with_key(engine, String::new())
    }

    pub fn with_key(engine: StorageEngine, api_key: String) -> Self {
        let engine_arc = Arc::new(Mutex::new(engine));
        let stats = {
            let e = engine_arc.lock().expect("engine lock");
            build_stats(&e)
        };
        let sync = SyncEngine::new(Arc::clone(&engine_arc));
        AppState {
            engine: engine_arc,
            sync,
            stats: Arc::new(Mutex::new(stats)),
            policies: Arc::new(Mutex::new(Vec::new())),
            version: env!("CARGO_PKG_VERSION"),
            started_at: std::time::Instant::now(),
            api_key: Arc::new(api_key),
        }
    }

    /// Recompute IndexStats from the current snapshot.
    pub fn refresh_stats(&self) {
        let engine = self.engine.lock().expect("engine lock");
        let new_stats = build_stats(&engine);
        drop(engine);
        *self.stats.lock().expect("stats lock") = new_stats;
    }

    pub fn current_stats(&self) -> IndexStats {
        self.stats.lock().expect("stats lock").clone()
    }
}

fn build_stats(engine: &StorageEngine) -> IndexStats {
    let snap = engine.snapshot();
    let all = snap.all_entities();

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut class_counts: HashMap<String, usize> = HashMap::new();

    for e in &all {
        *type_counts.entry(e._type.as_str().to_owned()).or_insert(0) += 1;
        *class_counts
            .entry(e._class.as_str().to_owned())
            .or_insert(0) += 1;
    }

    IndexStats::new(
        type_counts,
        class_counts,
        snap.entity_count(),
        snap.relationship_count(),
    )
}
