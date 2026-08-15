// Transaction CRUD. Reads within a semester require ClassAuth (data isolation
// between classes). Writes additionally record an audit entry inside the same
// database transaction so the operation log never drifts from the data.
use axum::extract::{Path, Query, State};
use axum::Json;
use sqlx::Row;

use crate::auth::ClassAuth;
use crate::error::{AppError, AppResult};
use crate::handlers::{lookup_semester_class, now_rfc3339, record_audit, row_to_transaction};
use crate::limits::{
    check_optional_query_date, check_optional_text, check_required_text,
    check_rfc3339_timestamp, MAX_AMOUNT_CENTS, MAX_OPERATOR_LENGTH, MAX_QUERY_FILTER_LENGTH,
};
use crate::models::{CreateTransaction, TransactionList, TransactionQuery};
use crate::state::AppState;

// GET /api/semesters/:id/transactions - filtered, paginated listing. Filters
// are pure conjunctions so the composite index (semester_id, occurred_at) and
// (class_id, kind, occurred_at) cover the common access patterns.
pub async fn list_transactions(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
    Query(query): Query<TransactionQuery>,
) -> AppResult<Json<TransactionList>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    validate_transaction_query(&query)?;
    // Bound the page so the derived offset cannot overflow i64.
    const MAX_PAGE_NUMBER: i64 = 1_000_000;
    let page = query.page.unwrap_or(1).clamp(1, MAX_PAGE_NUMBER);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 200);
    let offset = (page - 1) * page_size;

    // semester_id is a BIGINT/INTEGER column and must be bound as i64.
    // Binding it through the string-based binds array would trigger a
    // text-to-bigint type mismatch on PostgreSQL.
    let mut where_clause = String::from("semester_id = ?");
    let mut binds: Vec<String> = Vec::new();
    if let Some(kind) = &query.kind {
        where_clause.push_str(" AND kind = ?");
        binds.push(kind.clone());
    }
    if let Some(from) = &query.from {
        where_clause.push_str(" AND occurred_at >= ?");
        binds.push(from.clone());
    }
    if let Some(to) = &query.to {
        // datetime-local inputs may send only the date portion (YYYY-MM-DD).
        // Append end-of-day time so the <= comparison covers the full day.
        where_clause.push_str(" AND occurred_at <= ?");
        binds.push(format!("{}T23:59:59Z", to));
    }
    if let Some(search) = &query.search {
        where_clause.push_str(
            " AND (source LIKE ? ESCAPE '/' OR purpose LIKE ? ESCAPE '/' \
             OR item LIKE ? ESCAPE '/' OR operator LIKE ? ESCAPE '/')",
        );
        let pattern = format!("%{}%", escape_like(search));
        binds.push(pattern.clone());
        binds.push(pattern.clone());
        binds.push(pattern.clone());
        binds.push(pattern);
    }

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM transactions WHERE {where_clause}");
    // semester_id is bound as i64 before the dynamic binds.
    let mut count_query = sqlx::query(&count_sql).bind(semester_id);
    for b in &binds {
        count_query = count_query.bind(b);
    }
    let total: i64 = count_query
        .fetch_one(&state.pool)
        .await?
        .try_get::<i64, _>("cnt")?;

    let list_sql = format!(
        "SELECT id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at FROM transactions WHERE {where_clause} \
         ORDER BY occurred_at DESC, id DESC LIMIT ? OFFSET ?"
    );
    let mut list_query = sqlx::query(&list_sql).bind(semester_id);
    for b in &binds {
        list_query = list_query.bind(b);
    }
    list_query = list_query.bind(page_size).bind(offset);
    let rows = list_query.fetch_all(&state.pool).await?;
    let data = rows
        .iter()
        .map(row_to_transaction)
        .collect::<Result<_, _>>()?;
    Ok(Json(TransactionList {
        data,
        total,
        page,
        page_size,
    }))
}

