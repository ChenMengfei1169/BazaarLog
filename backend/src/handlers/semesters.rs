// Semester lifecycle: list, create, and archive. All operations are class
// scoped and require ClassAuth. The path class id must match the authenticated
// class id so a session cannot operate on another class's semesters.
use axum::extract::{Path, State};
use axum::Json;
use sqlx::Row;

use crate::auth::ClassAuth;
use crate::error::{AppError, AppResult};
use crate::handlers::{
    foreign_resource, is_unique_violation, lookup_semester_class, now_rfc3339, record_audit,
    report_cache_key, row_to_semester, AUDIT_CHAIN_CACHE_KEY,
};
use crate::limits::{
    check_optional_date, check_optional_text, check_required_text, MAX_SEMESTER_NAME_LENGTH,
};
use crate::models::{CreateSemester, Semester};
use crate::state::AppState;

/// GET /api/classes/:id/semesters
pub async fn list_semesters(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<Vec<Semester>>> {
    if auth.class_id != class_id {
        return Err(foreign_resource());
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

/// POST /api/classes/:id/semesters
pub async fn create_semester(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
    Json(body): Json<CreateSemester>,
) -> AppResult<Json<Semester>> {
    if auth.class_id != class_id {
        return Err(foreign_resource());
    }
    check_required_text(&body.name, "semester name", MAX_SEMESTER_NAME_LENGTH)?;
    check_optional_text(body.start_date.as_deref(), "start_date")?;
    check_optional_text(body.end_date.as_deref(), "end_date")?;
    check_optional_date(body.start_date.as_deref(), "start_date")?;
    check_optional_date(body.end_date.as_deref(), "end_date")?;
    // Hold the mutation guard across the write so the audit chain anchor read
    // inside record_audit cannot race another writer.
    let _mutation_guard = state.mutation_guard.lock().await;
    let mut transaction = state.pool.begin().await?;
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
    .fetch_one(&mut *transaction)
    .await
    {
        Ok(row) => row,
        Err(error) if is_unique_violation(&error) => {
            return Err(AppError::Conflict(
                "a semester with this name already exists".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let semester = row_to_semester(&row)?;
    // Semester creation is audited like every other create, so a ledger cannot
    // gain a semester without leaving a trace in the operation log.
    record_audit(
        &mut transaction,
        class_id,
        None,
        "create",
        &auth.operator,
        None,
        serde_json::to_string(&semester).ok(),
    )
    .await?;
    transaction.commit().await?;
    state.cache.invalidate(AUDIT_CHAIN_CACHE_KEY);
    crate::audit::refresh_seal(&state.pool, &state.config.database_url).await?;
    Ok(Json(semester))
}

/// POST /api/semesters/:id/archive - manually flips the archived flag. The
/// looked-up class_id must match the authenticated class id. Like every other
/// mutation, the change is committed together with an audit entry so the
/// irreversible transition to read-only is attributable.
pub async fn archive_semester(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<serde_json::Value>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(foreign_resource());
    }
    // Hold the mutation guard across the whole transaction so the audit chain
    // anchor read inside record_audit can never race another writer.
    let _mutation_guard = state.mutation_guard.lock().await;
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        "UPDATE semesters SET archived = 1 WHERE id = ? AND archived = 0 \
         RETURNING id, name",
    )
    .bind(semester_id)
    .fetch_optional(&mut *transaction)
    .await?;
    // The audit log only accepts the 'update' action, so the archival is
    // recorded as an update whose payload captures the new archived state.
    if let Some(row) = row {
        let semester_name: String = row.try_get("name")?;
        record_audit(
            &mut transaction,
            class_id,
            None,
            "update",
            &auth.operator,
            None,
            Some(
                serde_json::json!({
                    "archived": true,
                    "semester_id": semester_id,
                    "semester_name": semester_name,
                })
                .to_string(),
            ),
        )
        .await?;
    }
    transaction.commit().await?;
    state.cache.invalidate(&report_cache_key(semester_id));
    state.cache.invalidate(AUDIT_CHAIN_CACHE_KEY);
    crate::audit::refresh_seal(&state.pool, &state.config.database_url).await?;
    Ok(Json(serde_json::json!({ "archived": true })))
}
