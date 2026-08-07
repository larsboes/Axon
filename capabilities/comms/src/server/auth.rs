use super::*;

pub(super) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Axum middleware layer that rejects requests without a valid shared secret.
/// Reads `Authorization: Bearer <token>` or `X-Axon-Token: <token>` and
/// compares constant-time against the configured value. Health and read-only
/// routes bypass this entirely — only mutating routes carry the layer.
pub(super) async fn require_auth(
    headers: axum::http::HeaderMap,
    axum::extract::State(secret): axum::extract::State<Option<String>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let expected = match &secret {
        Some(s) if !s.is_empty() => s.as_str(),
        _ => {
            // No secret configured: block mutating routes with a helpful message.
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "api_secret_file is not configured — mutating routes are disabled. See comms.config.example.json."
                })),
            ).into_response();
        }
    };

    // Try Authorization: Bearer first, then X-Axon-Token.
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-axon-token").and_then(|v| v.to_str().ok()));

    match token {
        Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing authentication token" })),
        )
            .into_response(),
    }
}

/// Convert a (StatusCode, Json<Value>) into an axum Response for the auth
/// middleware's error paths.
use axum::response::IntoResponse;