// POST /api/semesters/:id/transactions
pub async fn create_transaction(
    State(state): State<AppState>,
    Path(semester_id): Path<i64>,
    auth: ClassAuth,
    Json(body): Json<CreateTransaction>,
) -> AppResult<Json<crate::models::Transaction>> {
    let class_id = lookup_semester_class(&state, semester_id).await?;
    if class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    ensure_semester_writable(&state.pool, semester_id).await?;
    validate_transaction_body(&body)?;
    let occurred_at = body.occurred_at.unwrap_or_else(now_rfc3339);

    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO transactions \
         (semester_id, class_id, kind, amount_cents, source, purpose, item, operator, \
         occurred_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         RETURNING id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at",
    )
    .bind(semester_id)
    .bind(class_id)
    .bind(&body.kind)
    .bind(body.amount_cents)
    .bind(&body.source)
    .bind(&body.purpose)
    .bind(&body.item)
    .bind(body.operator.trim())
    .bind(&occurred_at)
    .bind(now_rfc3339())
    .fetch_one(&mut *tx)
    .await?;
    let transaction = row_to_transaction(&row)?;
    let after = serde_json::to_string(&transaction).ok();
    record_audit(
        &mut tx,
        class_id,
        Some(transaction.id),
        "create",
        &auth.operator,
        None,
        after,
    )
    .await?;
    tx.commit().await?;
    state
        .cache
        .invalidate(&crate::handlers::report_cache_key(semester_id));
    Ok(Json(transaction))
}

