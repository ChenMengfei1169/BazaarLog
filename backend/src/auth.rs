// Per-class password authentication. Passwords are hashed with Argon2id and
// never stored or returned in plaintext. The ClassAuth extractor verifies the
// session token supplied via headers on every mutating request; the legacy
// password-header path exists only for pre-token scripts and is disabled by
// default.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::async_trait;
use axum::extract::ConnectInfo;
use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use sqlx::Row;
use zeroize::Zeroizing;

use crate::encoding::encode_hex;
use crate::error::{AppError, AppResult};
use crate::limits::{MAX_OPERATOR_LENGTH, MAX_PASSWORD_LENGTH};
use crate::state::AppState;

// Argon2id cost parameters. 64 MiB of memory across three iterations makes
// offline cracking of a leaked hash impractical on commodity hardware while
// keeping interactive logins comfortably fast. Old hashes created with the
// previous weaker parameters keep verifying because verify_password derives
// the parameters from each hash's own PHC string.
const ARGON2_MEMORY_KIB: u32 = 65_536;
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

// A fixed, valid Argon2id hash burned against a nonsense password whenever a
// class id does not exist. The verification is wasted work but keeps response
// timing indistinguishable from a real wrong-password attempt, so an attacker
// cannot probe which class ids exist through timing.
pub const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=65536,t=3,p=1$ZHVtbXktc2FsdC0xNmJv$D2zpyf3jbEUfpFDFyLNeyHAsbP5rCT8YrFpsppR6OmE";

// Bounds the stored User-Agent so session memory stays bounded.
const MAX_USER_AGENT_LENGTH: usize = 256;

/// Percent-decodes a URI-encoded ASCII string back to UTF-8. The frontend's
/// fetch Headers rejects non-ISO-8859-1 characters, so any header value that
/// may contain non-ASCII bytes (X-Operator, X-Class-Password) is encoded via
/// encodeURIComponent before being set. This function reverses that encoding by
/// converting %XX triples back to their byte values.
fn percent_decode(encoded: &str) -> String {
    fn hex_digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = encoded.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Caps the stored User-Agent at [`MAX_USER_AGENT_LENGTH`] characters.
fn truncate_user_agent(user_agent: Option<&str>) -> Option<String> {
    user_agent.map(|agent| agent.chars().take(MAX_USER_AGENT_LENGTH).collect())
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .map_err(|error| anyhow::anyhow!("invalid argon2 parameters: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("password hash failed: {error}"))?
        .to_string();
    Ok(hash)
}

/// Verifies a password against a PHC-encoded hash. The verification derives the
/// Argon2 parameters from the hash itself, since memory, iteration, and
/// parallelism are embedded in the PHC string, so hashes created under any past
/// or future cost settings keep verifying without code changes.
///
/// This is the synchronous primitive. Handlers must go through
/// [`verify_password_bounded`] or [`hash_password_bounded`] instead, which move
/// the work off the async runtime and cap concurrency.
pub fn verify_password(password: &str, encoded: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

// Caps concurrent password work. Argon2id is deliberately memory-hard
// (ARGON2_MEMORY_KIB per verification), so the cap is what keeps a flood of
// unauthenticated attempts from exhausting process memory.
pub const MAX_CONCURRENT_PASSWORD_VERIFICATIONS: usize = 4;

// How long a request may wait for a password-work permit before giving up.
// Waiting, rather than failing immediately, keeps a burst of legitimate logins
// working: each verification takes about 100 ms, so a short queue drains
// quickly. A genuine flood still gets a 429 once the wait elapses, which bounds
// both memory and latency.
const PASSWORD_PERMIT_WAIT: Duration = Duration::from_secs(5);

/// Takes a permit from the global password-work semaphore, waiting at most
/// [`PASSWORD_PERMIT_WAIT`]. The permit is released when the returned value is
/// dropped, so callers must hold it for the duration of the hash or
/// verification.
async fn acquire_password_permit(state: &AppState) -> AppResult<tokio::sync::OwnedSemaphorePermit> {
    match tokio::time::timeout(
        PASSWORD_PERMIT_WAIT,
        state.password_semaphore.clone().acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => Ok(permit),
        // The semaphore is never closed, so a closed-permit error is
        // unreachable in practice; treat it like saturation rather than a 500.
        Ok(Err(_)) | Err(_) => Err(AppError::RateLimited),
    }
}

/// Runs one password verification on the blocking pool under the global
/// concurrency cap. Returns [`AppError::RateLimited`] when the cap stays
/// saturated for longer than [`PASSWORD_PERMIT_WAIT`].
///
/// Both the password and the encoded hash are moved in, so the caller keeps no
/// copy, and the password buffer is wrapped in [`Zeroizing`] inside the task
/// that owns it, which wipes the heap buffer as soon as verification finishes.
pub async fn verify_password_bounded(
    state: &AppState,
    password: String,
    encoded: String,
) -> AppResult<bool> {
    let permit = acquire_password_permit(state).await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let password = Zeroizing::new(password);
        verify_password(&password, &encoded)
    })
    .await
    .map_err(|error| {
        AppError::Internal(anyhow::anyhow!(
            "password verification task failed: {error}"
        ))
    })
}

/// Runs one password hash on the blocking pool under the same cap as
/// [`verify_password_bounded`]. Used by class creation, which is a public
/// endpoint.
pub async fn hash_password_bounded(state: &AppState, password: String) -> AppResult<String> {
    let permit = acquire_password_permit(state).await?;
    let hashed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let password = Zeroizing::new(password);
        hash_password(&password)
    })
    .await
    .map_err(|error| {
        AppError::Internal(anyhow::anyhow!("password hashing task failed: {error}"))
    })??;
    Ok(hashed)
}

