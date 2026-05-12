use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use codeless_adapters_host::SecretStore;
use thiserror::Error;

use crate::{AppState, TOKEN_SECRET_KEY};

/// Validates the `Authorization: Bearer <token>` header against the
/// shared bearer token. Used as middleware on every `/rpc/*` route.
/// SSE uses query-param auth instead because `EventSource` cannot set
/// headers — see `sse::events_handler`.
pub(crate) async fn bearer_layer(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let supplied = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);

    match supplied {
        Some(t) if constant_time_eq(t, &state.bearer_token) => Ok(next.run(req).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Constant-time compare so timing observations cannot reveal a
/// prefix match of the token. The token length itself is not secret
/// (32-byte hex by convention from `--init-token`), so length-mismatch
/// short-circuits are fine.
pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[derive(Debug, Error)]
pub enum TokenLoadError {
    #[error(
        "bearer token not configured — run `codeless serve --init-token` first to generate one"
    )]
    Missing,
}

pub(crate) fn load_bearer_token(store: &SecretStore) -> Result<String, TokenLoadError> {
    store
        .get(TOKEN_SECRET_KEY)
        .map(str::to_owned)
        .ok_or(TokenLoadError::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_str_eq() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }
}
