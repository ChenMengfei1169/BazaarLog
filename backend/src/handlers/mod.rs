// Handler module root: wires routes, shared row mappers, and the audit helper.
use axum::Json;
use axum::routing::{get, post};
use sqlx::any::AnyRow;
use sqlx::Row;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::error::{AppError, AppResult};
use crate::metrics;
use crate::models::{Class, Semester, Transaction};
use crate::state::AppState;

pub mod classes;
pub mod reports;
pub mod semesters;
pub mod static_assets;
pub mod transactions;

pub fn build_router(state: AppState) -> axum::Router<()> {
    axum::Router::new()
        .route("/api/health", get(health))
        .route(
            "/api/classes",
            get(classes::list_classes).post(classes::create_class),
        )
        .route("/api/classes/:id/auth", post(classes::auth_class))
        .route("/api/classes/:id/audit_logs", get(classes::list_audit_logs))
        .route(
            "/api/classes/:id/semesters",
            get(semesters::list_semesters).post(semesters::create_semester),
        )
        .route("/api/semesters/:id/archive", post(semesters::archive_semester))
        .route(
            "/api/semesters/:id/transactions",
            get(transactions::list_transactions).post(transactions::create_transaction),
        )
        .route("/api/semesters/:id/report", get(reports::get_report))
        .route("/api/semesters/:id/export.xlsx", get(reports::export_excel))
        .route(
            "/api/transactions/:id",
            get(transactions::get_transaction)
                .put(transactions::update_transaction)
                .delete(transactions::delete_transaction),
        )
        .route("/metrics", get(metrics::metrics_handler))
        .fallback(static_assets::static_handler)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(metrics::metrics_middleware))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

// Records an audit entry on the same transaction as the mutating write so the
// operation log is committed atomically with the data change. Callers pass a
// borrowed connection via `&mut *tx` so this stays composable with other
// statements inside the same database transaction.
pub async fn record_audit(
    executor: &mut sqlx::AnyConnection,
    class_id: i64,
    transaction_id: Option<i64>,
    action: &str,
    operator: &str,
    before: Option<String>,
    after: Option<String>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_logs \
         (transaction_id, class_id, action, operator, payload_before, payload_after) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(transaction_id)
    .bind(class_id)
    .bind(action)
    .bind(operator)
    .bind(before)
    .bind(after)
    .execute(executor)
    .await?;
    Ok(())
}

pub fn report_cache_key(semester_id: i64) -> String {
    format!("report:semester:{semester_id}")
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn row_to_class(row: &AnyRow) -> AppResult<Class> {
    Ok(Class {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
    })
}

pub fn row_to_semester(row: &AnyRow) -> AppResult<Semester> {
    // PostgreSQL maps SMALLINT to i16, SQLite maps INTEGER to i64.
    // Try i16 first (PostgreSQL); fall back to i64 (SQLite).
    let archived_int: i64 = row
        .try_get::<i16, _>("archived")
        .map(|v| v as i64)
        .or_else(|_| row.try_get::<i64, _>("archived"))?;
    Ok(Semester {
        id: row.try_get("id")?,
        class_id: row.try_get("class_id")?,
        name: row.try_get("name")?,
        archived: archived_int != 0,
        start_date: optional_string(row, "start_date")?,
        end_date: optional_string(row, "end_date")?,
        created_at: row.try_get("created_at")?,
    })
}

pub fn row_to_transaction(row: &AnyRow) -> AppResult<Transaction> {
    Ok(Transaction {
        id: row.try_get("id")?,
        semester_id: row.try_get("semester_id")?,
        class_id: row.try_get("class_id")?,
        kind: row.try_get("kind")?,
        amount_cents: row.try_get("amount_cents")?,
        source: optional_string(row, "source")?,
        purpose: optional_string(row, "purpose")?,
        item: optional_string(row, "item")?,
        operator: row.try_get("operator")?,
        occurred_at: row.try_get("occurred_at")?,
        created_at: row.try_get("created_at")?,
    })
}

// SQLX::Any cannot decode SQL NULL into Option<T> due to a type-info mismatch
// in the Any adapter (the column type is reported as NULL, which Option<T>'s
// compatible check does not accept on 0.7.x). These helpers swallow the
// resulting ColumnDecode error and return None, which is the only case that
// produces a ColumnDecode error for these TEXT/INTEGER columns.
pub fn optional_string(row: &AnyRow, col: &str) -> AppResult<Option<String>> {
    row.try_get::<Option<String>, _>(col)
        .or_else(|e| match e {
            sqlx::Error::ColumnDecode { .. } => Ok(None),
            _ => Err(e.into()),
        })
}

pub fn optional_i64(row: &AnyRow, col: &str) -> AppResult<Option<i64>> {
    row.try_get::<Option<i64>, _>(col)
        .or_else(|e| match e {
            sqlx::Error::ColumnDecode { .. } => Ok(None),
            _ => Err(e.into()),
        })
}

/// Looks up the class_id for a semester. Shared across handler submodules.
pub async fn lookup_semester_class(state: &AppState, semester_id: i64) -> AppResult<i64> {
    let row = sqlx::query("SELECT class_id FROM semesters WHERE id = ?")
        .bind(semester_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(row.try_get::<i64, _>("class_id")?)
}
