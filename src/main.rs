mod auth;
mod config;
mod db;
mod error;
mod ingest;
mod routes;

use axum::extract::DefaultBodyLimit;
use config::Config;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub api_key: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok(); // ok() because .env is optional in prod

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();

    tracing::info!("connecting to database");
    let pool = db::create_pool(&config.database_url).await?;

    tracing::info!("running migrations");
    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = AppState {
        db: pool,
        api_key: config.api_key.clone(),
    };

    let app = routes::build_router(state).layer(DefaultBodyLimit::max(config.max_upload_bytes));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(listener, app).await?;

    Ok(())
}

// Re-export so submodules (routes, ingest) can refer to crate::AppState etc.
pub use error::AppError;
