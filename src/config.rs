#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub max_upload_bytes: usize,
    pub api_key: String,
    pub rate_limit_per_second: u64,
    pub rate_limit_burst: u32,
}

impl std::fmt::Debug for Config {
    // Manual impl (not #[derive(Debug)]) specifically to redact api_key -
    // this struct may get logged or included in error context someday,
    // and a derived Debug would print the secret in plain text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &self.database_url)
            .field("port", &self.port)
            .field("max_upload_bytes", &self.max_upload_bytes)
            .field("api_key", &"[redacted]")
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);
        let max_upload_bytes = std::env::var("MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100 * 1024 * 1024); // 100 MB default

        // Required, not optional with an "auth disabled" fallback - a
        // missing API_KEY should fail startup loudly, not silently leave
        // every route open. If you genuinely want no auth (e.g. running
        // behind something else that handles it), that should be an
        // explicit, deliberate code change, not the default behavior of
        // forgetting an env var.
        let api_key = std::env::var("API_KEY")
            .expect("API_KEY must be set - generate one with e.g. `openssl rand -hex 32`");

        // Sustained rate: one request replenished every N seconds on
        // average. Default of 10/sec is generous for a single local
        // user/script, while still stopping a runaway loop from hammering
        // the DB. Burst is how many requests can fire immediately before
        // the steady-state rate kicks in.
        let rate_limit_per_second = std::env::var("RATE_LIMIT_PER_SECOND")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);
        let rate_limit_burst = std::env::var("RATE_LIMIT_BURST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        Self {
            database_url,
            port,
            max_upload_bytes,
            api_key,
            rate_limit_per_second,
            rate_limit_burst,
        }
    }
}
