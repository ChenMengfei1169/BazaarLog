// BazaarLog entry point. Loads configuration, initializes tracing, opens the
// database, bootstraps the schema, spawns the background archive task, and
// serves the embedded frontend plus the JSON API on the configured address.
mod archive;
mod audit;
mod auth;
mod cache;
mod config;
mod db;
mod encoding;
mod error;
mod excel;
mod handlers;
mod limits;
mod metrics;
mod models;
mod pow;
mod security;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use sqlx::AnyPool;
use tracing_subscriber::EnvFilter;

use crate::auth::SessionStore;
use crate::cache::Cache;
use crate::config::Config;
use crate::handlers::build_router;
use crate::security::RateLimiter;
use crate::state::AppState;

/// Sliding-window request budgets. Every limiter uses the same one-minute
/// window; only the allowance differs.
const RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
/// Generic traffic budget. Blunts bulk abuse of the read endpoints.
const GLOBAL_REQUESTS_PER_MINUTE: usize = 120;
/// Budget for the password-checking endpoints, keyed by class id plus client
/// address. Ten attempts a minute is enough for a teacher who mistypes.
const AUTH_ATTEMPTS_PER_MINUTE: usize = 10;
/// Budget for authentication keyed by client address alone. The class id in
/// the request path is attacker-chosen, so this is the bucket that actually
/// bounds how often one caller can make the server run Argon2.
const AUTH_ATTEMPTS_PER_MINUTE_PER_CLIENT: usize = 20;
/// Budget for any request carrying class credentials, each of which triggers
/// an Argon2 verification.
const CLASS_CREDENTIAL_REQUESTS_PER_MINUTE: usize = 30;
/// Budget for the audit chain verification endpoint, which hashes the whole
/// audit table on a cache miss.
const AUDIT_CHAIN_REQUESTS_PER_MINUTE: usize = 6;
/// Used to convert the configured session lifetime from hours to seconds.
const SECONDS_PER_HOUR: u64 = 3_600;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Structured logging that respects RUST_LOG; defaults to info.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    tracing::info!(
        database_url = %redact_database_url(&config.database_url),
        host = %config.host,
        port = config.port,
        is_sqlite = config.is_sqlite(),
        "starting BazaarLog"
    );

    let pool: AnyPool = db::connect(&config.database_url).await?;
    db::init_schema(&pool, config.is_sqlite()).await?;
    db::ensure_class_name_unique(&pool).await?;
    // Upgrade older databases with the audit chain columns and backfill the
    // chain for rows that predate it, so existing audit history is covered.
    db::ensure_audit_chain_columns(&pool, config.is_sqlite()).await?;
    // Drop the audit_logs.transaction_id foreign key (if an older database
    // still has it) so deleting a transaction can never rewrite historical
    // audit rows and break the chain. Runs before the backfill so the rebuilt
    // table is chained in one pass.
    db::ensure_audit_log_immutable(&pool, config.is_sqlite()).await?;
    audit::backfill_chain(&pool).await?;
    // Anchor the newest audit hash in the seal file so truncation of the audit
    // log tail stays detectable. Must run before the ACL hardening below,
    // which covers the seal file as well as the database.
    audit::refresh_seal(&pool, &config.database_url).await?;
    // Tighten the ACL of both the SQLite database file and its audit seal file
    // so other local accounts cannot read or rewrite either one: an
    // unprotected seal could be forged or deleted, which would silently
    // disable truncation detection. Best effort; see
    // db::harden_database_file_acl.
    db::harden_database_file_acl(&config.database_url);

    let state = AppState {
        pool,
        config: config.clone(),
        cache: Arc::new(Cache::new(config.cache_ttl_secs)),
        rate_limiter: RateLimiter::new(GLOBAL_REQUESTS_PER_MINUTE, RATE_LIMIT_WINDOW_SECONDS),
        auth_rate_limiter: RateLimiter::new(AUTH_ATTEMPTS_PER_MINUTE, RATE_LIMIT_WINDOW_SECONDS),
        global_auth_rate_limiter: RateLimiter::new(
            AUTH_ATTEMPTS_PER_MINUTE_PER_CLIENT,
            RATE_LIMIT_WINDOW_SECONDS,
        ),
        class_auth_rate_limiter: RateLimiter::new(
            CLASS_CREDENTIAL_REQUESTS_PER_MINUTE,
            RATE_LIMIT_WINDOW_SECONDS,
        ),
        audit_chain_rate_limiter: RateLimiter::new(
            AUDIT_CHAIN_REQUESTS_PER_MINUTE,
            RATE_LIMIT_WINDOW_SECONDS,
        ),
        // Argon2id needs ARGON2_MEMORY_KIB (64 MiB) per verification and runs
        // on the blocking pool, so the number of simultaneous verifications
        // has to be bounded or a flood of unauthenticated attempts exhausts
        // process memory.
        password_semaphore: Arc::new(tokio::sync::Semaphore::new(
            auth::MAX_CONCURRENT_PASSWORD_VERIFICATIONS,
        )),
        // A refresh or a server restart issues a new token and invalidates the
        // old one. Only token digests are stored in memory.
        sessions: SessionStore::new(config.session_ttl_hours * SECONDS_PER_HOUR),
        auth_failures: security::AuthFailureTracker::new(),
        challenges: Arc::new(pow::ChallengeStore::new()),
        // Serializes audited mutations so the audit hash chain cannot fork.
        mutation_guard: Arc::new(tokio::sync::Mutex::new(())),
    };

    // Background sweep that flips the archived flag on stale semesters.
    archive::spawn_archive_task(state.clone());

    let router = build_router(state);
    let listen_address = format!("{}:{}", config.host, config.port);
    let listener = match tokio::net::TcpListener::bind(&listen_address).await {
        Ok(listener) => listener,
        Err(error) => {
            return Err(anyhow::anyhow!(
                "{}",
                describe_bind_failure(&listen_address, &error)
            ));
        }
    };
    // Refuse plaintext HTTP on a non-loopback interface unless explicitly
    // allowed: the per-class password travels in request headers and is
    // visible on the wire, so serving it to the LAN without TLS would leak
    // credentials to anyone sniffing the network segment.
    if !is_loopback_host(&config.host) {
        if !config.allow_plaintext_lan {
            return Err(anyhow::anyhow!(
                "refusing to serve plaintext HTTP on non-loopback host '{}': the \
                 class password is sent in request headers and can be intercepted \
                 on the network. Terminate TLS in front (e.g. a reverse proxy) \
                 or set BAZAARLOG_ALLOW_PLAINTEXT_LAN=1 on a trusted LAN.",
                config.host
            ));
        }
        tracing::warn!(
            host = %config.host,
            "serving over plaintext HTTP on a non-loopback interface; the class \
             password is sent in request headers and can be intercepted on the \
             network. Terminate TLS in front (e.g. a reverse proxy) before \
             exposing BazaarLog beyond localhost."
        );
    }
    tracing::info!(
        address = %listen_address,
        "BazaarLog listening; open http://localhost:{port}",
        port = config.port
    );
    // into_make_service_with_connect_info is required so the rate limiter can
    // read the real client address from ConnectInfo; without it every request
    // collapses into a single shared bucket and the limiter is a no-op.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Turns a bind failure into an actionable message.
