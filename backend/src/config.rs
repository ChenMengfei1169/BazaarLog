// Runtime configuration loaded from environment variables with sane defaults
// so the shipped BazaarLog.exe can be started by double-clicking.
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub cache_ttl_secs: u64,
    pub archive_days: i64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("BAZAARLOG_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://bazaarlog.db?mode=rwc".into()),
            host: env::var("BAZAAR LOG_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: env::var("BAZAARLOG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            cache_ttl_secs: env::var("BAZAARLOG_CACHE_TTL_SECS")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(30),
            archive_days: env::var("BAZAARLOG_ARCHIVE_DAYS")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(365),
        }
    }

    // The bundled single-machine build uses SQLite; production deployments point
    // this at a postgres:// URL.
    pub fn is_sqlite(&self) -> bool {
        self.database_url.starts_with("sqlite")
    }
}