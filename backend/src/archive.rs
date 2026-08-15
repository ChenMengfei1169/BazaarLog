// Background archival of stale semesters. Runs once at startup and then every
// six hours. A semester is archived when its end_date is older than the
// configured archive_days window. Archived semesters remain queryable for
// reporting but are visually de-emphasized in the UI.
use chrono::{TimeDelta, Utc};

use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // Run once immediately so a fresh start reconciles state, then sweep
        // on a fixed cadence.
        loop {
            if let Err(e) = run_once(&state).await {
                tracing::warn!(error = %e, "archive sweep failed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

async fn run_once(state: &AppState) -> anyhow::Result<()> {
    let cutoff = (Utc::now() - TimeDelta::days(state.config.archive_days))
        .format("%Y-%m-%d")
        .to_string();
    let result = sqlx::query(
        "UPDATE semesters SET archived = 1 \
         WHERE archived = 0 AND end_date IS NOT NULL AND end_date <> '' AND end_date < ?",
    )
    .bind(cutoff)
    .execute(&state.pool)
    .await?;
    let affected = result.rows_affected();
    if affected > 0 {
        tracing::info!(rows_archived = affected, "archived stale semesters");
    }
    Ok(())
}
