use crate::{db::queries, error::AppError, ingest::resolve_ingestor, AppState};
use axum::{extract::Multipart, extract::State, Json};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Serialize)]
pub struct FileResult {
    filename: String,
    status: String, // "ok" | "error"
    asset_type: Option<String>,
    batch_id: Option<Uuid>,
    records_inserted: Option<i64>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct UploadSummary {
    files: Vec<FileResult>,
}

/// Accepts one or more files in a multipart/form-data upload.
/// Each file is processed independently - one bad file does not
/// fail the others; failures are reported per-file in the response.
pub async fn upload_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadSummary>, AppError> {
    let mut results = Vec::new();
    let mut saw_any_file = false;

    while let Some(field) = multipart.next_field().await? {
        let Some(filename) = field.file_name().map(|s| s.to_string()) else {
            // Skip non-file fields (e.g. plain form text fields)
            continue;
        };
        saw_any_file = true;

        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => {
                results.push(FileResult {
                    filename,
                    status: "error".to_string(),
                    asset_type: None,
                    batch_id: None,
                    records_inserted: None,
                    error: Some(format!("failed to read upload stream: {e}")),
                });
                continue;
            }
        };

        results.push(process_single_file(&state, &filename, &data).await);
    }

    if !saw_any_file {
        return Err(AppError::NoFileProvided);
    }

    Ok(Json(UploadSummary { files: results }))
}

async fn process_single_file(state: &AppState, filename: &str, data: &[u8]) -> FileResult {
    let ingestor = match resolve_ingestor(filename, data) {
        Ok(i) => i,
        Err(e) => {
            return FileResult {
                filename: filename.to_string(),
                status: "error".to_string(),
                asset_type: None,
                batch_id: None,
                records_inserted: None,
                error: Some(e.to_string()),
            }
        }
    };

    let records = match ingestor.parse(filename, data) {
        Ok(r) => r,
        Err(e) => {
            return FileResult {
                filename: filename.to_string(),
                status: "error".to_string(),
                asset_type: None,
                batch_id: None,
                records_inserted: None,
                error: Some(e.to_string()),
            }
        }
    };

    let asset_type = records
        .first()
        .map(|r| r.asset_type.to_string())
        .unwrap_or_default();

    match queries::insert_batch(&state.db, filename, &asset_type, records).await {
        Ok(summary) => FileResult {
            filename: filename.to_string(),
            status: "ok".to_string(),
            asset_type: Some(asset_type),
            batch_id: Some(summary.batch_id),
            records_inserted: Some(summary.records_inserted),
            error: None,
        },
        Err(e) => FileResult {
            filename: filename.to_string(),
            status: "error".to_string(),
            asset_type: Some(asset_type),
            batch_id: None,
            records_inserted: None,
            error: Some(format!("database error: {e}")),
        },
    }
}

/// Simple read endpoint to verify ingested data - fetch recent records,
/// optionally filtered by asset_type. Handy for sanity-checking uploads
/// without reaching for psql.
pub async fn list_recent_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<Value>>, AppError> {
    let rows: Vec<(Value,)> = sqlx::query_as(
        r#"
        SELECT jsonb_build_object(
            'id', id,
            'source_filename', source_filename,
            'asset_type', asset_type,
            'payload', payload,
            'batch_id', batch_id,
            'created_at', created_at
        )
        FROM ingested_assets
        ORDER BY created_at DESC
        LIMIT 50
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows.into_iter().map(|(v,)| v).collect()))
}
