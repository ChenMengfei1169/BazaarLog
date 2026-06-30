// Per-class password authentication. Passwords are hashed with Argolid and
// never stored or returned in plaintext. The ClassAuth extractor verifies the
// password supplied via headers on every mutating request.
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

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
        None => Err(AppError::Unauthorized),
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

// Authenticated principal extracted from request headers:
//   X-Class-Id       - the class the caller claims to operate on
//   X-Class-Password - that class's plaintext password (URL-encoded by the
//                      frontend to keep the header ISO-8859-1 safe; decoded
//                      here via percent_decode)
//   X-Operator       - display name recorded in audit logs (URL-encoded by the
//                      frontend; defaults to "anonymous" when absent)
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
        let class_id = headers
            .get("x-class-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or(AppError::Unauthorized)?;
        // Both password and operator are URI-encoded by the frontend.
        let password = percent_decode(
            headers
                .get("x-class-password")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        );
        let operator = percent_decode(
            headers
                .get("x-operator")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("anonymous"),
        );
        verify_class(state, class_id, &password).await?;
        Ok(ClassAuth { class_id, operator })
    }
}
