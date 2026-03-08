//! REST API integration tests for parallax-server.
//!
//! Tests the full HTTP stack: middleware → router → handler,
//! covering auth (INV-A01), request-ID (INV-A05), and all endpoints.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use parallax_server::{router, AppState};
use parallax_store::{StorageEngine, StoreConfig};
use tempfile::TempDir;
use tower::ServiceExt as _; // for `oneshot`

// ─── helpers ─────────────────────────────────────────────────────────────────

fn open_app(api_key: &str) -> (axum::Router, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::open(StoreConfig::new(dir.path())).expect("open engine");
    let state = AppState::with_key(engine, api_key.to_owned());
    (router(state), dir)
}

async fn get(app: axum::Router, path: &str, token: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::empty()).unwrap();
    app.oneshot(req).await.unwrap()
}

async fn post_json(
    app: axum::Router,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_owned())).unwrap();
    app.oneshot(req).await.unwrap()
}

// ─── health ──────────────────────────────────────────────────────────────────

/// Health endpoint requires no auth (INV-A01 exception).
#[tokio::test]
async fn health_no_auth_required() {
    let (app, _dir) = open_app("secret");
    let resp = get(app, "/v1/health", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Health returns correct JSON fields.
#[tokio::test]
async fn health_response_fields() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/health", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "healthy");
    assert!(json["version"].is_string());
    assert!(json["uptime_seconds"].is_number());
}

// ─── auth middleware ──────────────────────────────────────────────────────────

/// Requests to protected endpoints without token get 401.
#[tokio::test]
async fn protected_endpoint_requires_auth() {
    let (app, _dir) = open_app("my-secret-key");
    let resp = get(app, "/v1/stats", None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Correct bearer token is accepted.
#[tokio::test]
async fn correct_token_accepted() {
    let (app, _dir) = open_app("my-secret-key");
    let resp = get(app, "/v1/stats", Some("my-secret-key")).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

/// Wrong bearer token is rejected.
#[tokio::test]
async fn wrong_token_rejected() {
    let (app, _dir) = open_app("correct-key");
    let resp = get(app, "/v1/stats", Some("wrong-key")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Open mode (empty key) allows all requests.
#[tokio::test]
async fn open_mode_allows_all() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/stats", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── request-ID middleware ────────────────────────────────────────────────────

/// Every response carries X-Request-Id (INV-A05).
#[tokio::test]
async fn response_has_request_id() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/health", None).await;
    assert!(
        resp.headers().contains_key("x-request-id"),
        "X-Request-Id must be present"
    );
}

/// Caller-supplied X-Request-Id is propagated back.
#[tokio::test]
async fn caller_request_id_propagated() {
    let (app, _dir) = open_app("");
    let req = Request::builder()
        .method("GET")
        .uri("/v1/health")
        .header("x-request-id", "my-trace-id-42")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let id = resp
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(id, "my-trace-id-42");
}

// ─── stats ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_returns_counts() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/stats", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["total_entities"].is_number());
    assert!(json["total_relationships"].is_number());
}

// ─── ingest/sync ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn ingest_sync_creates_entities() {
    let (app, _dir) = open_app("");
    let body = r#"{
        "connector_id": "test",
        "sync_id": "s1",
        "entities": [
            {"entity_type": "host", "entity_key": "h1"},
            {"entity_type": "host", "entity_key": "h2"}
        ],
        "relationships": []
    }"#;
    let resp = post_json(app, "/v1/ingest/sync", None, body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["entities_created"], 2);
}

// ─── ingest/write ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn ingest_write_accepts_raw_batch() {
    let (app, _dir) = open_app("");
    let body = r#"{
        "write_id": "w1",
        "entities": [{"entity_type": "host", "entity_key": "h1"}],
        "relationships": []
    }"#;
    let resp = post_json(app, "/v1/ingest/write", None, body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["write_id"], "w1");
    assert!(json["ops_written"].as_u64().unwrap() >= 1);
}

// ─── metrics ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_returns_prometheus_format() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/metrics", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.contains("text/plain"));
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("parallax_wal_appends_total"));
    assert!(text.contains("parallax_entities_total"));
}

// ─── connectors ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_connectors_returns_array() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/connectors", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["connectors"].is_array());
}

// ─── query ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pql_query_returns_result() {
    let (app, _dir) = open_app("");
    let body = r#"{"pql": "FIND *"}"#;
    let resp = post_json(app, "/v1/query", None, body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["count"].is_number());
}

#[tokio::test]
async fn pql_bad_query_returns_400() {
    let (app, _dir) = open_app("");
    let body = r#"{"pql": "NOT VALID PQL!!!"}"#;
    let resp = post_json(app, "/v1/query", None, body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["error"].is_string());
}

// ─── v0.2: policy REST API ────────────────────────────────────────────────────

/// `GET /v1/policies` returns an empty rule list initially.
#[tokio::test]
async fn v02_policy_list_initially_empty() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/policies", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["count"], 0);
    assert!(json["rules"].as_array().unwrap().is_empty());
}

/// `POST /v1/policies` loads rules; `GET /v1/policies` reflects them.
#[tokio::test]
async fn v02_policy_set_and_list() {
    let (app, _dir) = open_app("");
    let body = r#"{
        "rules": [
            {
                "id": "test-001",
                "name": "Test Rule",
                "severity": "high",
                "description": "A test rule",
                "query": "FIND host",
                "frameworks": [],
                "schedule": "manual",
                "remediation": "Fix it.",
                "enabled": true
            }
        ]
    }"#;
    // Set the rules.
    let resp = post_json(app.clone(), "/v1/policies", None, body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["loaded"], 1);

    // List them back.
    let resp = get(app, "/v1/policies", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["rules"][0]["id"], "test-001");
    assert_eq!(json["rules"][0]["severity"], "high");
}

