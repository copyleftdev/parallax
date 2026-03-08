//! HTTP route handlers — REST API (INV-A04, INV-A05, INV-A06).
//!
//! **Spec reference:** `specs/06-api-surface.md` §6.3

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use compact_str::CompactString;
use parallax_connect::builder::{entity as build_entity, relationship as build_relationship};
use parallax_core::{
    entity::EntityId, property::Value, relationship::RelationshipId, source::SourceTag,
    timestamp::Timestamp,
};
use parallax_graph::GraphReader;
use parallax_policy::{compute_posture, PolicyEvaluator, PolicyRule, RuleStatus};
use parallax_query::{execute, parse, plan, QueryLimits, QueryResult};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::state::AppState;

// ─── Request / response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct QueryRequest {
    pub pql: String,
}

#[derive(Serialize)]
pub struct QueryResponse {
    pub count: u64,
    /// Simplified result: list of entity display names + types.
    pub entities: Vec<EntitySummary>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct EntitySummary {
    pub id: String,
    pub entity_type: String,
    pub display_name: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
}

#[derive(Serialize)]
pub struct StatsResponse {
    pub total_entities: usize,
    pub total_relationships: usize,
    pub entity_types: usize,
    pub entity_classes: usize,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub request_id: String,
}

/// A single entity record in an ingest request.
#[derive(Deserialize)]
pub struct IngestEntity {
    pub entity_type: String,
    pub entity_key: String,
    pub entity_class: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub properties: std::collections::HashMap<String, serde_json::Value>,
}

/// A single relationship record in an ingest request.
#[derive(Deserialize)]
pub struct IngestRelationship {
    pub from_type: String,
    pub from_key: String,
    pub verb: String,
    pub to_type: String,
    pub to_key: String,
    #[serde(default)]
    pub properties: std::collections::HashMap<String, serde_json::Value>,
}

/// `POST /v1/ingest/write` request body — raw upsert without sync context.
#[derive(Deserialize)]
pub struct WriteRequest {
    /// Caller-supplied identifier for this write batch (for idempotency tracking).
    pub write_id: String,
    #[serde(default)]
    pub entities: Vec<IngestEntity>,
    #[serde(default)]
    pub relationships: Vec<IngestRelationship>,
}

/// `POST /v1/ingest/sync` request body.
#[derive(Deserialize)]
pub struct SyncRequest {
    pub connector_id: String,
    pub sync_id: String,
    #[serde(default)]
    pub entities: Vec<IngestEntity>,
    #[serde(default)]
    pub relationships: Vec<IngestRelationship>,
}

#[derive(Serialize)]
pub struct SyncResponse {
    pub sync_id: String,
    pub entities_created: u64,
    pub entities_updated: u64,
    pub entities_deleted: u64,
    pub relationships_created: u64,
    pub relationships_deleted: u64,
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// `GET /v1/health` — liveness probe (INV-A01: no auth required for health).
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        version: state.version,
        uptime_seconds: state.started_at.elapsed().as_secs(),
    })
}

/// `GET /v1/stats` — graph statistics.
pub async fn stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let s = state.current_stats();
    Json(StatsResponse {
        total_entities: s.total_entities,
        total_relationships: s.total_relationships,
        entity_types: s.type_counts.len(),
        entity_classes: s.class_counts.len(),
    })
}

/// `POST /v1/query` — execute a PQL query.
pub async fn query(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let stats = state.current_stats();

    // Parse and plan.
    let ast = match parse(&req.pql) {
        Ok(q) => q,
        Err(e) => {
            warn!(error = %e, pql = %req.pql, "PQL parse error");
            return (
                StatusCode::BAD_REQUEST,
                Json(QueryResponse {
                    count: 0,
                    entities: vec![],
                    error: Some(e.to_string()),
                }),
            );
        }
    };

    let query_plan = match plan(ast, &stats) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(QueryResponse {
                    count: 0,
                    entities: vec![],
                    error: Some(e.to_string()),
                }),
            );
        }
    };

    // Execute against current snapshot.
    let engine = state.engine.lock().expect("engine lock");
    let snap = engine.snapshot();
    drop(engine);

    let graph = GraphReader::new(&snap);
    let result = match execute(&query_plan, &graph, QueryLimits::default()) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(QueryResponse {
                    count: 0,
                    entities: vec![],
                    error: Some(e.to_string()),
                }),
            );
        }
    };

    let count = result.count();
    let entities = match result {
        QueryResult::Entities(ents) => ents
            .iter()
            .map(|e| EntitySummary {
                id: format!("{}", e.id),
                entity_type: e._type.as_str().to_owned(),
                display_name: e.display_name.as_str().to_owned(),
            })
            .collect(),
        QueryResult::Traversals(ts) => ts
            .iter()
            .map(|t| EntitySummary {
                id: format!("{}", t.entity.id),
                entity_type: t.entity._type.as_str().to_owned(),
                display_name: t.entity.display_name.as_str().to_owned(),
            })
            .collect(),
        QueryResult::Scalar(_) => vec![],
        QueryResult::Paths(_) => vec![],
        QueryResult::Grouped(_) => vec![],
    };

    (
        StatusCode::OK,
        Json(QueryResponse {
            count,
            entities,
            error: None,
        }),
    )
}

