use crate::{error::AppError, AppState};
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

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
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ingested_assets WHERE asset_type = $1",
            )
            .bind(asset_type)
            .fetch_one(&state.db)
            .await?;

            let rows: Vec<(Value,)> = sqlx::query_as(
                r#"
                SELECT jsonb_build_object(
                    'id', id,
                    'source_filename', source_filename,
                    'asset_type', asset_type,
                    'payload', payload,
                    'batch_id', batch_id,
                    'row_index', row_index,
                    'created_at', created_at
                )
                FROM ingested_assets
                WHERE asset_type = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(asset_type)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?;

            (rows.into_iter().map(|(v,)| v).collect(), total)
        }
        None => {
            let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingested_assets")
                .fetch_one(&state.db)
                .await?;

            let rows: Vec<(Value,)> = sqlx::query_as(
                r#"
                SELECT jsonb_build_object(
                    'id', id,
                    'source_filename', source_filename,
                    'asset_type', asset_type,
                    'payload', payload,
                    'batch_id', batch_id,
                    'row_index', row_index,
                    'created_at', created_at
                )
                FROM ingested_assets
                ORDER BY created_at DESC
                LIMIT $1 OFFSET $2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?;

            (rows.into_iter().map(|(v,)| v).collect(), total)
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
    let rows = sqlx::query(
        r#"
        SELECT asset_type, COUNT(*) as count
        FROM ingested_assets
        GROUP BY asset_type
        ORDER BY asset_type
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let types = rows
        .into_iter()
        .map(|row| {
            // COUNT(*) is reported as nullable by Postgres' wire protocol
            // metadata even though it can never actually be NULL; using
            // try_get with a fallback avoids a runtime panic if a given
            // sqlx/Postgres version combination surfaces it as Option<i64>.
            let count: i64 = row
                .try_get::<i64, _>("count")
                .or_else(|_| row.try_get::<Option<i64>, _>("count").map(|v| v.unwrap_or(0)))
                .unwrap_or(0);

            AssetType {
                asset_type: row.get("asset_type"),
                count,
            }
        })
        .collect();

    Ok(Json(types))
}