///
/// The raw OS error is just "access denied" or "address in use", which leaves
/// an operator unable to tell a port conflict from a port that Windows has
/// reserved. That distinction matters on Windows: Hyper-V, WSL, and Docker
/// Desktop reserve blocks of ports at boot, so a default port can start failing
/// after a reboot with no process holding it. See docs/windows7-run.md.
fn describe_bind_failure(listen_address: &str, error: &std::io::Error) -> String {
    let mut message = format!("could not listen on {listen_address}: {error}");
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => message.push_str(
            ". On Windows this is WSAEACCES (os error 10013), which means the port is \
             reserved or held exclusively rather than simply busy. Run \
             `netsh interface ipv4 show excludedportrange protocol=tcp` to see the \
             reserved ranges: Hyper-V, WSL, and Docker Desktop reserve blocks at boot \
             that can include 3000, and the ranges change between reboots. Start \
             BazaarLog on a port outside them with BAZAARLOG_PORT.",
        ),
        std::io::ErrorKind::AddrInUse => message.push_str(
            ". Another process is already listening on that port; stop it or start \
             BazaarLog on a different one with BAZAARLOG_PORT.",
        ),
        _ => {}
    }
    message
}

/// Hides the password segment of a database URL before it lands in logs.
fn redact_database_url(database_url: &str) -> String {
    let Some(scheme_end) = database_url.find("://") else {
        return database_url.to_string();
    };
    let credentials_and_host = &database_url[scheme_end + 3..];
    let Some(at_index) = credentials_and_host.find('@') else {
        return database_url.to_string();
    };
    let scheme = &database_url[..scheme_end];
    let host = &credentials_and_host[at_index..];
    format!("{scheme}://***{host}")
}

/// Addresses that never leave the local machine and are therefore safe to
/// serve over plaintext HTTP.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}