/// `GET /v1/entities/:id` — direct entity lookup by hex-encoded ID.
pub async fn get_entity(
    State(state): State<AppState>,
    Path(id_hex): Path<String>,
) -> impl IntoResponse {
    let entity_id = match parse_entity_id(&id_hex) {
        Some(id) => id,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "invalid entity ID format (expected 32-char hex)"}),
                ),
            );
        }
    };

    let engine = state.engine.lock().expect("engine lock");
    let snap = engine.snapshot();
    drop(engine);

    match snap.get_entity(entity_id) {
        Some(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id_hex,
                "type": e._type.as_str(),
                "class": e._class.as_str(),
                "display_name": e.display_name.as_str(),
                "properties": serde_json::to_value(&e.properties).unwrap_or(serde_json::Value::Null),
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "entity not found"})),
        ),
    }
}

/// `GET /v1/relationships/:id` — direct relationship lookup by hex-encoded ID.
pub async fn get_relationship(
    State(state): State<AppState>,
    Path(id_hex): Path<String>,
) -> impl IntoResponse {
    let bytes = match hex::decode(&id_hex) {
        Ok(b) if b.len() == 16 => b,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "invalid relationship ID format (expected 32-char hex)"}),
                ),
            );
        }
    };
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    let rel_id = RelationshipId(arr);

    let engine = state.engine.lock().expect("engine lock");
    let snap = engine.snapshot();
    drop(engine);

    match snap.get_relationship(rel_id) {
        Some(r) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id_hex,
                "from_id": format!("{}", r.from_id),
                "to_id": format!("{}", r.to_id),
                "class": r._class.as_str(),
                "properties": serde_json::to_value(&r.properties).unwrap_or(serde_json::Value::Null),
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "relationship not found"})),
        ),
    }
}

/// `POST /v1/ingest/sync` — connector sync commit.
pub async fn ingest_sync(
    State(state): State<AppState>,
    Json(req): Json<SyncRequest>,
) -> impl IntoResponse {
    let source = SourceTag {
        connector_id: CompactString::new(&req.connector_id),
        sync_id: CompactString::new(&req.sync_id),
        sync_timestamp: Timestamp::now(),
    };

    // Materialize entities via EntityBuilder.
    let entities: Vec<_> = req
        .entities
        .iter()
        .map(|e| {
            let mut b = build_entity(&e.entity_type, &e.entity_key);
            if let Some(ref cls) = e.entity_class {
                b = b.class(cls);
            }
            if let Some(ref name) = e.display_name {
                b = b.display_name(name);
            }
            for (k, v) in &e.properties {
                if let Some(pv) = json_to_value(v) {
                    b = b.property(k, pv);
                }
            }
            b.build("default", source.clone())
        })
        .collect();

    // Materialize relationships via RelationshipBuilder.
    let relationships: Vec<_> = req
        .relationships
        .iter()
        .filter_map(|r| {
            let mut b = build_relationship(&r.from_key, &r.verb, &r.to_key)
                .from_type(&r.from_type)
                .to_type(&r.to_type);
            for (k, v) in &r.properties {
                if let Some(pv) = json_to_value(v) {
                    b = b.property(k, pv);
                }
            }
            b.build("default", source.clone())
        })
        .collect();

    match state
        .sync
        .commit_sync(&req.connector_id, &req.sync_id, entities, relationships)
    {
        Ok(result) => {
            // Refresh cached stats after successful sync.
            state.refresh_stats();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "sync_id": result.sync_id,
                    "entities_created": result.stats.entities_created,
                    "entities_updated": result.stats.entities_updated,
                    "entities_deleted": result.stats.entities_deleted,
                    "relationships_created": result.stats.relationships_created,
                    "relationships_deleted": result.stats.relationships_deleted,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// `GET /v1/connectors` — list registered connectors (stub for v0.1).
pub async fn list_connectors() -> impl IntoResponse {
    Json(serde_json::json!({
        "connectors": [],
        "note": "Connector registry is managed externally in v0.1. Use POST /v1/ingest/sync to push data."
    }))
}

/// `POST /v1/connectors/:id/sync` — trigger a connector sync (stub for v0.1).
pub async fn trigger_connector_sync(Path(connector_id): Path<String>) -> impl IntoResponse {
    // v0.1: connectors push via POST /v1/ingest/sync; pull-mode triggers are v0.2.
    Json(serde_json::json!({
        "connector_id": connector_id,
        "status": "accepted",
        "note": "Pull-mode sync triggers are deferred to v0.2. Connectors should push via POST /v1/ingest/sync."
    }))
}

/// `POST /v1/ingest/write` — raw WriteBatch endpoint (direct API ingestion).
///
/// Accepts a list of entity and relationship upserts/deletes without a
/// connector sync context. Useful for scripting and one-off imports.
pub async fn ingest_write(
    State(state): State<AppState>,
    Json(req): Json<WriteRequest>,
) -> impl IntoResponse {
    let source = SourceTag {
        connector_id: CompactString::new("direct"),
        sync_id: CompactString::new(&req.write_id),
        sync_timestamp: Timestamp::now(),
    };

    let entities: Vec<_> = req
        .entities
        .iter()
        .map(|e| {
            let mut b = build_entity(&e.entity_type, &e.entity_key);
            if let Some(ref cls) = e.entity_class {
                b = b.class(cls);
            }
            if let Some(ref name) = e.display_name {
                b = b.display_name(name);
            }
            for (k, v) in &e.properties {
                if let Some(pv) = json_to_value(v) {
                    b = b.property(k, pv);
                }
            }
            b.build("default", source.clone())
        })
        .collect();

    let relationships: Vec<_> = req
        .relationships
        .iter()
        .filter_map(|r| {
            let mut b = build_relationship(&r.from_key, &r.verb, &r.to_key)
                .from_type(&r.from_type)
                .to_type(&r.to_type);
            for (k, v) in &r.properties {
                if let Some(pv) = json_to_value(v) {
                    b = b.property(k, pv);
                }
            }
            b.build("default", source.clone())
        })
        .collect();

    let mut batch = parallax_store::WriteBatch::new();
    for e in entities {
        batch.upsert_entity(e);
    }
    for r in relationships {
        batch.upsert_relationship(r);
    }

    let written = batch.len();
    {
        let mut engine = state.engine.lock().expect("engine lock");
        match engine.write(batch) {
            Ok(_) => {}
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                );
            }
        }
    }
    state.refresh_stats();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "write_id": req.write_id,
            "ops_written": written,
        })),
    )
}

