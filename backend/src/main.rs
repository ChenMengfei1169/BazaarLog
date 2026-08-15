// BazaarLog entry point. Loads configuration, initializes tracing, opens the
// database, bootstraps the schema, spawns the background archive task, and
// serves the embedded frontend plus the JSON API on the configured address.
mod archive;
mod auth;
mod cache;
mod config;
mod db;
mod error;
mod excel;
mod handlers;
mod limits;
mod metrics;
mod models;
mod security;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use sqlx::AnyPool;
use tracing_subscriber::EnvFilter;

use crate::cache::Cache;
use crate::config::Config;
use crate::handlers::build_router;
use crate::auth::SessionStore;
use crate::security::RateLimiter;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize structured logging. Respects RUST_LOG; defaults to info.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = Config::from_env();
    tracing::info!(
        database_url = %redact_url(&config.database_url),
        host = %config.host,
        port = config.port,
        is_sqlite = config.is_sqlite(),
        "starting BazaarLog"
    );

    let pool: AnyPool = db::connect(&config.database_url).await?;
    db::init_schema(&pool, config.is_sqlite()).await?;
    db::ensure_class_name_unique(&pool).await?;

    let state = AppState {
        pool,
        config: config.clone(),
        cache: Arc::new(Cache::new(config.cache_ttl_secs)),
        // Global limiter: 120 requests/min per client. Auth limiter: 10 password
        // attempts/min so brute forcing the /auth endpoint stalls while
        // legitimate classroom logins are not blocked.
        rate_limiter: RateLimiter::new(120, 60),
        auth_rate_limiter: RateLimiter::new(10, 60),
        // ClassAuth-protected requests verify the password with Argon2, so
        // they get a budget tighter than the global one but looser than the
        // explicit auth endpoints to avoid blocking normal browsing.
        class_auth_rate_limiter: RateLimiter::new(30, 60),
        // Login sessions last 12 hours; a refresh (or server restart) issues
        // a new token and the old one becomes invalid.
        sessions: SessionStore::new(12 * 60 * 60),
    };

    // Background sweep that flips the archived flag on stale semesters.
    archive::spawn(state.clone());

    let app = build_router(state);
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // Warn before serving over plaintext HTTP on a non-loopback interface: the
    // per-class password travels in request headers and is visible on the wire.
    if !is_loopback_host(&config.host) {
        tracing::warn!(
            host = %config.host,
            "serving over plaintext HTTP on a non-loopback interface; the class \
             password is sent in request headers and can be intercepted on the \
             network. Terminate TLS in front (e.g. a reverse proxy) before \
             exposing BazaarLog beyond localhost."
        );
    }
    tracing::info!(address = %addr, "BazaarLog listening; open http://localhost:{port}", port = config.port);
    // into_make_service_with_connect_info is required so the rate limiter can
    // read the real client IP from ConnectInfo; without it every request
    // collapses into a single shared bucket and the limiter is a no-op.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// Hides the password segment of a database URL before it lands in logs.
fn redact_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after = &url[scheme_end + 3..];
        if let Some(at) = after.find('@') {
            let (scheme, _) = url.split_at(scheme_end);
            let (_, host) = after.split_at(at);
            return format!("{scheme}://***{host}");
        }
    }
    url.to_string()
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}