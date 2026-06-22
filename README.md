# asset-ingest-api

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

This project now lives inside the `asset-ingest` monorepo, alongside the
`ui/` React app. `docker-compose.yml` lives at the monorepo root (one
level up from this directory), since Postgres is shared infrastructure
rather than something specific to this crate.

1. Start Postgres (from the monorepo root, not from `api/`):

   ```bash
   cd ..   # if you're inside api/
   docker compose up -d
   ```

   Wait a few seconds for the healthcheck to pass (`docker compose ps`).

2. Install `sqlx-cli` if you don't already have it (only needed if you
   want to run migrations manually — the app also runs them automatically
   on startup):

   ```bash
   cargo install sqlx-cli --no-default-features --features postgres,rustls
   ```

3. Set up your `.env` (one already exists with a generated key for local
   dev, but if you're starting fresh, copy the template and generate
   your own key):

   ```bash
   cp .env.example .env
   # then fill in API_KEY, e.g.:
   echo "API_KEY=$(openssl rand -hex 32)" >> .env
   ```

4. Build and run (from inside `api/`):

   ```bash
   cd api   # if you're at the monorepo root
   cargo run
   ```

   On first run this will take a while (compiling all dependencies). The
   app reads `.env` automatically (via `dotenvy`) from the current
   working directory — so run `cargo run` from inside `api/`, where
   `.env` lives, not from the monorepo root. It runs migrations against
   the `DATABASE_URL` there, and starts listening on port 3000.

   Startup fails loudly if `API_KEY` isn't set — that's deliberate, see
   the Authentication section below.

5. Check it's up (the health endpoint doesn't require auth):

   ```bash
   curl http://localhost:3000/health
   # -> ok
   ```

## Authentication

Every route except `/health` requires an `X-API-Key` header matching the
`API_KEY` set in `.env`. Requests without it, or with the wrong value,
get a `401`:

```bash
curl http://localhost:3000/assets/recent
# -> {"error":"missing or invalid API key"}

curl -H "X-API-Key: $(grep API_KEY .env | cut -d= -f2)" http://localhost:3000/assets/recent
# -> [...]
```

All the `curl` examples below assume you've exported your key for
convenience:

```bash
export ASSET_API_KEY=$(grep API_KEY .env | cut -d= -f2)
```

This is a single shared secret, sized for "keep this off my LAN" rather
than multi-user access control — see the Notes section below for where
this would need to evolve if more than one person/service needs distinct
credentials.

## Rate limiting

Every route except `/health` is rate-limited, keyed by API key rather
than IP address (`src/rate_limit.rs`). Defaults to 10 requests/second
sustained with a burst of 20, configurable via `RATE_LIMIT_PER_SECOND`
and `RATE_LIMIT_BURST` in `.env`.

Requests over the limit get a `429`:

```json
{ "error": "rate limit exceeded, retry after 2s" }
```

A few things worth knowing about how this is scoped:

- **Limiting runs before auth, not after.** A flood of requests with a
  missing or wrong API key is still throttled — they all share one
  "invalid" bucket rather than each bad attempt getting its own fresh
  allowance by varying the (garbage) key on every request. The `401`
  for bad auth still happens, just after the rate-limit check.
- **In-memory, per-process.** State resets on restart and isn't shared
  across multiple instances of the API. Fine for a single local process;
  would need a shared backend (e.g. Redis) to mean anything across
  multiple replicas.
- A background thread sweeps stale rate-limit buckets every 60 seconds
  (the crate's own recommended pattern), so long-running processes
  don't accumulate unbounded memory from one-off bad-auth attempts.

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
curl -H "X-API-Key: $ASSET_API_KEY" -F "file=@/tmp/sample.csv" http://localhost:3000/upload
```

Upload a JSON file and an image together in one request:

```bash
echo '{"event": "signup", "user_id": 42}' > /tmp/sample.json

curl -H "X-API-Key: $ASSET_API_KEY" \
     -F "file=@/tmp/sample.csv" \
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
curl -H "X-API-Key: $ASSET_API_KEY" http://localhost:3000/assets/recent | jq
```

### Browsing endpoints

For paginated, filterable browsing (used by the `ui/` React app in this
monorepo), two endpoints were added alongside `/assets/recent`:

```bash
# Paginated, optionally filtered by type
curl -H "X-API-Key: $ASSET_API_KEY" \
     "http://localhost:3000/assets?type=csv_row&limit=10&offset=0" | jq

# Distinct asset types currently present, with counts
curl -H "X-API-Key: $ASSET_API_KEY" http://localhost:3000/assets/types | jq
```

`GET /assets` accepts:
- `type` (optional) - filter to one asset_type; omit for all types
- `limit` (optional, default 25, max 200)
- `offset` (optional, default 0)

Response shape:
```json
{
  "assets": [ ... ],
  "total": 142,
  "limit": 25,
  "offset": 0
}
```

### Browsing UI

The `ui/` directory at the monorepo root is a React app that talks to
this API for browsing - see `../ui/README.md` for setup. This API ships
with a permissive CORS layer (`tower_http::cors::CorsLayer::permissive()`)
to support that cross-origin local dev setup; tighten this before
deploying anywhere beyond localhost.

Or go straight to Postgres (run from the monorepo root, where
`docker-compose.yml` lives):

```bash
docker compose exec postgres psql -U asset_user -d asset_ingest \
  -c "SELECT source_filename, asset_type, payload FROM ingested_assets ORDER BY created_at DESC LIMIT 5;"
```

## Notes on what's intentionally simple here

This is a starting point, not production-ready:

- **Single shared API key, not per-user/per-service auth.** Every
  authenticated request uses the same `X-API-Key` value, checked via
  constant-time comparison in `src/auth.rs`. There's no concept of
  separate credentials, scopes, or revoking one caller without rotating
  the key for everyone. Fine for "just me, locally"; if this ever needs
  to support multiple distinct clients, the natural next step is moving
  key lookup into the DB (a `api_keys` table with name/hash/revoked_at)
  rather than a single `Config::api_key` string comparison.
- **Rate limiting is in-memory and per-process** (see Rate Limiting
  section above) — it resets on restart and wouldn't coordinate across
  multiple instances if this ever ran as more than one process.
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
  auth.rs              # X-API-Key middleware (constant-time comparison)
  rate_limit.rs         # per-API-key rate limiting (tower_governor)
  db/
    mod.rs            # pool creation
    queries.rs         # insert_batch: transactional bulk insert
  ingest/
    mod.rs             # Ingestor trait + resolve_ingestor dispatch
    csv_ingestor.rs
    json_ingestor.rs
    image_ingestor.rs
  routes/
    mod.rs              # router assembly + CORS layer
    upload.rs            # /upload and /assets/recent handlers
    assets.rs              # /assets and /assets/types (filtered/paginated browsing)
migrations/
  20260618000001_init.sql
```
