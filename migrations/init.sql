CREATE TABLE IF NOT EXISTS ingested_assets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    source_filename TEXT NOT NULL,
    asset_type TEXT NOT NULL,           -- 'csv_row' | 'json' | 'image_metadata'
    payload JSONB NOT NULL,             -- the actual parsed content
    batch_id UUID NOT NULL,             -- groups rows from the same upload/file
    row_index INTEGER,                  -- position within the source file, if applicable
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_ingested_assets_batch_id ON ingested_assets (batch_id);
CREATE INDEX IF NOT EXISTS idx_ingested_assets_asset_type ON ingested_assets (asset_type);
CREATE INDEX IF NOT EXISTS idx_ingested_assets_payload_gin ON ingested_assets USING GIN (payload);

CREATE TABLE IF NOT EXISTS upload_batches (
    id UUID PRIMARY KEY,
    filename TEXT NOT NULL,
    asset_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'processing', -- 'processing' | 'completed' | 'failed'
    records_inserted INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);
