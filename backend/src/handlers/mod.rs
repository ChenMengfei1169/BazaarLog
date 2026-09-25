// Handler module root: wires routes, shared row mappers, and the audit helper.
use axum::routing::{get, post};
use axum::Json;
use sqlx::any::AnyRow;
use sqlx::Row;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::error::{AppError, AppResult};
use crate::metrics;
use crate::models::{Class, Semester, Transaction};
use crate::security;
use crate::state::AppState;

pub mod classes;
pub mod reports;
pub mod semesters;
pub mod static_assets;
pub mod transactions;

/// Upper bound on a request body, applied before any handler sees it.
const MAX_REQUEST_BODY_BYTES: usize = 1_048_576;
/// Upper bound on how long a single request may run, so a stalled client or a
/// pathological query cannot hold a connection open indefinitely.
const REQUEST_TIMEOUT_SECONDS: u64 = 60;

/// Cache key for the audit chain verification report. The chain is global, one
/// chain across every class, so there is a single key rather than one per class,
/// and it is invalidated by every audited mutation.
pub const AUDIT_CHAIN_CACHE_KEY: &str = "audit:chain";

/// How long the expensive full-chain scan result may be cached. Deliberately
/// much shorter than the default cache TTL (BAZAARLOG_CACHE_TTL_SECS): the chain
/// report is a tamper-detection signal, and any caching of it directly becomes
/// detection latency for out-of-band edits to the audit table. The cost is
/// bounded by the dedicated rate limiter on the endpoint, not by this value.
///
/// Note this only covers the internal-chain scan. The external seal check is
/// O(1) and runs on every request regardless of this cache; see
/// [`classes::verify_audit_chain`].
pub const AUDIT_CHAIN_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

pub fn build_router(state: AppState) -> axum::Router<()> {
    axum::Router::new()
        .route("/api/health", get(health))
        .route(
            "/api/classes",
            get(classes::list_classes).post(classes::create_class),
        )
        // Public proof-of-work challenge required by class creation.
        .route("/api/classes/challenge", get(classes::get_create_challenge))
        .route("/api/classes/:id/auth", post(classes::auth_class))
        .route("/api/classes/:id/audit_logs", get(classes::list_audit_logs))
        // Verifies the tamper-evident audit hash chain.
        .route(
            "/api/classes/:id/audit_logs/chain",
            get(classes::verify_audit_chain),
        )
        .route("/api/logout", post(classes::logout))
        .route(
            "/api/classes/:id/semesters",
            get(semesters::list_semesters).post(semesters::create_semester),
        )
        .route(
            "/api/semesters/:id/archive",
            post(semesters::archive_semester),
        )
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
        // Layers are applied innermost first, so the layer added last is the
        // outermost and wraps every response produced by the layers inside it,
        // including the 429 replies from the rate limiter and the 408 from the
        // timeout.
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(metrics::metrics_middleware))
        // Per-address rate limiting; the strict budget guards the
        // password-checking endpoints. State is bound here because the
        // middleware extracts AppState before the router's final
        // .with_state() runs.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security::apply_rate_limit,
        ))
        // Explicit cross-origin mutation rejection (Origin versus Host check).
        .layer(axum::middleware::from_fn(
            security::reject_cross_origin_mutations,
        ))
        // Reject oversized request bodies before they reach a handler.
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        // Bound how long any single request may run.
        .layer(tower_http::timeout::TimeoutLayer::new(
            std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
        ))
        // Defensive headers outermost so they cover every response, including
        // the error replies produced by the layers above.
        .layer(axum::middleware::from_fn(security::add_security_headers))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// Records an audit entry on the same transaction as the mutating write, so the