pub async fn verify_class(state: &AppState, class_id: i64, password: String) -> AppResult<()> {
    let row = sqlx::query("SELECT password_hash FROM classes WHERE id = ?")
        .bind(class_id)
        .fetch_optional(&state.pool)
        .await?;
    // A missing class still burns one full Argon2 verification against the
    // dummy hash, so the response timing does not reveal which class ids exist.
    let encoded = match row {
        Some(row) => row.try_get::<String, _>("password_hash")?,
        None => DUMMY_PASSWORD_HASH.to_string(),
    };
    if verify_password_bounded(state, password, encoded).await? {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// Server-side session store. A successful authentication returns an opaque
/// random token bound to the class id, the operator name typed at login time,
/// the client address, and the User-Agent. Subsequent requests present the
/// token instead of the password, so the operator recorded in the audit log
/// comes from the session rather than a client-supplied X-Operator header that
/// could be spoofed by anyone who holds the password.
///
/// Security: the store persists only the SHA-256 digest of a token, never the
/// token itself, so the store can no longer be dumped to recover live
/// credentials. Every transient copy the server itself owns, whether the issued
/// token, a presented token, or a password, is wrapped in [`Zeroizing`] so its
/// heap buffer is wiped when dropped. A local process that reads this server's
/// heap may still find copies inside framework buffers it does not control; the
/// short TTL, the address and User-Agent binding, and running under a dedicated
/// low-privilege account limit what can be done with such a copy.
#[derive(Clone)]
pub struct SessionStore {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    session_ttl: Duration,
    max_sessions: usize,
}

struct SessionEntry {
    class_id: i64,
    operator: String,
    expires_at: Instant,
    // Binding hints captured at login time. A request that presents the token
    // from a different address or browser is rejected.
    client_ip: Option<IpAddr>,
    user_agent: Option<String>,
}

// Bounds memory even if many tokens are issued over the lifetime of the app.
const MAX_SESSIONS: usize = 10_000;

impl SessionStore {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            session_ttl: Duration::from_secs(ttl_secs),
            max_sessions: MAX_SESSIONS,
        }
    }

    /// Issues a new token. The plaintext token is returned to the caller once
    /// inside a [`Zeroizing`] wrapper so its heap buffer is wiped when it is
    /// dropped; only its digest is stored.
    ///
    /// Infallible by design: when the store is full the session closest to
    /// expiry is evicted rather than refusing the login. Refusing would turn a
    /// full store into a login outage for everyone, including legitimate users.
    pub fn create(
        &self,
        class_id: i64,
        operator: String,
        client_ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Zeroizing<String> {
        let mut token_bytes = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *token_bytes);
        let token: Zeroizing<String> = Zeroizing::new(encode_hex(&*token_bytes));
        let user_agent = truncate_user_agent(user_agent);
        let expires_at = Instant::now() + self.session_ttl;
        let token_digest = hash_session_token(&token);
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        // Clear expired entries before enforcing the cap.
        if sessions.len() >= self.max_sessions {
            let now = Instant::now();
            sessions.retain(|_, entry| entry.expires_at > now);
        }
        // Still at capacity: evict the soonest-to-expire session so the new
        // login always succeeds.
        while sessions.len() >= self.max_sessions {
            let Some(soonest) = sessions
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(digest, _)| digest.clone())
            else {
                break;
            };
            sessions.remove(&soonest);
        }
        sessions.insert(
            token_digest,
            SessionEntry {
                class_id,
                operator,
                expires_at,
                client_ip,
                user_agent,
            },
        );
        token
    }

    /// Resolves a presented token to its principal, checking expiry and the
    /// address and User-Agent bindings. A token presented from a different
    /// context is rejected without being revoked, so a legitimate client that
    /// simply changed networks is not logged out.
    pub fn get(
        &self,
        token: &str,
        client_ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Option<(i64, String)> {
        let token_digest = hash_session_token(token);
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        match sessions.get(&token_digest) {
            Some(entry) if entry.expires_at > now => {
                let address_matches = entry
                    .client_ip
                    .map_or(true, |bound_address| Some(bound_address) == client_ip);
                let agent_matches = entry
                    .user_agent
                    .as_deref()
                    .map_or(true, |bound_agent| Some(bound_agent) == user_agent);
                if address_matches && agent_matches {
                    Some((entry.class_id, entry.operator.clone()))
                } else {
                    None
                }
            }
            Some(_) => {
                sessions.remove(&token_digest);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) {
        let token_digest = hash_session_token(token);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&token_digest);
        }
    }
}