/// `POST /v1/policies` with invalid PQL returns 400 (INV-P06).
#[tokio::test]
async fn v02_policy_set_invalid_pql_rejected() {
    let (app, _dir) = open_app("");
    let body = r#"{
        "rules": [
            {
                "id": "bad-rule",
                "name": "Bad PQL",
                "severity": "medium",
                "description": "",
                "query": "NOT VALID PQL !!!",
                "frameworks": [],
                "schedule": "manual",
                "remediation": "",
                "enabled": true
            }
        ]
    }"#;
    let resp = post_json(app, "/v1/policies", None, body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["error"].is_string(), "must return error message");
}

/// `POST /v1/policies` with missing rules array returns 400.
#[tokio::test]
async fn v02_policy_set_missing_rules_key_rejected() {
    let (app, _dir) = open_app("");
    let body = r#"{"not_rules": []}"#;
    let resp = post_json(app, "/v1/policies", None, body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// `POST /v1/policies/evaluate` with no rules returns empty results.
#[tokio::test]
async fn v02_policy_evaluate_no_rules_returns_empty() {
    let (app, _dir) = open_app("");
    let resp = post_json(app, "/v1/policies/evaluate", None, "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["total"], 0);
    assert_eq!(json["pass"], 0);
    assert_eq!(json["fail"], 0);
    assert!(json["results"].as_array().unwrap().is_empty());
}

/// `POST /v1/policies/evaluate` runs rules against the current graph.
#[tokio::test]
async fn v02_policy_evaluate_with_data() {
    let (app, _dir) = open_app("");

    // Ingest some hosts.
    let ingest_body = r#"{
        "connector_id": "test",
        "sync_id": "s1",
        "entities": [
            {"entity_type": "host", "entity_key": "h1"},
            {"entity_type": "host", "entity_key": "h2"}
        ],
        "relationships": []
    }"#;
    let resp = post_json(app.clone(), "/v1/ingest/sync", None, ingest_body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Load a rule that will find those hosts.
    let set_body = r#"{
        "rules": [
            {
                "id": "all-hosts",
                "name": "All Hosts",
                "severity": "info",
                "description": "Finds all hosts",
                "query": "FIND host",
                "frameworks": [],
                "schedule": "manual",
                "remediation": "",
                "enabled": true
            }
        ]
    }"#;
    let resp = post_json(app.clone(), "/v1/policies", None, set_body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Evaluate.
    let resp = post_json(app, "/v1/policies/evaluate", None, "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["total"], 1, "one rule evaluated");
    // FIND host finds 2 hosts → violations → Fail
    assert_eq!(json["fail"], 1);
    assert_eq!(json["pass"], 0);
}

/// `GET /v1/policies/posture` returns framework posture JSON.
#[tokio::test]
async fn v02_policy_posture_no_policies() {
    let (app, _dir) = open_app("");
    let resp = get(app, "/v1/policies/posture", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // No policies → score defaults to 1.0 with a note.
    assert!(json["overall_score"].is_number());
    assert!(json["note"].is_string());
}

/// `GET /v1/policies/posture?framework=NIST-CSF` filters by the requested framework.
#[tokio::test]
async fn v02_policy_posture_with_framework_param() {
    let (app, _dir) = open_app("");

    // Load a rule with a CIS framework mapping.
    let set_body = r#"{
        "rules": [
            {
                "id": "cis-001",
                "name": "CIS Rule",
                "severity": "high",
                "description": "",
                "query": "FIND host",
                "frameworks": [{"framework": "CIS-Controls-v8", "control": "1.1"}],
                "schedule": "manual",
                "remediation": "",
                "enabled": true
            }
        ]
    }"#;
    let resp = post_json(app.clone(), "/v1/policies", None, set_body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Request posture for a different framework.
    let resp = get(app, "/v1/policies/posture?framework=NIST-CSF", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["framework"], "NIST-CSF");
    // No rules mapped to NIST-CSF → controls array is empty.
    assert!(json["controls"].as_array().unwrap().is_empty());
}

/// Multiple rules can be loaded; disabled rules are not evaluated.
#[tokio::test]
async fn v02_policy_set_disabled_rule_not_evaluated() {
    let (app, _dir) = open_app("");
    let set_body = r#"{
        "rules": [
            {
                "id": "enabled-rule",
                "name": "Enabled",
                "severity": "medium",
                "description": "",
                "query": "FIND host",
                "frameworks": [],
                "schedule": "manual",
                "remediation": "",
                "enabled": true
            },
            {
                "id": "disabled-rule",
                "name": "Disabled",
                "severity": "critical",
                "description": "",
                "query": "FIND host",
                "frameworks": [],
                "schedule": "manual",
                "remediation": "",
                "enabled": false
            }
        ]
    }"#;
    let resp = post_json(app.clone(), "/v1/policies", None, set_body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["loaded"], 2, "both rules loaded");

    // Evaluate — only enabled rule runs.
    let resp = post_json(app, "/v1/policies/evaluate", None, "{}").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Enabled rule: finds nothing in empty graph → Pass.
    // Disabled rule: not included in evaluate payload → total = 1.
    assert_eq!(json["total"], 1, "only the enabled rule is evaluated");
}
