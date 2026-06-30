// Reporting: aggregated dashboard and Excel export. The aggregated report is
// cached in memory for the configured TTL and invalidated on any mutation
// affecting the semester.
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use sqlx::Row;

use crate::auth::ClassAuth;
use crate::error::{AppError, AppResult};
use crate::excel::build_excel;
use crate::handlers::{lookup_semester_class, report_cache_key};
use crate::models::{ItemRanking, Report, ReportSummary};
use crate::state::AppState;

// GET /api/semesters/:id/report
pub async fn get_report(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<Report>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    let cache_key = report_cache_key(semester_id);
    if let Some(cached) = state.cache.get(&cache_key) {
        if let Ok(report) = serde_json::from_str::<Report>(&cached) {
            return Ok(Json(report));
        }
    }
    let summary = fetch_summary(&state, semester_id).await?;
    let item_ranking = fetch_item_ranking(&state, semester_id).await?;
    let report = Report {
        summary,
        item_ranking,
    };
    if let Ok(serialized) = serde_json::to_string(&report) {
        state.cache.set(cache_key, serialized);
    }
    Ok(Json(report))
}

// GET /api/semesters/:id/export.xlsx
pub async fn export_excel(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<impl IntoResponse> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    let transactions = fetch_all_transactions(&state, semester_id).await?;
    let summary = fetch_summary(&state, semester_id).await?;
    let item_ranking = fetch_item_ranking(&state, semester_id).await?;
    let report = Report {
        summary,
        item_ranking,
    };
    let bytes = build_excel(&transactions, &report)
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
    let headers = [
        (
            axum::http::header::CONTENT_TYPE,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        (
            axum::http::header::CONTENT_DISPOSITION,
            "attachment; filename=\"bazaarlog_report.xlsx\"",
        ),
    ];
    Ok((headers, bytes))
}

async fn fetch_summary(state: &AppState, semester_id: i64) -> AppResult<ReportSummary> {
    let row = sqlx::query(
        "SELECT \
         COALESCE(SUM(CASE WHEN kind = 'income'  THEN amount_cents ELSE 0 END), 0) AS income_cents, \
         COALESCE(SUM(CASE WHEN kind = 'expense' THEN amount_cents ELSE 0 END), 0) AS expense_cents, \
         COALESCE(SUM(CASE WHEN kind = 'income'  THEN 1 ELSE 0 END), 0) AS income_count, \
         COALESCE(SUM(CASE WHEN kind = 'expense' THEN 1 ELSE 0 END), 0) AS expense_count \
         FROM transactions WHERE semester_id = ?",
    )
    .bind(semester_id)
    .fetch_one(&state.pool)
    .await?;
    let total_income_cents = row.try_get::<i64, _>("income_cents")?;
    let total_expense_cents = row.try_get::<i64, _>("expense_cents")?;
    Ok(ReportSummary {
        total_income_cents,
        total_expense_cents,
        balance_cents: total_income_cents - total_expense_cents,
        income_count: row.try_get::<i64, _>("income_count")?,
        expense_count: row.try_get::<i64, _>("expense_count")?,
    })
}

async fn fetch_item_ranking(state: &AppState, semester_id: i64) -> AppResult<Vec<ItemRanking>> {
    let rows = sqlx::query(
        "SELECT \
         COALESCE(NULLIF(item, ''), '(unspecified)') AS item_name, \
         COUNT(*) AS qty, \
         COALESCE(SUM(amount_cents), 0) AS total_cents \
         FROM transactions WHERE semester_id = ? AND kind = 'income' \
         GROUP BY item_name ORDER BY qty DESC, total_cents DESC LIMIT 20",
    )
    .bind(semester_id)
    .fetch_all(&state.pool)
    .await?;
    let ranking = rows
        .iter()
        .map(|r| {
            Ok(ItemRanking {
                item: r.try_get::<String, _>("item_name")?,
                quantity: r.try_get::<i64, _>("qty")?,
                total_cents: r.try_get::<i64, _>("total_cents")?,
            })
        })
        .collect::<Result<_, AppError>>()?;
    Ok(ranking)
}

async fn fetch_all_transactions(
    state: &AppState,
    semester_id: i64,
) -> AppResult<Vec<crate::models::Transaction>> {
    let rows = sqlx::query(
        "SELECT id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at FROM transactions \
         WHERE semester_id = ? ORDER BY occurred_at, id",
    )
    .bind(semester_id)
    .fetch_all(&state.pool)
    .await?;
    rows.iter()
        .map(crate::handlers::row_to_transaction)
        .collect()
}
