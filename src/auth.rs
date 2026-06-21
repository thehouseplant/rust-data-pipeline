use crate::{error::AppError, AppState};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

const API_KEY_HEADER: &str = "x-api-key";

/// Gates every route it's applied to behind a single shared API key,
/// supplied via the `X-API-Key` header.
///
/// This is deliberately the simplest auth model that's still real auth:
/// one secret, checked in constant time, no sessions or token expiry to
/// manage. It's sized for "keep this off the rest of my LAN," not for
/// multiple distinct users - if that need shows up later, this is the
/// seam where per-key lookup (DB-backed, named keys, revocation) would
/// replace the single Config::api_key comparison.
pub async fn require_api_key(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let provided = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(AppError::Unauthorized)?;

    if constant_time_eq(provided.as_bytes(), state.api_key.as_bytes()) {
        Ok(next.run(request).await)
    } else {
        Err(AppError::Unauthorized)
    }
}

/// Compares two byte slices in constant time with respect to their
/// *contents* - it still short-circuits on length mismatch, which leaks
/// length but not content, and length alone isn't sensitive for a fixed
/// shared secret used the way this one is (it's not a low-entropy guess
/// surface like a 4-digit PIN). A naive `a == b` would let an attacker
/// recover the key one byte at a time by timing how far the comparison
/// gets before it bails - this avoids that without pulling in a crypto
/// crate for one comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_byte_slices_are_equal() {
        assert!(constant_time_eq(b"secret-key-123", b"secret-key-123"));
    }

    #[test]
    fn different_content_same_length_is_not_equal() {
        assert!(!constant_time_eq(b"secret-key-123", b"secret-key-456"));
    }

    #[test]
    fn different_length_is_not_equal() {
        assert!(!constant_time_eq(b"short", b"a-much-longer-key"));
    }

    #[test]
    fn empty_slices_are_equal() {
        // Degenerate case - in practice this never matters here since
        // Config::api_key is read via env::var and won't be empty unless
        // someone sets API_KEY="" deliberately, but worth pinning the
        // behavior rather than leaving it implicit.
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn single_byte_difference_is_detected() {
        assert!(!constant_time_eq(b"aaaaaaaa", b"aaaaaaab"));
    }
}