// GET /api/transactions/:id
pub async fn get_transaction(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<crate::models::Transaction>> {
    let row = sqlx::query(
        "SELECT id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at FROM transactions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let tx = row_to_transaction(&row)?;
    if tx.class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    Ok(Json(tx))
}

// PUT /api/transactions/:id
pub async fn update_transaction(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    auth: ClassAuth,
    Json(body): Json<CreateTransaction>,
) -> AppResult<Json<crate::models::Transaction>> {
    validate_transaction_body(&body)?;
    let occurred_at = body.occurred_at.unwrap_or_else(now_rfc3339);

    let mut tx = state.pool.begin().await?;
    let existing_row = sqlx::query(
        "SELECT id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at FROM transactions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let existing = row_to_transaction(&existing_row)?;
    if existing.class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    ensure_semester_writable(&mut *tx, existing.semester_id).await?;
    let before = serde_json::to_string(&existing).ok();

    let row = sqlx::query(
        "UPDATE transactions SET \
         kind = ?, amount_cents = ?, source = ?, purpose = ?, item = ?, operator = ?, \
         occurred_at = ? WHERE id = ? \
         RETURNING id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at",
    )
    .bind(&body.kind)
    .bind(body.amount_cents)
    .bind(&body.source)
    .bind(&body.purpose)
    .bind(&body.item)
    .bind(body.operator.trim())
    .bind(&occurred_at)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let updated = row_to_transaction(&row)?;
    let after = serde_json::to_string(&updated).ok();
    record_audit(
        &mut tx,
        existing.class_id,
        Some(id),
        "update",
        &auth.operator,
        before,
        after,
    )
    .await?;
    tx.commit().await?;
    state
        .cache
        .invalidate(&crate::handlers::report_cache_key(existing.semester_id));
    Ok(Json(updated))
}

// DELETE /api/transactions/:id
pub async fn delete_transaction(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<serde_json::Value>> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT id, semester_id, class_id, kind, amount_cents, source, purpose, item, \
         operator, occurred_at, created_at FROM transactions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;
    let existing = row_to_transaction(&row)?;
    if existing.class_id != auth.class_id {
        return Err(AppError::Unauthorized);
    }
    ensure_semester_writable(&mut *tx, existing.semester_id).await?;
    let before = serde_json::to_string(&existing).ok();
    // Delete the row first, then record the audit entry. Inserting the audit
    // row before to delete would create a transient FK reference to the row
    // being removed, which SQLite's ON DELETE SET NULL resolves but some
    // connection configurations reject mid-transaction. By deleting first and
    // storing transaction_id = None on the audit row, we avoid the circular
    // reference entirely while still preserving the full before-snapshot.
    sqlx::query("DELETE FROM transactions WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    record_audit(
        &mut tx,
        existing.class_id,
        None,
        "delete",
        &auth.operator,
        before,
        None,
    )
    .await?;
    tx.commit().await?;
    state
        .cache
        .invalidate(&crate::handlers::report_cache_key(existing.semester_id));
    Ok(Json(serde_json::json!({ "deleted": id })))
}

fn validate_kind(kind: &str) -> AppResult<()> {
    if kind == "income" || kind == "expense" {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "kind must be 'income' or 'expense'".into(),
        ))
    }
}

// Shared validation for create and update so both paths enforce the same
// bounds on every user-controlled field before it reaches the database.
fn validate_transaction_body(body: &CreateTransaction) -> AppResult<()> {
    validate_kind(&body.kind)?;
    if body.amount_cents <= 0 {
        return Err(AppError::BadRequest("amount_cents must be positive".into()));
    }
    if body.amount_cents > MAX_AMOUNT_CENTS {
        return Err(AppError::BadRequest(format!(
            "amount_cents may not exceed {MAX_AMOUNT_CENTS}"
        )));
    }
    check_required_text(&body.operator, "operator", MAX_OPERATOR_LENGTH)?;
    check_optional_text(&body.source, "source")?;
    check_optional_text(&body.purpose, "purpose")?;
    check_optional_text(&body.item, "item")?;
    if let Some(occurred_at) = &body.occurred_at {
        check_rfc3339_timestamp(occurred_at, "occurred_at")?;
    }
    Ok(())
}

// Rejects oversized or malformed filter values before they reach the database.
// The search term feeds a LIKE pattern, so bounding its length prevents an
// attacker from forcing a pathological full-table scan.
fn validate_transaction_query(query: &TransactionQuery) -> AppResult<()> {
    if let Some(kind) = &query.kind {
        if kind != "income" && kind != "expense" {
            return Err(AppError::BadRequest(
                "kind must be 'income' or 'expense'".into(),
            ));
        }
    }
    if let Some(search) = &query.search {
        if search.chars().count() > MAX_QUERY_FILTER_LENGTH {
            return Err(AppError::BadRequest(format!(
                "search may not exceed {MAX_QUERY_FILTER_LENGTH} characters"
            )));
        }
    }
    check_optional_query_date(&query.from, "from")?;
    check_optional_query_date(&query.to, "to")?;
    Ok(())
}

// Escapes LIKE wildcards so user input is matched literally instead of acting
// as a pattern. The '/' escape character is omitted from the pattern itself.
fn escape_like(value: &str) -> String {
    value.replace('/', "//").replace('%', "/%").replace('_', "/_")
}

// Blocks mutations on an archived semester so the read-only promise shown in
// the UI is enforced server-side too, not just in the browser. Runs through
// the caller's executor so it can participate in an open database transaction.
async fn ensure_semester_writable<'e, E>(executor: E, semester_id: i64) -> AppResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Any>,
{
    let row = sqlx::query("SELECT archived FROM semesters WHERE id = ?")
        .bind(semester_id)
        .fetch_optional(executor)
        .await?
        .ok_or(AppError::NotFound)?;
    // PostgreSQL maps SMALLINT to i16; SQLite maps INTEGER to i64.
    let archived_int: i64 = row
        .try_get::<i16, _>("archived")
        .map(|smallint_value| smallint_value as i64)
        .or_else(|_| row.try_get::<i64, _>("archived"))?;
    if archived_int != 0 {
        return Err(AppError::Conflict(
            "semester is archived and read-only".into(),
        ));
    }
    Ok(())
}
