// Shared application state cheaply cloned into every request handler.
use std::sync::Arc;

use sqlx::AnyPool;

use crate::{auth::SessionStore, cache::Cache, config::Config, security::RateLimiter};

#[derive(Clone)]
pub struct AppState {
    pub pool: AnyPool,
    pub config: Config,
    pub cache: Arc<Cache>,
    // Sliding-window rate limiters. The global limiter blunts generic traffic
    // abuse; the stricter limiter protects password-checking endpoints from
    // brute forcing; the class-auth limiter throttles every request that
    // carries the class password header (each runs an Argon2 verification).
    pub rate_limiter: RateLimiter,
    pub auth_rate_limiter: RateLimiter,
    pub class_auth_rate_limiter: RateLimiter,
    // Server-side login sessions binding a class id to the operator name.
    pub sessions: SessionStore,
}
