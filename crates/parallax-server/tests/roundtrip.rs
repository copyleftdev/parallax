//! End-to-end integration test: ingest → store → graph → query → REST API.
//!
//! This is the "done done" test — it exercises every crate in the dependency
//! chain in a single scenario: push data via the REST API, then query it back.
//!
//! Stack exercised:
//!   parallax-server (REST) → parallax-ingest (validate + sync) →
//!   parallax-store (WAL + MemTable) → parallax-graph (traversal) →
//!   parallax-query (PQL parse/plan/execute)

use axum::body::Body;
use axum::http::{Request, StatusCode};
use parallax_server::{router, AppState};
use parallax_store::{StoreConfig, StorageEngine};
use tempfile::TempDir;
use tower::ServiceExt as _;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open_app() -> (axum::Router, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine");
    let state = AppState::new(engine);
    (router(state), dir)
}

async fn post(app: axum::Router, path: &str, body: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get_json(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder().method("GET").uri(path).body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ─── tests ───────────────────────────────────────────────────────────────────

/// Push two hosts via ingest/sync, then query them back via PQL.
#[tokio::test]
async fn ingest_then_pql_query_roundtrip() {
    use tower::Service;

    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::open(StoreConfig::new(dir.path())).unwrap();
    let state = AppState::new(engine);
    let mut app = router(state);

    // Ingest two hosts via POST /v1/ingest/sync.
    let sync_req = Request::builder()
        .method("POST").uri("/v1/ingest/sync")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{
            "connector_id": "test-connector",
            "sync_id": "sync-001",
            "entities": [
                {"entity_type": "host", "entity_key": "h1", "entity_class": "Host",
                 "display_name": "Server 1", "properties": {"state": "running"}},
                {"entity_type": "host", "entity_key": "h2", "entity_class": "Host",
                 "display_name": "Server 2", "properties": {"state": "stopped"}}
            ],
            "relationships": []
        }"#))
        .unwrap();

    let resp = app.call(sync_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["entities_created"], 2, "ingest failed: {body}");

    // Query via PQL on the same app (same engine).
    let query_req = Request::builder()
        .method("POST").uri("/v1/query")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"pql": "FIND host"}"#))
        .unwrap();

    let qresp = app.call(query_req).await.unwrap();
    assert_eq!(qresp.status(), StatusCode::OK);
    let qbytes = axum::body::to_bytes(qresp.into_body(), usize::MAX).await.unwrap();
    let qbody: serde_json::Value = serde_json::from_slice(&qbytes).unwrap();
    assert_eq!(qbody["count"], 2, "expected 2 results from FIND host, got: {qbody}");
}

/// Ingest with referential integrity: relationship endpoint must exist.
#[tokio::test]
async fn ingest_valid_relationship_accepted() {
    let (app, _dir) = open_app();

    let body = r#"{
        "connector_id": "test",
        "sync_id": "s1",
        "entities": [
            {"entity_type": "host",    "entity_key": "h1", "entity_class": "Host"},
            {"entity_type": "service", "entity_key": "svc1", "entity_class": "Service"}
        ],
        "relationships": [
            {"from_type": "host", "from_key": "h1", "verb": "RUNS",
             "to_type": "service", "to_key": "svc1"}
        ]
    }"#;

    let (status, resp) = post(app, "/v1/ingest/sync", body).await;
    assert_eq!(status, StatusCode::OK, "expected OK: {resp}");
    assert_eq!(resp["relationships_created"], 1);
}

/// Dangling relationship (to_id doesn't exist) must be rejected (INV-04).
#[tokio::test]
async fn ingest_dangling_relationship_rejected() {
    let (app, _dir) = open_app();

    let body = r#"{
        "connector_id": "test",
        "sync_id": "s1",
        "entities": [
            {"entity_type": "host", "entity_key": "h1", "entity_class": "Host"}
        ],
        "relationships": [
            {"from_type": "host", "from_key": "h1", "verb": "RUNS",
             "to_type": "service", "to_key": "ghost-service-not-in-batch"}
        ]
    }"#;

    let (status, _) = post(app, "/v1/ingest/sync", body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR,
        "dangling relationship must be rejected");
}

