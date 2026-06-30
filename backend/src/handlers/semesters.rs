// Semester lifecycle: list, create, and archive. All operations are class
// scoped and require ClassAuth. The path class id must match the authenticated
// class id so a session cannot operate on another class's semesters.
use axum::extract::{Path, State};
use axum::Json;

use crate::auth::ClassAuth;
use crate::error::{AppError, AppResult};
use crate::handlers::{lookup_semester_class, now_rfc3339, row_to_semester};
use crate::models::CreateSemester;
use crate::state::AppState;

// GET /api/classes/:id/semesters
pub async fn list_semesters(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<Vec<crate::models::Semester>>> {
    if auth.class_id != class_id {
        return Err(AppError::Unauthorized);
    }
    let rows = sqlx::query(
        "SELECT id, class_id, name, archived, start_date, end_date, created_at \
         FROM semesters WHERE class_id = ? ORDER BY id",
    )
    .bind(class_id)
    .fetch_all(&state.pool)
    .await?;
    let semesters = rows.iter().map(row_to_semester).collect::<Result<_, _>>()?;
    Ok(Json(semesters))
}

// POST /api/classes/:id/semesters
pub async fn create_semester(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
    Json(body): Json<CreateSemester>,
) -> AppResult<Json<crate::models::Semester>> {
    if auth.class_id != class_id {
        return Err(AppError::Unauthorized);
    }
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("semester name is required".into()));
    }
    let row = sqlx::query(
        "INSERT INTO semesters (class_id, name, archived, start_date, end_date, created_at) \
         VALUES (?, ?, 0, ?, ?, ?) \
         RETURNING id, class_id, name, archived, start_date, end_date, created_at",
    )
    .bind(class_id)
    .bind(&body.name)
    .bind(&body.start_date)
    .bind(&body.end_date)
    .bind(now_rfc3339())
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(row_to_semester(&row)?))
}

// POST /api/semesters/:id/archive - manually flips the archived flag. Looked-up
// class_id must match the authenticated class id.
pub async fn archive_semester(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<serde_json::Value>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    sqlx::query("UPDATE semesters SET archived = 1 WHERE id = ?")
        .bind(semester_id)
        .execute(&state.pool)
        .await?;
    state
        .cache
        .invalidate(&crate::handlers::report_cache_key(semester_id));
    Ok(Json(serde_json::json!({ "archived": true })))
}
