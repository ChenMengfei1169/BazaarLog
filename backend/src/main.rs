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
mod metrics;
mod models;
mod state;

use std::sync::Arc;

use sqlx::AnyPool;
use tracing_subscriber::EnvFilter;

use crate::cache::Cache;
use crate::config::Config;
use crate::handlers::build_router;
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

    let state = AppState {
        pool,
        config: config.clone(),
        cache: Arc::new(Cache::new(config.cache_ttl_secs)),
    };

    // Background sweep that flips the archived flag on stale semesters.
    archive::spawn(state.clone());

    let app = build_router(state);
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(address = %addr, "BazaarLog listening; open http://localhost:{port}", port = config.port);
    axum::serve(listener, app).await?;
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
