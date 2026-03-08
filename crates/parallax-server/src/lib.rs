//! # parallax-server
//!
//! REST server for the Parallax graph engine.
//!
//! **Spec reference:** `specs/06-api-surface.md`

pub mod middleware;
pub mod routes;
pub mod state;

pub use state::AppState;

use anyhow::Result;
use axum::{middleware as axum_middleware, Router};
use axum::routing::{get, post};
use parallax_store::{StoreConfig, StorageEngine};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

/// Build the Axum router wired to `state`.
///
/// Layers applied (outer → inner):
/// 1. `TraceLayer` — HTTP access logging
/// 2. `request_id_middleware` — injects X-Request-Id (INV-A05)
/// 3. `auth_middleware` — bearer-token API key check (INV-A01)
pub fn router(state: AppState) -> Router {
    let api_key = Arc::clone(&state.api_key);

    Router::new()
        .route("/v1/health", get(routes::health))
        .route("/v1/stats", get(routes::stats))
        .route("/v1/query", post(routes::query))
        .route("/v1/entities/:id", get(routes::get_entity))
        .route("/v1/relationships/:id", get(routes::get_relationship))
        .route("/v1/ingest/sync", post(routes::ingest_sync))
        .route("/v1/ingest/write", post(routes::ingest_write))
        .route("/v1/connectors", get(routes::list_connectors))
        .route("/v1/connectors/:id/sync", post(routes::trigger_connector_sync))
        .route("/v1/policies", get(routes::list_policies).post(routes::set_policies))
        .route("/v1/policies/evaluate", post(routes::evaluate_policies))
        .route("/v1/policies/posture", get(routes::policy_posture))
        .route("/metrics", get(routes::prometheus_metrics))
        .layer(axum_middleware::from_fn(middleware::request_id_middleware))
        .layer(axum_middleware::from_fn(move |req, next| {
            middleware::auth_middleware(Arc::clone(&api_key), req, next)
        }))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Open a storage engine at `data_dir` and start serving on `host:port`.
///
/// Reads `PARALLAX_API_KEY` from the environment. If the variable is unset
/// or empty, the server starts in open/dev mode (no auth enforcement).
///
/// Blocks until the server shuts down (Ctrl-C / SIGTERM).
pub async fn serve(host: &str, port: u16, data_dir: &str) -> Result<()> {
    let api_key = std::env::var("PARALLAX_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        tracing::warn!("PARALLAX_API_KEY is not set — server running in open/dev mode (INV-A01 not enforced)");
    }

    let config = StoreConfig::new(data_dir);
    let engine = StorageEngine::open(config)?;
    let state = AppState::with_key(engine, api_key);
    let app = router(state);

    let addr = format!("{host}:{port}");
    tracing::info!("parallax-server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