/// `GET /metrics` — Prometheus text format metrics (INV-A07: observability).
pub async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let engine = state.engine.lock().expect("engine lock");
    let m = engine.metrics().snapshot();
    let snap = engine.snapshot();
    let entity_count = snap.entity_count();
    let rel_count = snap.relationship_count();
    drop(engine);

    let body = format!(
        "# HELP parallax_wal_appends_total Total WAL batch appends\n\
         # TYPE parallax_wal_appends_total counter\n\
         parallax_wal_appends_total {wal_appends}\n\
         # HELP parallax_wal_bytes_written_total Total bytes written to WAL\n\
         # TYPE parallax_wal_bytes_written_total counter\n\
         parallax_wal_bytes_written_total {wal_bytes}\n\
         # HELP parallax_memtable_inserts_total Total MemTable write ops\n\
         # TYPE parallax_memtable_inserts_total counter\n\
         parallax_memtable_inserts_total {memtable_inserts}\n\
         # HELP parallax_memtable_bytes Current MemTable size in bytes\n\
         # TYPE parallax_memtable_bytes gauge\n\
         parallax_memtable_bytes {memtable_bytes}\n\
         # HELP parallax_snapshots_published_total Total MVCC snapshots published\n\
         # TYPE parallax_snapshots_published_total counter\n\
         parallax_snapshots_published_total {snapshots}\n\
         # HELP parallax_entities_total Current total live entities\n\
         # TYPE parallax_entities_total gauge\n\
         parallax_entities_total {entities}\n\
         # HELP parallax_relationships_total Current total live relationships\n\
         # TYPE parallax_relationships_total gauge\n\
         parallax_relationships_total {relationships}\n",
        wal_appends = m.wal_appends,
        wal_bytes = m.wal_bytes_written,
        memtable_inserts = m.memtable_inserts,
        memtable_bytes = m.memtable_bytes,
        snapshots = m.snapshots_published,
        entities = entity_count,
        relationships = rel_count,
    );

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

// ─── Policy handlers ──────────────────────────────────────────────────────────

