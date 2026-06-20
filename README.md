# Rust Data Pipeline

A small Rust/axum/Postgres service that accepts file uploads (CSV, JSON,
images) over HTTP, parses them per-type, and stores the results as JSONB
rows in Postgres.

## Architecture

- **`Ingestor` trait** (`src/ingest/mod.rs`) — one implementation per asset
  type (`csv_ingestor.rs`, `json_ingestor.rs`, `image_ingestor.rs`). Adding
  a new asset type means writing a new file here and one match arm in
  `resolve_ingestor` — nothing else changes.
- **Storage model** — single `ingested_assets` table with a `payload JSONB`
  column and an `asset_type` discriminator, plus an `upload_batches` table
  tracking each upload. This is deliberately flexible while you're still
  exploring what shapes your data takes. Once shapes stabilize for a given
  type, consider giving it a dedicated typed table.
- **Per-file isolation** — a multipart request can contain several files;
  one failing file doesn't fail the others. The response reports per-file
  status.

## Prerequisites

- Rust (stable) + Cargo — install via [rustup](https://rustup.rs) if you
  don't have it: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Docker + Docker Compose (for local Postgres)

## Running locally

1. Start Postgres:

   ```bash
   docker compose up -d
   ```

   Wait a few seconds for the healthcheck to pass (`docker compose ps`).

2. Install `sqlx-cli` if you don't already have it (only needed if you
   want to run migrations manually — the app also runs them automatically
   on startup):

   ```bash
   cargo install sqlx-cli --no-default-features --features postgres,rustls
   ```

3. Build and run:

   ```bash
   cargo run
   ```

   On first run this will take a while (compiling all dependencies). The
   app reads `.env` automatically (via `dotenvy`), runs migrations against
   the `DATABASE_URL` there, and starts listening on port 3000.

4. Check it's up:

   ```bash
   curl http://localhost:3000/health
   # -> ok
   ```

## Trying it out

Create a sample CSV:

```bash
cat > /tmp/sample.csv <<'EOF'
name,age,city
Alice,30,Seattle
Bob,25,Austin
EOF
```

Upload it:

```bash
curl -F "file=@/tmp/sample.csv" http://localhost:3000/upload
```

Upload a JSON file and an image together in one request:

```bash
echo '{"event": "signup", "user_id": 42}' > /tmp/sample.json

curl -F "file=@/tmp/sample.csv" \
     -F "file=@/tmp/sample.json" \
     -F "file=@/path/to/photo.jpg" \
     http://localhost:3000/upload
```

Each call returns a JSON summary like:

```json
{
  "files": [
    { "filename": "sample.csv", "status": "ok", "asset_type": "csv_row",
      "batch_id": "...", "records_inserted": 2, "error": null }
  ]
}
```

Check what landed in the DB without leaving HTTP:

```bash
curl http://localhost:3000/assets/recent | jq
```

Or go straight to Postgres:

```bash
docker compose exec postgres psql -U asset_user -d asset_ingest \
  -c "SELECT source_filename, asset_type, payload FROM ingested_assets ORDER BY created_at DESC LIMIT 5;"
```

## Notes on what's intentionally simple here

This is a starting point, not production-ready:

- **No auth.** Add a middleware layer (e.g. API key header check) before
  exposing this beyond localhost.
- **Plain `sqlx::query`, not `sqlx::query!`.** The macro form checks SQL
  against your live schema at compile time, which is great once your
  schema stabilizes — but it requires `DATABASE_URL` to be reachable at
  `cargo build` time, or a checked-in `.sqlx` query cache (`cargo sqlx
  prepare`) for CI/offline builds. Worth switching to once you're past
  the experimentation phase.
- **No retry/backoff** on the DB connection at startup — if Postgres
  isn't ready yet, the app will just fail to start. Fine locally; for
  production, add a connect-retry loop or rely on your orchestrator's
  restart policy.
- **CSV/JSON fields all land as strings/JSON values** without further
  type coercion (e.g. CSV numeric columns stay as JSON strings, not
  numbers). Worth revisiting once you know what queries you'll run
  against this data — JSONB lets you defer that decision.
- **Image ingestor only extracts EXIF.** No thumbnailing, no actual
  image bytes stored — only metadata, per the original ask.

## Project layout

```
src/
  main.rs            # bootstrap: config, db pool, migrations, router, serve
  config.rs           # env-based config struct
  error.rs            # AppError (HTTP-facing) + IngestError (parser-facing)
  db/
    mod.rs            # pool creation
    queries.rs         # insert_batch: transactional bulk insert
  ingest/
    mod.rs             # Ingestor trait + resolve_ingestor dispatch
    csv_ingestor.rs
    json_ingestor.rs
    image_ingestor.rs
  routes/
    mod.rs              # router assembly
    upload.rs            # /upload and /assets/recent handlers
migrations/
  20260618000001_init.sql
```
