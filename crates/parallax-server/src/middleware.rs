//! Server middleware — authentication and request tracing.
//!
//! **Spec reference:** `specs/06-api-surface.md` §6.4, §6.8
//!
//! INV-A01: Every API request is authenticated. Unauthenticated requests
//!          receive 401 with no information leakage.
//! INV-A02: API keys are never logged, returned in responses, or stored in plaintext.
//! INV-A03: Rate limits are enforced before query execution, not after.
//! INV-A05: All responses include request-id for tracing.

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use tracing::warn;

/// Constant-time string comparison to avoid timing attacks (INV-A02).
///
/// Does NOT short-circuit on length mismatch to avoid leaking key length
/// via a timing side-channel. Pads the shorter input with zeros so the
/// byte-fold always runs to the length of the longer string.
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let max_len = a.len().max(b.len());
    let diff = a.len() ^ b.len(); // non-zero if lengths differ
    let mismatch = (0..max_len).fold(diff, |acc, i| {
        let ab = a.get(i).copied().unwrap_or(0);
        let bb = b.get(i).copied().unwrap_or(0);
        acc | (ab ^ bb) as usize
    });
    mismatch == 0
}

/// Bearer-token API key authentication middleware (INV-A01, INV-A02).
///
/// Skips authentication for `GET /v1/health` (liveness probe).
/// Returns `401 Unauthorized` with a generic message for any other
/// unauthenticated request — no internal detail is leaked (INV-A06).
///
/// The expected key is loaded once from the `AppState`; it is never
/// echoed back or included in log output.
pub async fn auth_middleware(
    expected_key: Arc<String>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // Health endpoint is always accessible (INV-A01 exception).
    if req.uri().path() == "/v1/health" {
        return next.run(req).await;
    }

    // If no key is configured, the server is in open mode (dev/embedded).
    if expected_key.is_empty() {
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            warn!("unauthenticated request to {}", req.uri().path());
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "authentication required"})),
            )
                .into_response();
        }
    };

    if !ct_eq(token, expected_key.as_str()) {
        warn!("invalid API key for {}", req.uri().path());
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "authentication required"})),
        )
            .into_response();
    }

    next.run(req).await
}

/// Request-ID injection middleware (INV-A05).
///
/// Generates a UUID v4 request ID if none is provided by the caller,
/// and propagates it as `X-Request-Id` in both the request (for
/// downstream handlers) and the response.
pub async fn request_id_middleware(mut req: Request<Body>, next: Next) -> Response {
    static X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

    // Use provided request-id or generate a new one.
    let id = req
        .headers()
        .get(&X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let id_value = HeaderValue::from_str(&id).unwrap_or_else(|_| HeaderValue::from_static(""));

    // Inject into request headers so handlers can read it.
    req.headers_mut().insert(X_REQUEST_ID.clone(), id_value.clone());

    let mut resp = next.run(req).await;

    // Propagate to response (INV-A05).
    resp.headers_mut().insert(X_REQUEST_ID.clone(), id_value);

    resp
}
