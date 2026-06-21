use crate::{auth::require_api_key, AppState};
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;

pub mod assets;
pub mod upload;

pub fn build_router(state: AppState) -> Router {
    // Deliberately two routers merged together, not one router with
    // per-route auth exclusions: this makes "/health is the only
    // unauthenticated route" something you can see at a glance here,
    // rather than something you'd have to verify by checking that every
    // other route individually opted in to the auth layer.
    let public_routes = Router::new().route("/health", get(health_handler));

    let protected_routes = Router::new()
        .route("/upload", post(upload::upload_handler))
        .route("/assets/recent", get(upload::list_recent_handler))
        .route("/assets", get(assets::list_assets_handler))
        .route("/assets/types", get(assets::list_asset_types_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

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