/// operation log is committed atomically with the data change. Callers pass a
/// borrowed connection via `&mut *transaction` so this stays composable with
/// other statements inside the same database transaction.
///
/// Every entry is chained to the previous one: the row stores the previous row's
/// entry_hash as prev_hash, and its own entry_hash commits the full content,
/// including prev_hash and the autoincrement id, to SHA-256. Editing, deleting,
/// or inserting any row breaks every subsequent link, so tampering is detectable
/// via [`crate::audit::verify_chain`]. Callers must hold
/// AppState.mutation_guard for the whole transaction so the anchor read can never
/// race another writer.
pub async fn record_audit(
    executor: &mut sqlx::AnyConnection,
    class_id: i64,
    transaction_id: Option<i64>,
    action: &str,
    operator: &str,
    payload_before: Option<String>,
    payload_after: Option<String>,
) -> AppResult<()> {
    let occurred_at = now_rfc3339();
    let prev_hash = match sqlx::query("SELECT entry_hash FROM audit_logs ORDER BY id DESC LIMIT 1")
        .fetch_optional(&mut *executor)
        .await?
    {
        Some(row) => row.try_get::<String, _>("entry_hash")?,
        None => crate::audit::AUDIT_CHAIN_GENESIS.to_string(),
    };
    // The entry_hash commits the autoincrement id, so it is computed after the
    // insert via RETURNING and stored with a follow-up update inside the same
    // transaction.
    let inserted = sqlx::query(
        "INSERT INTO audit_logs \
         (transaction_id, class_id, action, operator, payload_before, payload_after, \
         occurred_at, prev_hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(transaction_id)
    .bind(class_id)
    .bind(action)
    .bind(operator)
    .bind(&payload_before)
    .bind(&payload_after)
    .bind(&occurred_at)
    .bind(&prev_hash)
    .fetch_one(&mut *executor)
    .await?;
    let id: i64 = inserted.try_get("id")?;
    let entry_hash = crate::audit::compute_entry_hash(
        &prev_hash,
        &crate::audit::AuditChainRow {
            id,
            class_id,
            transaction_id,
            action,
            operator,
            payload_before: payload_before.as_deref(),
            payload_after: payload_after.as_deref(),
            occurred_at: &occurred_at,
        },
    );
    sqlx::query("UPDATE audit_logs SET entry_hash = ? WHERE id = ?")
        .bind(&entry_hash)
        .bind(id)
        .execute(&mut *executor)
        .await?;
    Ok(())
}

pub fn report_cache_key(semester_id: i64) -> String {
    format!("report:semester:{semester_id}")
}

/// Reported for a resource that either does not exist or belongs to another
/// class. Both cases return the same 404 on purpose: if a foreign resource
/// answered 403 or 401 instead, any authenticated caller could use the status
/// code as an existence oracle over the global id space and enumerate which
/// semester and transaction ids exist.
pub fn foreign_resource() -> AppError {
    AppError::NotFound
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
    // PostgreSQL maps SMALLINT to i16, SQLite maps INTEGER to i64. Try i16
    // first for PostgreSQL, then fall back to i64 for SQLite.
    let archived_int: i64 = row
        .try_get::<i16, _>("archived")
        .map(i64::from)
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

/// Reads a nullable TEXT column. `sqlx::Any` cannot decode SQL NULL into
/// `Option<T>` because of a type-info mismatch in the Any adapter: the column
/// type is reported as NULL, which the compatibility check for `Option<T>` does
/// not accept on sqlx 0.8. This helper swallows the resulting `ColumnDecode`
/// error and returns `None`, which is the only case that produces a
/// `ColumnDecode` error for these TEXT columns.
pub fn optional_string(row: &AnyRow, column_name: &str) -> AppResult<Option<String>> {
    row.try_get::<Option<String>, _>(column_name)
        .or_else(|error| match error {
            sqlx::Error::ColumnDecode { .. } => Ok(None),
            _ => Err(error.into()),
        })
}

/// Reads a nullable INTEGER column. See [`optional_string`] for why the
/// `ColumnDecode` error is mapped to `None`.
pub fn optional_i64(row: &AnyRow, column_name: &str) -> AppResult<Option<i64>> {
    row.try_get::<Option<i64>, _>(column_name)
        .or_else(|error| match error {
            sqlx::Error::ColumnDecode { .. } => Ok(None),
            _ => Err(error.into()),
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

/// Detects UNIQUE constraint violations, SQLite code 2067 and PostgreSQL
/// SQLSTATE 23505, so handlers can answer with 409 instead of a generic 500.
pub fn is_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database_error) => {
            let code = database_error
                .code()
                .as_deref()
                .map(ToString::to_string)
                .unwrap_or_default();
            code == "2067" || code == "23505"
        }
        _ => false,
    }
}
