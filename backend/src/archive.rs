// Background archival of stale semesters. Runs once at startup and then every
// six hours. A semester is archived when its end_date is older than the
// configured archive_days window. Archived semesters remain queryable for
// reporting but are visually de-emphasized in the UI.
use chrono::{TimeDelta, Utc};

use crate::state::AppState;

// How long the sweep waits between runs after its startup pass.
const SWEEP_INTERVAL_SECONDS: u64 = 6 * 60 * 60;

/// Starts the background sweep. The task lives for the lifetime of the server.
pub fn spawn_archive_task(state: AppState) {
    tokio::spawn(async move {
        // Run once immediately so a fresh start reconciles state, then sweep on
        // a fixed cadence.
        loop {
            if let Err(error) = run_archive_sweep(&state).await {
                tracing::warn!(error = %error, "archive sweep failed");
            }
            tokio::time::sleep(std::time::Duration::from_secs(SWEEP_INTERVAL_SECONDS)).await;
        }
    });
}

/// Archives every semester whose end date has fallen outside the configured
/// window and reports how many rows changed.
async fn run_archive_sweep(state: &AppState) -> anyhow::Result<()> {
    let cutoff_date = (Utc::now() - TimeDelta::days(state.config.archive_days))
        .format("%Y-%m-%d")
        .to_string();
    let result = sqlx::query(
        "UPDATE semesters SET archived = 1 \
         WHERE archived = 0 AND end_date IS NOT NULL AND end_date <> '' AND end_date < ?",
    )
    .bind(cutoff_date)
    .execute(&state.pool)
    .await?;
    let archived_rows = result.rows_affected();
    if archived_rows > 0 {
        tracing::info!(rows_archived = archived_rows, "archived stale semesters");
    }
    Ok(())
}
