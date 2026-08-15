// Class management: list, create, password verification, and audit log view.
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use sqlx::Row;

use crate::auth::{hash_password, verify_password, ClassAuth, DUMMY_PASSWORD_HASH};
use crate::error::{AppError, AppResult};
use crate::handlers::row_to_class;
use crate::limits::{
    check_required_text, MAX_CLASS_NAME_LENGTH, MAX_OPERATOR_LENGTH, MAX_PASSWORD_LENGTH,
    MIN_PASSWORD_LENGTH,
};
use crate::models::{AuditLog, AuthClass, CreateClass};
use crate::state::AppState;

// GET /api/classes - public; returns the list of class names so the switcher
// can populate its dropdown before the user has authenticated.
pub async fn list_classes(State(state): State<AppState>) -> AppResult<Json<Vec<crate::models::Class>>> {
    let rows = sqlx::query("SELECT id, name, created_at FROM classes ORDER BY id")
        .fetch_all(&state.pool)
        .await?;
    let classes = rows.iter().map(row_to_class).collect::<Result<_, _>>()?;
    Ok(Json(classes))
}

// POST /api/classes - public; creates a class with an Argolid password hash.
pub async fn create_class(
    State(state): State<AppState>,
    Json(body): Json<CreateClass>,
) -> AppResult<Json<crate::models::Class>> {
    check_required_text(&body.name, "class name", MAX_CLASS_NAME_LENGTH)?;
    // Normalize surrounding whitespace so "Final Class" and "Final Class "
    // cannot exist side by side as two indistinguishable dropdown entries.
    let class_name = body.name.trim();
    // Reject duplicate names; a hard UNIQUE index is not added because it
    // would fail on databases that already contain duplicates.
    let existing_class = sqlx::query("SELECT id FROM classes WHERE name = ?")
        .bind(class_name)
        .fetch_optional(&state.pool)
        .await?;
    if existing_class.is_some() {
        return Err(AppError::Conflict(
            "a class with this name already exists".into(),
        ));
    }
    // Count characters, not bytes, so multi-byte names are bounded correctly.
    let password_length = body.password.chars().count();
    if !(MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&password_length) {
        return Err(AppError::BadRequest(format!(
            "password must be {MIN_PASSWORD_LENGTH} to {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    let hash = hash_password(&body.password)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    let row = match sqlx::query(
        "INSERT INTO classes (name, password_hash) VALUES (?, ?) \
         RETURNING id, name, created_at",
    )
    .bind(class_name)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await
    {
        Ok(row) => row,
        Err(error) if crate::handlers::is_unique_violation(&error) => {
            return Err(AppError::Conflict(
                "a class with this name already exists".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Json(row_to_class(&row)?))
}

// POST /api/classes/:id/auth - public; returns 200 with a session marker when
// the password matches, 401 otherwise. The frontend stores the password in
// memory and supplies it via headers on subsequent class-scoped requests.
pub async fn auth_class(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<AuthClass>,
) -> AppResult<Json<serde_json::Value>> {
    // Bound the password length before running the expensive Argon2
    // verification so an attacker cannot reach the hasher with an oversized
    // password and exhaust server CPU/memory. The create path enforces the
    // same upper bound, so no legitimate password exceeds it.
    if body.password.chars().count() > MAX_PASSWORD_LENGTH {
        return Err(AppError::BadRequest(format!(
            "password may not exceed {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    let row = sqlx::query("SELECT password_hash FROM classes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let hash: String = match row {
        Some(row) => row.try_get("password_hash")?,
        None => {
            // Burn equivalent Argon2 time so a missing class id is not
            // distinguishable from a wrong password through response timing.
            verify_password("invalid-dummy-password-burn", DUMMY_PASSWORD_HASH);
            return Err(AppError::Unauthorized);
        }
    };
    if verify_password(&body.password, &hash) {
        // Bind the typed operator name to a server-side session token so the
        // audit log records it authoritatively instead of trusting a header.
        let operator: String = body
            .operator
            .as_deref()
            .unwrap_or("anonymous")
            .trim()
            .chars()
            .take(MAX_OPERATOR_LENGTH)
            .collect();
        let operator = if operator.is_empty() {
            "anonymous".to_string()
        } else {
            operator
        };
        let token = state
            .sessions
            .create(id, operator)
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("session store at capacity")))?;
        Ok(Json(serde_json::json!({
            "authenticated": true,
            "token": token,
        })))
    } else {
        Err(AppError::Unauthorized)
    }
}

// POST /api/logout - revokes the presented session token. Does not require a
// valid ClassAuth because a stale or expired token must still be discardable.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    if let Some(token) = headers
        .get("x-session-token")
        .and_then(|value| value.to_str().ok())
    {
        state.sessions.revoke(token);
    }
    Ok(Json(serde_json::json!({ "logged_out": true })))
}

// GET /api/classes/:id/audit_logs - returns the most recent operation log
// entries for the class. Requires ClassAuth so cross-class visibility stays
// blocked even if a stale URL is replayed.
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<Vec<AuditLog>>> {
    if auth.class_id != class_id {
        return Err(AppError::Unauthorized);
    }
    let rows = sqlx::query(
        "SELECT id, transaction_id, class_id, action, operator, payload_before, \
         payload_after, occurred_at FROM audit_logs \
         WHERE class_id = ? ORDER BY occurred_at DESC, id DESC LIMIT 200",
    )
    .bind(class_id)
    .fetch_all(&state.pool)
    .await?;
    let logs = rows
        .iter()
        .map(|r| {
            Ok(AuditLog {
                id: r.try_get("id")?,
                transaction_id: crate::handlers::optional_i64(r, "transaction_id")?,
                class_id: crate::handlers::optional_i64(r, "class_id")?,
                action: r.try_get("action")?,
                operator: r.try_get("operator")?,
                payload_before: crate::handlers::optional_string(r, "payload_before")?,
                payload_after: crate::handlers::optional_string(r, "payload_after")?,
                occurred_at: r.try_get("occurred_at")?,
            })
        })
        .collect::<Result<_, AppError>>()?;
    Ok(Json(logs))
}