/// `GET /v1/policies` — list all loaded policy rules.
pub async fn list_policies(State(state): State<AppState>) -> impl IntoResponse {
    let rules = state.policies.lock().expect("policies lock");
    let list: Vec<_> = rules
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "name": r.name,
                "severity": r.severity.to_string(),
                "description": r.description,
                "query": r.query,
                "enabled": r.enabled,
                "frameworks": r.frameworks.iter().map(|f| serde_json::json!({
                    "framework": f.framework,
                    "control": f.control,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(serde_json::json!({ "rules": list, "count": list.len() }))
}

/// `POST /v1/policies` — replace the loaded rule set from a JSON body.
///
/// Body: `{ "rules": [ PolicyRule JSON, … ] }`
///
/// PQL in each rule is validated at load time (INV-P06).
pub async fn set_policies(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rule_values = match body.get("rules").and_then(|v| v.as_array()) {
        Some(arr) => arr.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "body must contain a 'rules' array"})),
            );
        }
    };

    let rules: Vec<PolicyRule> = match serde_json::from_value(serde_json::Value::Array(rule_values))
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("rule parse error: {e}")})),
            );
        }
    };

    // Validate PQL for each rule (INV-P06).
    let stats = state.current_stats();
    let enabled: Vec<PolicyRule> = rules.iter().filter(|r| r.enabled).cloned().collect();
    if let Err(e) = PolicyEvaluator::load(enabled, &stats) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("policy validation failed: {e}")})),
        );
    }

    let count = rules.len();
    *state.policies.lock().expect("policies lock") = rules;

    (StatusCode::OK, Json(serde_json::json!({ "loaded": count })))
}

/// `POST /v1/policies/evaluate` — evaluate all enabled rules against current snapshot.
pub async fn evaluate_policies(State(state): State<AppState>) -> impl IntoResponse {
    let rules = state.policies.lock().expect("policies lock").clone();
    let stats = state.current_stats();

    let enabled: Vec<PolicyRule> = rules.iter().filter(|r| r.enabled).cloned().collect();
    if enabled.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "results": [], "total": 0, "pass": 0, "fail": 0 })),
        );
    }

    let evaluator = match PolicyEvaluator::load(enabled, &stats) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    };

    let engine = state.engine.lock().expect("engine lock");
    let snap = engine.snapshot();
    drop(engine);

    let graph = GraphReader::new(&snap);
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());

    let pass = results
        .iter()
        .filter(|r| r.status == RuleStatus::Pass)
        .count();
    let fail = results
        .iter()
        .filter(|r| r.status == RuleStatus::Fail)
        .count();
    let total = results.len();

    let json_results: Vec<_> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "rule_id": r.rule_id,
                "status": format!("{:?}", r.status),
                "violation_count": r.violations.len(),
                "error": r.error.as_ref().map(|e| e.to_string()),
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "results": json_results,
            "total": total,
            "pass": pass,
            "fail": fail,
        })),
    )
}

/// `GET /v1/policies/posture?framework=<name>` — compute compliance posture.
pub async fn policy_posture(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let framework = params
        .get("framework")
        .cloned()
        .unwrap_or_else(|| "CIS-Controls-v8".to_owned());

    let rules = state.policies.lock().expect("policies lock").clone();
    let stats = state.current_stats();

    let enabled: Vec<PolicyRule> = rules.iter().filter(|r| r.enabled).cloned().collect();
    if enabled.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "framework": framework,
                "overall_score": 1.0,
                "controls": [],
                "note": "No policies loaded",
            })),
        );
    }

    let evaluator = match PolicyEvaluator::load(enabled.clone(), &stats) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    };

    let engine = state.engine.lock().expect("engine lock");
    let snap = engine.snapshot();
    drop(engine);

    let graph = GraphReader::new(&snap);
    let results = evaluator.evaluate_all(&graph, QueryLimits::default());
    let posture = compute_posture(&framework, &enabled, &results);

    let controls: Vec<_> = posture
        .controls
        .iter()
        .map(|c| {
            serde_json::json!({
                "control_id": c.control_id,
                "status": format!("{:?}", c.status),
                "rule_count": c.rule_count,
                "pass_count": c.pass_count,
                "fail_count": c.fail_count,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "framework": posture.framework,
            "overall_score": posture.overall_score,
            "controls": controls,
        })),
    )
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn parse_entity_id(id_hex: &str) -> Option<EntityId> {
    let bytes = hex::decode(id_hex).ok()?;
    if bytes.len() != 16 {
        return None;
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&bytes);
    Some(EntityId(arr))
}

/// Convert a `serde_json::Value` to a Parallax `Value`.
/// Returns `None` for unsupported types (objects, null arrays).
fn json_to_value(v: &serde_json::Value) -> Option<Value> {
    match v {
        serde_json::Value::String(s) => Some(Value::from(s.as_str())),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Value::from(i))
            } else {
                n.as_f64().map(Value::from)
            }
        }
        serde_json::Value::Bool(b) => Some(Value::from(*b)),
        serde_json::Value::Null => Some(Value::Null),
        serde_json::Value::Array(arr) => {
            // Only string arrays are supported.
            let strings: Option<Vec<CompactString>> = arr
                .iter()
                .map(|v| v.as_str().map(CompactString::new))
                .collect();
            strings.map(Value::StringList)
        }
        serde_json::Value::Object(_) => None, // INV-03: no nested objects
    }
}
