use crate::{
    auth::require_api_key,
    rate_limit::ApiKeyExtractor,
    AppState,
};
use axum::{
    body::Body,
    middleware,
    routing::{get, post},
    Router,
};
use governor::middleware::NoOpMiddleware;
use tower_governor::GovernorLayer;
use tower_http::cors::CorsLayer;

pub mod assets;
pub mod upload;

/// Type alias purely for readability - GovernorLayer's full generic
/// signature (key extractor, governor middleware, response body type)
/// is verbose to spell out at every call site.
pub type ApiRateLimitLayer = GovernorLayer<ApiKeyExtractor, NoOpMiddleware, Body>;

pub fn build_router(state: AppState, rate_limit: ApiRateLimitLayer) -> Router {
    // Deliberately two routers merged together, not one router with
    // per-route auth exclusions: this makes "/health is the only
    // unauthenticated route" something you can see at a glance here,
    // rather than something you'd have to verify by checking that every
    // other route individually opted in to the auth layer.
    let public_routes = Router::new().route("/health", get(health_handler));

    // Layer order matters here: .layer() calls stack outermost-last, so
    // the rate limiter (added second) wraps the auth check (added
    // first) and therefore runs BEFORE it on incoming requests. This is
    // deliberate - every request gets counted against its bucket (real
    // key or the shared "invalid" bucket) even if auth then rejects it,
    // so a flood of bad-auth attempts is itself throttled rather than
    // bypassing the limiter entirely by being rejected too "early" to
    // count.
    let protected_routes = Router::new()
        .route("/upload", post(upload::upload_handler))
        .route("/assets/recent", get(upload::list_recent_handler))
        .route("/assets", get(assets::list_assets_handler))
        .route("/assets/types", get(assets::list_asset_types_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ))
        .route_layer(rate_limit);

    public_routes
        .merge(protected_routes)
        // Permissive CORS: this API is a local dev tool with a separate
        // React frontend on a different port. Tighten this with
        // .allow_origin(...) before deploying anywhere beyond localhost.
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health_handler() -> &'static str {
    "ok"
}
