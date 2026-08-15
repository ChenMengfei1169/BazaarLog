// Semester lifecycle: list, create, and archive. All operations are class
// scoped and require ClassAuth. The path class id must match the authenticated
// class id so a session cannot operate on another class's semesters.
use axum::extract::{Path, State};
use axum::Json;
use sqlx::Row;

use crate::auth::ClassAuth;
use crate::error::{AppError, AppResult};
use crate::handlers::{lookup_semester_class, now_rfc3339, row_to_semester};
use crate::limits::{check_optional_date, check_optional_text, check_required_text, MAX_SEMESTER_NAME_LENGTH};
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
    check_required_text(&body.name, "semester name", MAX_SEMESTER_NAME_LENGTH)?;
    check_optional_text(&body.start_date, "start_date")?;
    check_optional_text(&body.end_date, "end_date")?;
    check_optional_date(&body.start_date, "start_date")?;
    check_optional_date(&body.end_date, "end_date")?;
    let row = match sqlx::query(
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
    .await
    {
        Ok(row) => row,
        Err(error) if crate::handlers::is_unique_violation(&error) => {
            return Err(AppError::Conflict(
                "a semester with this name already exists".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Json(row_to_semester(&row)?))
}

// POST /api/semesters/:id/archive - manually flips the archived flag. Looked-up
// class_id must match the authenticated class id. Like every other mutation,
// the change is committed together with an audit entry so the irreversible
// transition to read-only is attributable.
pub async fn archive_semester(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<serde_json::Value>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "UPDATE semesters SET archived = 1 WHERE id = ? AND archived = 0 \
         RETURNING id, name",
    )
    .bind(semester_id)
    .fetch_optional(&mut *tx)
    .await?;
    // The audit log only accepts the 'update' action, so the archival is
    // recorded as an update whose payload captures the new archived state.
    if let Some(row) = row {
        let name: String = row.try_get("name")?;
        crate::handlers::record_audit(
            &mut tx,
            class_id,
            None,
            "update",
            &auth.operator,
            None,
            Some(
                serde_json::json!({
                    "archived": true,
                    "semester_id": semester_id,
                    "semester_name": name,
                })
                .to_string(),
            ),
        )
        .await?;
    }
    tx.commit().await?;
    state
        .cache
        .invalidate(&crate::handlers::report_cache_key(semester_id));
    Ok(Json(serde_json::json!({ "archived": true })))
}