/// Stats endpoint reflects ingested entities.
#[tokio::test]
async fn stats_reflects_ingested_data() {
    let (app1, _dir) = open_app();
    let (app2, _dir2) = open_app();

    // Initial stats: zero entities.
    let (_, before) = get_json(app1, "/v1/stats").await;
    assert_eq!(before["total_entities"], 0);

    // Ingest one entity.
    let body = r#"{
        "connector_id": "c1", "sync_id": "s1",
        "entities": [{"entity_type": "host", "entity_key": "h1", "entity_class": "Host"}],
        "relationships": []
    }"#;
    post(app2.clone(), "/v1/ingest/sync", body).await;

    // Note: same app handle needed for stats to see the ingested data.
    // This test opens a fresh engine so stats start at 0 and sees the write.
    let sync_req = Request::builder()
        .method("POST").uri("/v1/ingest/sync")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_owned())).unwrap();

    let stats_req = Request::builder().method("GET").uri("/v1/stats")
        .body(Body::empty()).unwrap();

    // Use Tower's `call` via a shared service to maintain state.
    use tower::Service;
    let dir3 = TempDir::new().unwrap();
    let engine3 = StorageEngine::open(StoreConfig::new(dir3.path())).unwrap();
    let state3 = AppState::new(engine3);
    let mut app3 = router(state3);

    let _ = app3.call(sync_req).await.unwrap();
    let stats_resp = app3.call(stats_req).await.unwrap();
    let bytes = axum::body::to_bytes(stats_resp.into_body(), usize::MAX).await.unwrap();
    let stats: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stats["total_entities"], 1);
}

/// Entity lookup by ID after ingest via direct write endpoint.
#[tokio::test]
async fn entity_lookup_after_write() {
    use parallax_core::entity::EntityId;

    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::open(StoreConfig::new(dir.path())).unwrap();
    let state = AppState::new(engine);

    use tower::Service;
    let mut app = router(state);

    let write_body = r#"{
        "write_id": "w1",
        "entities": [{"entity_type": "host", "entity_key": "h1", "entity_class": "Host"}],
        "relationships": []
    }"#;

    let write_req = Request::builder()
        .method("POST").uri("/v1/ingest/write")
        .header("Content-Type", "application/json")
        .body(Body::from(write_body)).unwrap();

    app.call(write_req).await.unwrap();

    // Derive the expected entity ID (same logic as EntityId::derive with account "default").
    let id = EntityId::derive("default", "host", "h1");
    let id_hex = format!("{id}");

    let lookup_req = Request::builder()
        .method("GET").uri(format!("/v1/entities/{id_hex}"))
        .body(Body::empty()).unwrap();

    let resp = app.call(lookup_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["type"], "host");
}

/// Full pipeline: ingest → PQL property filter → correct count.
#[tokio::test]
async fn pql_property_filter_after_ingest() {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::open(StoreConfig::new(dir.path())).unwrap();
    let state = AppState::new(engine);

    use tower::Service;
    let mut app = router(state);

    // Ingest two hosts: one running, one stopped.
    let ingest = r#"{
        "connector_id": "c1", "sync_id": "s1",
        "entities": [
            {"entity_type": "host", "entity_key": "h1", "entity_class": "Host",
             "properties": {"state": "running"}},
            {"entity_type": "host", "entity_key": "h2", "entity_class": "Host",
             "properties": {"state": "stopped"}}
        ],
        "relationships": []
    }"#;

    let ingest_req = Request::builder()
        .method("POST").uri("/v1/ingest/sync")
        .header("Content-Type", "application/json")
        .body(Body::from(ingest)).unwrap();
    app.call(ingest_req).await.unwrap();

    // Query: only running hosts.
    let query_req = Request::builder()
        .method("POST").uri("/v1/query")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"pql": "FIND host WITH state = 'running'"}"#))
        .unwrap();
    let resp = app.call(query_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["count"], 1, "only 1 running host expected, got: {json}");
    assert_eq!(json["entities"][0]["entity_type"], "host");
}
