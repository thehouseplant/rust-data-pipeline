use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("unsupported asset type for file '{0}'")]
    UnsupportedType(String),

    #[error("failed to parse file '{filename}': {reason}")]
    ParseError { filename: String, reason: String },

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("multipart error: {0}")]
    Multipart(#[from] axum::extract::multipart::MultipartError),

    #[error("no file provided in upload")]
    NoFileProvided,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("missing or invalid API key")]
    Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::UnsupportedType(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::ParseError { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::NoFileProvided => StatusCode::BAD_REQUEST,
            AppError::Multipart(_) => StatusCode::BAD_REQUEST,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        };

        // Don't log full request failure details for auth failures at
        // error level - a stream of bad/missing keys from a misconfigured
        // client (or a scan) shouldn't fill error-level logs the way a
        // genuine parse/DB failure should. warn! is enough to notice a
        // pattern without treating every failed auth attempt as urgent.
        if matches!(self, AppError::Unauthorized) {
            tracing::warn!("rejected request: missing or invalid API key");
        } else {
            tracing::error!(error = %self, "request failed");
        }

        let body = Json(json!({
            "error": self.to_string(),
        }));

        (status, body).into_response()
    }
}

/// Errors specific to the ingestion/parsing step, kept separate from AppError
/// so Ingestor impls don't need to know about HTTP concerns.
#[derive(thiserror::Error, Debug)]
pub enum IngestError {
    #[error("{0}")]
    Malformed(String),
}
