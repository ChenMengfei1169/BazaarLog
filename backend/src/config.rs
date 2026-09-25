// Runtime configuration loaded from environment variables with defaults that
// let the shipped BazaarLog.exe start by double-clicking.
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub host: String,
    pub port: u16,
    pub cache_ttl_secs: u64,
    pub archive_days: i64,
    // Optional bearer token gating the /metrics endpoint. When unset, /metrics
    // is only reachable from loopback so internal metrics stay private on a
    // LAN-deployed instance.
    pub metrics_token: Option<String>,
    // Session lifetime in hours. Short enough to limit the damage of a stolen
    // token, long enough that a teacher is not logged out mid-lesson.
    pub session_ttl_hours: u64,
    // Proof-of-work difficulty for the public class-creation endpoint, counted
    // in leading zero hex characters that the solution digest must start with.
    // Each extra character quadruples the average work a script must spend per
    // created class.
    pub proof_of_work_difficulty: usize,
    // When false, the legacy password-header authentication path is disabled:
    // every request must present a server-issued session token, so a client
    // can no longer spoof the operator name recorded in the audit log. Set to
    // 1 only for scripts that predate session tokens.
    pub enable_legacy_auth: bool,
    // When false, the server refuses to bind to a non-loopback address over
    // plaintext HTTP, because the class password travels in request headers
    // and would be sniffable on the network. Set to 1 to run on a trusted LAN
    // without a TLS-terminating reverse proxy.
    pub allow_plaintext_lan: bool,
}

impl Config {
    /// Builds the configuration from the process environment, falling back to
    /// the single-machine defaults for every value that is unset.
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("BAZAARLOG_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://bazaarlog.db?mode=rwc".into()),
            host: env::var("BAZAARLOG_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: parse_environment_value("BAZAARLOG_PORT").unwrap_or(3000),
            cache_ttl_secs: parse_environment_value("BAZAARLOG_CACHE_TTL_SECS").unwrap_or(30),
            archive_days: parse_environment_value("BAZAARLOG_ARCHIVE_DAYS").unwrap_or(365),
            metrics_token: env::var("BAZAARLOG_METRICS_TOKEN")
                .ok()
                .filter(|value| !value.is_empty()),
            session_ttl_hours: parse_environment_value("BAZAARLOG_SESSION_TTL_HOURS").unwrap_or(4),
            proof_of_work_difficulty: parse_environment_value("BAZAARLOG_POW_DIFFICULTY")
                .unwrap_or(4),
            enable_legacy_auth: read_boolean_flag("BAZAARLOG_ENABLE_LEGACY_AUTH"),
            allow_plaintext_lan: read_boolean_flag("BAZAARLOG_ALLOW_PLAINTEXT_LAN"),
        }
    }

    /// True when the connection string selects the bundled SQLite backend. The
    /// single-machine build uses SQLite; production deployments point
    /// BAZAARLOG_DATABASE_URL at a postgres:// URL instead.
    pub fn is_sqlite(&self) -> bool {
        self.database_url.starts_with("sqlite")
    }
}

/// Reads an environment variable and parses it, returning None when the
/// variable is unset or malformed so the caller's default applies.
fn parse_environment_value<T: std::str::FromStr>(name: &str) -> Option<T> {
    env::var(name).ok()?.parse().ok()
}

/// Reads a boolean environment flag. Only the exact value "1" enables it.
fn read_boolean_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| value == "1")
}
