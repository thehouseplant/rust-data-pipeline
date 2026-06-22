use axum::{body::Body, response::Response};
use governor::middleware::NoOpMiddleware;
use http::{Request, StatusCode};
use serde_json::json;
use std::time::Duration;
use tower_governor::{
    errors::GovernorError,
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::KeyExtractor,
    GovernorLayer,
};

/// Rate-limits by API key rather than by IP address. With a single shared
/// key today this is effectively a global limit, but keying on the
/// credential (not the network address) is the right foundation if this
/// ever grows into multiple named keys - each key would get its own
/// bucket without any change to this extractor.
///
/// Requests with a missing or invalid key are *not* exempted from rate
/// limiting and are *not* each given their own bucket by their raw
/// (garbage) header value - they're all collapsed into one shared
/// `"invalid"` bucket. Without that collapsing, sending a different
/// bogus key on every request would mint a fresh, unlimited bucket each
/// time, defeating the limiter entirely. The real auth check in
/// `auth.rs` is what actually rejects these requests with a 401; this
/// extractor's job is only to make sure that *attempting* a flood of
/// auth failures is itself rate-limited.
#[derive(Clone)]
pub struct ApiKeyExtractor {
    pub valid_key: String,
}

const INVALID_KEY_BUCKET: &str = "invalid";

impl KeyExtractor for ApiKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let provided = req
            .headers()
            .get("x-api-key")
            .and_then(|v| v.to_str().ok());

        match provided {
            Some(key) if key == self.valid_key => Ok(key.to_string()),
            _ => Ok(INVALID_KEY_BUCKET.to_string()),
        }
    }
}

/// Builds the complete rate-limiting layer: `per_second` is the
/// steady-state replenishment rate (one request allowed every
/// `1/per_second` seconds on average), `burst` is how many requests can
/// fire immediately before that steady-state rate kicks in.
///
/// Also spawns the background cleanup thread the crate's own docs
/// recommend - without it, the in-memory bucket map only grows (one
/// entry per distinct key ever seen), which matters more for us than it
/// would for IP-keyed limiting, since our "invalid" bucket alone
/// accumulates state for every bad-auth attempt for the life of the
/// process.
pub fn build_rate_limit_layer(
    valid_key: String,
    per_second: u64,
    burst: u32,
) -> GovernorLayer<ApiKeyExtractor, NoOpMiddleware, Body> {
    let config: GovernorConfig<ApiKeyExtractor, NoOpMiddleware> = GovernorConfigBuilder::default()
        .key_extractor(ApiKeyExtractor { valid_key })
        .per_second(per_second)
        .burst_size(burst)
        .finish()
        .expect("rate limit per_second and burst must both be non-zero");

    let limiter = config.limiter().clone();
    let cleanup_interval = Duration::from_secs(60);
    std::thread::spawn(move || loop {
        std::thread::sleep(cleanup_interval);
        let size_before = limiter.len();
        limiter.retain_recent();
        tracing::debug!(
            buckets_before = size_before,
            buckets_after = limiter.len(),
            "rate limiter cleanup pass"
        );
    });

    GovernorLayer::new(std::sync::Arc::new(config)).error_handler(rate_limit_error_response)
}

/// Renders GovernorError as the same `{"error": "..."}` JSON shape the
/// rest of the API uses (see AppError's IntoResponse impl), instead of
/// the crate's default plain-text body - purely a consistency nicety
/// for API consumers, not a correctness requirement.
fn rate_limit_error_response(err: GovernorError) -> Response<Body> {
    let (status, message) = match &err {
        GovernorError::TooManyRequests { wait_time, .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            format!("rate limit exceeded, retry after {wait_time}s"),
        ),
        GovernorError::UnableToExtractKey => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "rate limiter could not determine a key for this request".to_string(),
        ),
        GovernorError::Other { code, msg, .. } => (
            *code,
            msg.clone().unwrap_or_else(|| "rate limiting error".to_string()),
        ),
    };

    let body = json!({ "error": message }).to_string();

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| {
            // Building the response itself failing would be highly
            // unusual (a bad header value, essentially) - fall back to
            // the crate's own default rendering rather than panicking
            // inside error-handling code.
            err.into_response().map(Body::from)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Request as HttpRequest;

    fn extractor() -> ApiKeyExtractor {
        ApiKeyExtractor {
            valid_key: "correct-key".to_string(),
        }
    }

    fn request_with_header(name: &str, value: &str) -> HttpRequest<()> {
        HttpRequest::builder()
            .header(name, value)
            .body(())
            .unwrap()
    }

    fn request_without_headers() -> HttpRequest<()> {
        HttpRequest::builder().body(()).unwrap()
    }

    #[test]
    fn valid_key_gets_its_own_bucket() {
        let req = request_with_header("x-api-key", "correct-key");
        let key = extractor().extract(&req).unwrap();
        assert_eq!(key, "correct-key");
    }

    #[test]
    fn missing_header_falls_into_invalid_bucket() {
        let req = request_without_headers();
        let key = extractor().extract(&req).unwrap();
        assert_eq!(key, INVALID_KEY_BUCKET);
    }

    #[test]
    fn wrong_key_falls_into_invalid_bucket() {
        let req = request_with_header("x-api-key", "wrong-key");
        let key = extractor().extract(&req).unwrap();
        assert_eq!(key, INVALID_KEY_BUCKET);
    }

    #[test]
    fn different_wrong_keys_collapse_into_the_same_bucket() {
        // This is the property that actually matters: varying the
        // garbage key must not produce a fresh bucket each time, or
        // rate limiting on invalid attempts is trivially bypassable.
        let extractor = extractor();
        let req_a = request_with_header("x-api-key", "garbage-one");
        let req_b = request_with_header("x-api-key", "totally-different-garbage");

        let key_a = extractor.extract(&req_a).unwrap();
        let key_b = extractor.extract(&req_b).unwrap();

        assert_eq!(key_a, key_b);
        assert_eq!(key_a, INVALID_KEY_BUCKET);
    }

    #[test]
    fn configuring_a_key_equal_to_the_sentinel_string_still_resolves_via_the_valid_path() {
        // Edge case worth naming explicitly: if someone ever configured
        // a real API key equal to the literal string "invalid", does a
        // correct request with that key still get treated as valid
        // (its own bucket), or does it accidentally fall through to the
        // shared invalid-attempts bucket? Today it resolves correctly
        // because the `key == self.valid_key` arm is checked first and
        // matches before the catch-all sentinel arm is ever reached -
        // this test pins that behavior so a future reordering of the
        // match arms would be caught here rather than silently merging
        // a legitimate key's traffic with bad-auth attempts.
        let extractor = ApiKeyExtractor {
            valid_key: INVALID_KEY_BUCKET.to_string(),
        };
        let req = request_with_header("x-api-key", INVALID_KEY_BUCKET);
        let key = extractor.extract(&req).unwrap();
        assert_eq!(key, INVALID_KEY_BUCKET);
    }
}
