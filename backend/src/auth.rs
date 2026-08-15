// Per-class password authentication. Passwords are hashed with Argon2id and
// never stored or returned in plaintext. The ClassAuth extractor verifies the
// password supplied via headers on every mutating request.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::limits::{MAX_OPERATOR_LENGTH, MAX_PASSWORD_LENGTH};
use crate::state::AppState;

// A fixed, valid Argon2id hash burned against a nonsense password whenever a
// class id does not exist. The verification is wasted work but keeps response
// timing indistinguishable from a real (wrong-password) attempt, so an attacker
// cannot probe which class ids exist through timing.
pub const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$ZHVtbXktc2FsdC0xNmJv$AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

// Percent-decode a URI-encoded ASCII string back to UTF-8. The frontend's
// fetch Headers rejects non-ISO-8859-1 characters, so any header value that
// may contain non-ASCII bytes (X-Operator, X-Class-Password) is encoded via
// encodeURIComponent before being set. This function reverses that encoding
// by converting %XX triples back to their byte values.
fn percent_decode(s: &str) -> String {
    fn hex_digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hash failed: {e}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let parsed = match PasswordHash::new(encoded) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn verify_class(state: &AppState, class_id: i64, password: &str) -> AppResult<()> {
    let row = sqlx::query("SELECT password_hash FROM classes WHERE id = ?")
        .bind(class_id)
        .fetch_optional(&state.pool)
        .await?;
    match row {
        None => {
            // Burn equivalent Argon2 time so a missing class is not
            // distinguishable from a wrong password by timing.
            verify_password("invalid-dummy-password-burn", DUMMY_PASSWORD_HASH);
            Err(AppError::Unauthorized)
        }
        Some(r) => {
            let hash: String = r.try_get("password_hash")?;
            if verify_password(password, &hash) {
                Ok(())
            } else {
                Err(AppError::Unauthorized)
            }
        }
    }
}

// Server-side session store. A successful authentication returns an opaque
// random token bound to the class id and the operator name typed at login
// time. Subsequent requests present the token instead of the password, so the
// operator recorded in the audit log comes from the session rather than a
// client-supplied X-Operator header that could be spoofed by anyone who holds
// the password.
#[derive(Clone)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, SessionEntry>>>,
    ttl: Duration,
    max_entries: usize,
}

struct SessionEntry {
    class_id: i64,
    operator: String,
    expires_at: Instant,
}

// Bounds memory even if many tokens are issued over the lifetime of the app.
const MAX_SESSIONS: usize = 10_000;

impl SessionStore {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(ttl_secs),
            max_entries: MAX_SESSIONS,
        }
    }

    /// Issues a new token, or None when the store is at capacity.
    pub fn create(&self, class_id: i64, operator: String) -> Option<String> {
        let mut raw = [0u8; 32];
        OsRng.fill_bytes(&mut raw);
        let token: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();
        let expires_at = Instant::now() + self.ttl;
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Clear expired entries before enforcing the cap.
        if inner.len() >= self.max_entries {
            let now = Instant::now();
            inner.retain(|_, entry| entry.expires_at > now);
        }
        if inner.len() < self.max_entries {
            inner.insert(
                token.clone(),
                SessionEntry {
                    class_id,
                    operator,
                    expires_at,
                },
            );
            Some(token)
        } else {
            None
        }
    }

    pub fn get(&self, token: &str) -> Option<(i64, String)> {
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        match inner.get(token) {
            Some(entry) if entry.expires_at > now => Some((entry.class_id, entry.operator.clone())),
            Some(_) => {
                inner.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.remove(token);
        }
    }
}

// Authenticated principal. The session token is preferred: it is issued by
// the server after a verified login and carries the operator name captured at
// that moment, which keeps the audit log tamper-resistant. The password-header
// path below remains for legacy clients and scripting compatibility.
//
//   X-Session-Token - opaque token issued by POST /api/classes/:id/auth
//   X-Class-Id       - (legacy) the class the caller claims to operate on
//   X-Class-Password - (legacy) that class's plaintext password, URL-encoded
//   X-Operator       - (legacy) display name recorded in audit logs
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
        // Session tokens are the primary credential. A token that is invalid
        // or expired is rejected outright instead of falling back to password
        // auth, so a stale credential can never silently downgrade.
        if let Some(token) = headers
            .get("x-session-token")
            .and_then(|v| v.to_str().ok())
        {
            return match state.sessions.get(token) {
                Some((class_id, operator)) => Ok(ClassAuth { class_id, operator }),
                None => Err(AppError::Unauthorized),
            };
        }
        // Legacy path: password plus operator headers (URI-encoded by the
        // frontend; decoded here via percent_decode).
        let class_id = headers
            .get("x-class-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or(AppError::Unauthorized)?;
        let password = percent_decode(
            headers
                .get("x-class-password")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        );
        // Reject an oversized password before spending CPU on Argon2. The
        // header is naturally size-limited but this keeps the bound explicit.
        if password.chars().count() > MAX_PASSWORD_LENGTH {
            return Err(AppError::Unauthorized);
        }
        let operator = percent_decode(
            headers
                .get("x-operator")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("anonymous"),
        );
        // Trim and cap the operator name so the audit log cannot be spammed
        // with arbitrarily long or padded display names.
        let operator: String = operator
            .trim()
            .chars()
            .take(MAX_OPERATOR_LENGTH)
            .collect();
        verify_class(state, class_id, &password).await?;
        Ok(ClassAuth { class_id, operator })
    }
}