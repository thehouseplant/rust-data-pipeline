use crate::AppState;
use axum::{
    routing::{get, post},
    Router,
};

pub mod upload;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/upload", post(upload::upload_handler))
        .route("/assets/recent", get(upload::list_recent_handler))
        .with_state(state)
}

async fn health_handler() -> &'static str {
    "ok"
}
