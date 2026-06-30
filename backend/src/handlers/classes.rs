// Class management: list, create, password verification, and audit log view.
use axum::extract::{Path, State};
use axum::Json;
use sqlx::Row;

use crate::auth::{hash_password, ClassAuth};
use crate::error::{AppError, AppResult};
use crate::handlers::row_to_class;
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
    if body.name.trim().is_empty() || body.password.len() < 4 {
        return Err(AppError::BadRequest(
            "name must be non-empty and password at least 4 characters".into(),
        ));
    }
    let hash = hash_password(&body.password)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    let row = sqlx::query(
        "INSERT INTO classes (name, password_hash) VALUES (?, ?) \
         RETURNING id, name, created_at",
    )
    .bind(&body.name)
    .bind(&hash)
    .fetch_one(&state.pool)
    .await?;
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
    let row = sqlx::query("SELECT password_hash FROM classes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let hash: String = row
        .ok_or(AppError::Unauthorized)?
        .try_get("password_hash")?;
    if crate::auth::verify_password(&body.password, &hash) {
        Ok(Json(serde_json::json!({ "authenticated": true })))
    } else {
        Err(AppError::Unauthorized)
    }
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

