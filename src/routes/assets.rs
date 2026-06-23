use crate::{error::AppError, AppState};
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct ListAssetsParams {
    /// Filter by asset_type, e.g. "csv_row", "json", "image_metadata".
    /// Omit to return all types.
    #[serde(rename = "type", default)]
    asset_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    25
}

#[derive(Serialize)]
pub struct ListAssetsResponse {
    assets: Vec<Value>,
    total: i64,
    limit: i64,
    offset: i64,
}

/// Typed row struct for query_as! - selects columns directly rather than
/// routing through jsonb_build_object, which query_as! can't type-check.
/// Serialized to serde_json::Value before returning to the client so the
/// API response shape stays the same.
struct AssetRow {
    id: Uuid,
    source_filename: String,
    asset_type: String,
    payload: Value,
    batch_id: Uuid,
    row_index: Option<i32>,
    created_at: DateTime<Utc>,
}

impl From<AssetRow> for Value {
    fn from(row: AssetRow) -> Value {
        json!({
            "id": row.id,
            "source_filename": row.source_filename,
            "asset_type": row.asset_type,
            "payload": row.payload,
            "batch_id": row.batch_id,
            "row_index": row.row_index,
            "created_at": row.created_at,
        })
    }
}

/// Paginated, optionally type-filtered list of ingested assets, newest first.
pub async fn list_assets_handler(
    State(state): State<AppState>,
    Query(params): Query<ListAssetsParams>,
) -> Result<Json<ListAssetsResponse>, AppError> {
    // Clamp limit so a careless client can't request an unbounded page size.
    let limit = params.limit.clamp(1, 200);
    let offset = params.offset.max(0);

    let (assets, total) = match &params.asset_type {
        Some(asset_type) => {
            let total = sqlx::query_scalar!(
                r#"SELECT COUNT(*) as "count!: i64" FROM ingested_assets WHERE asset_type = $1"#,
                asset_type
            )
            .fetch_one(&state.db)
            .await?;

            let rows = sqlx::query_as!(
                AssetRow,
                r#"
                SELECT
                    id,
                    source_filename,
                    asset_type,
                    payload as "payload: Value",
                    batch_id,
                    row_index,
                    created_at
                FROM ingested_assets
                WHERE asset_type = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
                asset_type,
                limit,
                offset,
            )
            .fetch_all(&state.db)
            .await?;

            (rows.into_iter().map(Value::from).collect::<Vec<_>>(), total)
        }
        None => {
            let total = sqlx::query_scalar!(
                r#"SELECT COUNT(*) as "count!: i64" FROM ingested_assets"#
            )
            .fetch_one(&state.db)
            .await?;

            let rows = sqlx::query_as!(
                AssetRow,
                r#"
                SELECT
                    id,
                    source_filename,
                    asset_type,
                    payload as "payload: Value",
                    batch_id,
                    row_index,
                    created_at
                FROM ingested_assets
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
                limit,
                offset,
            )
            .fetch_all(&state.db)
            .await?;

            (rows.into_iter().map(Value::from).collect::<Vec<_>>(), total)
        }
    };

    Ok(Json(ListAssetsResponse {
        assets,
        total,
        limit,
        offset,
    }))
}

#[derive(Serialize)]
pub struct AssetType {
    asset_type: String,
    count: i64,
}

/// Distinct asset types currently present, with counts - lets the UI build
/// its filter options from real data instead of a hardcoded list that can
/// drift out of sync as new Ingestor types get added.
pub async fn list_asset_types_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<AssetType>>, AppError> {
    // COUNT(*) is marked non-nullable with ! because Postgres' wire
    // protocol reports it as nullable even though it can never be NULL -
    // query_as! would otherwise infer it as Option<i64>.
    let types = sqlx::query_as!(
        AssetType,
        r#"
        SELECT asset_type, COUNT(*) as "count!: i64"
        FROM ingested_assets
        GROUP BY asset_type
        ORDER BY asset_type
        "#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(types))
}
