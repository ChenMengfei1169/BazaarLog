// Shared application state cheaply cloned into every request handler.
use std::sync::Arc;

use sqlx::AnyPool;

use crate::auth::SessionStore;
use crate::cache::Cache;
use crate::config::Config;
use crate::pow::ChallengeStore;
use crate::security::{AuthFailureTracker, RateLimiter};

#[derive(Clone)]
pub struct AppState {
    pub pool: AnyPool,
    pub config: Config,
    pub cache: Arc<Cache>,
    // Sliding-window rate limiters. The global limiter blunts generic traffic
    // abuse; the stricter limiter protects password-checking endpoints from
    // brute forcing; the class-auth limiter throttles every request that
    // carries the class password header, each of which runs an Argon2
    // verification.
    pub rate_limiter: RateLimiter,
    pub auth_rate_limiter: RateLimiter,
    // Keyed by source address only. The auth limiter above is keyed by class id
    // plus address, so an attacker who varies the class id in the URL would
    // otherwise get a fresh budget for every id; this bucket cannot be reset
    // that way.
    pub global_auth_rate_limiter: RateLimiter,
    pub class_auth_rate_limiter: RateLimiter,
    // Guards the audit chain verification endpoint, which walks the whole
    // audit table and recomputes a SHA-256 per row.
    pub audit_chain_rate_limiter: RateLimiter,
    // Caps how many password verifications and hashes run at once. Each
    // Argon2id verification needs ARGON2_MEMORY_KIB (64 MiB) and runs on the
    // blocking pool, so an unbounded number of in-flight verifications is a
    // memory exhaustion vector even with the Argon2 call off the async runtime.
    pub password_semaphore: Arc<tokio::sync::Semaphore>,
    // Server-side login sessions binding a class id to the operator name. Only
    // SHA-256 digests of tokens are stored in memory.
    pub sessions: SessionStore,
    // Global per-class failed-login counter; independent of source address so
    // address rotation cannot bypass the brute-force lockout.
    pub auth_failures: AuthFailureTracker,
    // Single-use proof-of-work challenges for class creation.
    pub challenges: Arc<ChallengeStore>,
    // Serializes audited mutations so the audit hash chain cannot fork when
    // two writers commit concurrently.
    pub mutation_guard: Arc<tokio::sync::Mutex<()>>,
}
