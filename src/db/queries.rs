use crate::ingest::Record;
use sqlx::PgPool;
use uuid::Uuid;

pub struct BatchSummary {
    pub batch_id: Uuid,
    pub records_inserted: i64,
}

/// Creates a batch row, bulk-inserts all records for that batch inside one
/// transaction, and marks the batch completed. If anything fails partway,
/// the transaction rolls back so we never end up with a half-written batch.
pub async fn insert_batch(
    pool: &PgPool,
    filename: &str,
    asset_type: &str,
    records: Vec<Record>,
) -> Result<BatchSummary, sqlx::Error> {
    let batch_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO upload_batches (id, filename, asset_type, status)
        VALUES ($1, $2, $3, 'processing')
        "#,
    )
    .bind(batch_id)
    .bind(filename)
    .bind(asset_type)
    .execute(&mut *tx)
    .await?;

    let mut inserted: i64 = 0;
    for record in &records {
        sqlx::query(
            r#"
            INSERT INTO ingested_assets
                (source_filename, asset_type, payload, batch_id, row_index)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(filename)
        .bind(record.asset_type)
        .bind(&record.payload)
        .bind(batch_id)
        .bind(record.row_index)
        .execute(&mut *tx)
        .await?;

        inserted += 1;
    }

    sqlx::query(
        r#"
        UPDATE upload_batches
        SET status = 'completed', records_inserted = $1, completed_at = now()
        WHERE id = $2
        "#,
    )
    .bind(inserted as i32)
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(BatchSummary {
        batch_id,
        records_inserted: inserted,
    })
}