/// SHA-256 digest of a session token, hex-encoded. The digest is the only form
/// of a token that ever resides in server memory.
fn hash_session_token(token: &str) -> String {
    encode_hex(&Sha256::digest(token.as_bytes()))
}

/// Authenticated principal. The session token is the only accepted credential:
/// it is issued by the server after a verified login and carries the operator
/// name captured at that moment, which keeps the audit log tamper-resistant.
/// The password-header path below remains available only when
/// BAZAARLOG_ENABLE_LEGACY_AUTH=1 for scripts that predate session tokens; on
/// the default configuration any request without a session token is rejected,
/// so a client can no longer self-declare the operator name.
///
///   X-Session-Token  - opaque token issued by POST /api/classes/:id/auth
///   X-Class-Id       - (legacy) the class the caller claims to operate on
///   X-Class-Password - (legacy) that class's plaintext password, URL-encoded
///   X-Operator       - (legacy) display name recorded in audit logs
#[derive(Debug, Clone)]
pub struct ClassAuth {
    pub class_id: i64,
    pub operator: String,
}

#[async_trait]
impl FromRequestParts<AppState> for ClassAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;
        let client_ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0.ip());
        let user_agent = truncate_user_agent(
            headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok()),
        );
        // Session tokens are the primary credential. A token that is invalid or
        // expired is rejected outright instead of falling back to password
        // auth, so a stale credential can never silently downgrade.
        if let Some(token) = headers
            .get("x-session-token")
            .and_then(|value| value.to_str().ok())
        {
            // Zeroize the transient copy of the presented token after the
            // lookup so the plaintext does not linger in this request's heap.
            let token = Zeroizing::new(token.to_string());
            return match state
                .sessions
                .get(token.as_str(), client_ip, user_agent.as_deref())
            {
                Some((class_id, operator)) => Ok(ClassAuth { class_id, operator }),
                None => Err(AppError::Unauthorized),
            };
        }
        // Legacy path: password plus operator headers, URI-encoded by the
        // frontend and decoded here via percent_decode. Disabled unless
        // explicitly enabled in configuration.
        if !state.config.enable_legacy_auth {
            return Err(AppError::Unauthorized);
        }
        let class_id = headers
            .get("x-class-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or(AppError::Unauthorized)?;
        // The password is moved into verify_class below, which hands it to the
        // blocking pool and wipes the heap buffer there, so no plaintext copy is
        // left in this request's memory.
        let password = percent_decode(
            headers
                .get("x-class-password")
                .and_then(|value| value.to_str().ok())
                .unwrap_or(""),
        );
        // Reject an oversized password before spending CPU on Argon2. The header
        // is naturally size-limited but this keeps the bound explicit.
        if password.chars().count() > MAX_PASSWORD_LENGTH {
            return Err(AppError::Unauthorized);
        }
        let operator = percent_decode(
            headers
                .get("x-operator")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("anonymous"),
        );
        // Trim and cap the operator name so the audit log cannot be spammed with
        // arbitrarily long or padded display names.
        let operator: String = operator.trim().chars().take(MAX_OPERATOR_LENGTH).collect();
        // The per-class failure tracker applies to this path too, so brute force
        // through the legacy headers hits the same global lockout.
        if state.auth_failures.is_blocked(class_id) {
            return Err(AppError::RateLimited);
        }
        match verify_class(state, class_id, password).await {
            Ok(()) => {
                state.auth_failures.record_success(class_id);
                Ok(ClassAuth { class_id, operator })
            }
            Err(error) => {
                state.auth_failures.record_failure(class_id);
                Err(error)
            }
        }
    }
}
