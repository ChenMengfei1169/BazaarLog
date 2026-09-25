// Class management: list, create, password verification, audit log view, and
// audit chain verification.
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::header;
use axum::http::HeaderMap;
use axum::Json;
use sqlx::Row;
use zeroize::Zeroizing;

use crate::auth::{hash_password_bounded, verify_password_bounded, ClassAuth, DUMMY_PASSWORD_HASH};
use crate::error::{AppError, AppResult};
use crate::handlers::{
    foreign_resource, optional_i64, optional_string, record_audit, row_to_class,
    AUDIT_CHAIN_CACHE_KEY, AUDIT_CHAIN_CACHE_TTL,
};
use crate::limits::{
    check_required_text, MAX_CLASS_NAME_LENGTH, MAX_OPERATOR_LENGTH, MAX_PASSWORD_LENGTH,
    MIN_PASSWORD_LENGTH,
};
use crate::models::{AuditLog, AuthClass, Class, CreateClass};
use crate::state::AppState;

/// GET /api/classes - public; returns the list of class names so the switcher
/// can populate its dropdown before the user has authenticated.
pub async fn list_classes(State(state): State<AppState>) -> AppResult<Json<Vec<Class>>> {
    let rows = sqlx::query("SELECT id, name, created_at FROM classes ORDER BY id")
        .fetch_all(&state.pool)
        .await?;
    let classes = rows.iter().map(row_to_class).collect::<Result<_, _>>()?;
    Ok(Json(classes))
}

/// GET /api/classes/challenge - public; issues a fresh single-use proof-of-work
/// challenge that must accompany class creation.
pub async fn get_create_challenge(
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let nonce = state.challenges.issue();
    let difficulty = state.config.proof_of_work_difficulty.clamp(1, 16);
    Ok(Json(serde_json::json!({
        "nonce": nonce,
        "difficulty": difficulty,
    })))
}

/// POST /api/classes - public; creates a class with an Argon2id password hash.
/// Creation requires a valid proof-of-work challenge so bulk scripted creation
/// costs CPU per attempt instead of being free.
pub async fn create_class(
    State(state): State<AppState>,
    Json(body): Json<CreateClass>,
) -> AppResult<Json<Class>> {
    // Verify the proof-of-work challenge first so a rejected challenge never
    // reaches the expensive password hashing below.
    let difficulty = state.config.proof_of_work_difficulty.clamp(1, 16);
    let challenge_nonce = body.challenge_nonce.as_deref().unwrap_or("");
    let challenge_solution = body.challenge_solution.as_deref().unwrap_or("");
    if !state
        .challenges
        .verify_solution(challenge_nonce, challenge_solution, difficulty)
    {
        return Err(AppError::BadRequest(
            "a valid proof-of-work challenge is required to create a class; \
             fetch one from GET /api/classes/challenge first"
                .into(),
        ));
    }
    check_required_text(&body.name, "class name", MAX_CLASS_NAME_LENGTH)?;
    // Normalize surrounding whitespace so "Final Class" and "Final Class "
    // cannot exist side by side as two indistinguishable dropdown entries.
    let class_name = body.name.trim();
    // Reject duplicate names. A UNIQUE index also backs this, but it is created
    // only when the existing data is already unique, so the check stays here.
    let existing_class_row = sqlx::query("SELECT id FROM classes WHERE name = ?")
        .bind(class_name)
        .fetch_optional(&state.pool)
        .await?;
    if existing_class_row.is_some() {
        return Err(AppError::Conflict(
            "a class with this name already exists".into(),
        ));
    }
    // Count characters, not bytes, so multi-byte names are bounded correctly.
    let password_length = body.password.chars().count();
    if !(MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&password_length) {
        return Err(AppError::BadRequest(format!(
            "password must be {MIN_PASSWORD_LENGTH} to {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    // The password is moved into the blocking pool, which wraps it in Zeroizing
    // so its heap buffer is wiped once the hash is computed. The global
    // concurrency cap also stops this public endpoint from being used to exhaust
    // memory with simultaneous 64 MiB Argon2 hashes.
    let password_hash = hash_password_bounded(&state, body.password).await?;
    // Hold the mutation guard across the write so the audit chain anchor read
    // inside record_audit cannot race another writer. The Argon2 hash above
    // deliberately runs outside the guard, so a slow hash never serializes
    // unrelated mutations.
    let _mutation_guard = state.mutation_guard.lock().await;
    let mut transaction = state.pool.begin().await?;
    let row = match sqlx::query(
        "INSERT INTO classes (name, password_hash) VALUES (?, ?) \
         RETURNING id, name, created_at",
    )
    .bind(class_name)
    .bind(&password_hash)
    .fetch_one(&mut *transaction)
    .await
    {
        Ok(row) => row,
        Err(error) if crate::handlers::is_unique_violation(&error) => {
            return Err(AppError::Conflict(
                "a class with this name already exists".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    };
    let class = row_to_class(&row)?;
    // Class creation is audited like every other create, so no ledger can appear
    // without a trace. This endpoint is public and the proof-of-work challenge
    // carries no identity, so the entry records the established "anonymous"
    // sentinel instead of inventing an operator name.
    record_audit(
        &mut transaction,
        class.id,
        None,
        "create",
        "anonymous",
        None,
        serde_json::to_string(&class).ok(),
    )
    .await?;
    transaction.commit().await?;
    state.cache.invalidate(AUDIT_CHAIN_CACHE_KEY);
    crate::audit::refresh_seal(&state.pool, &state.config.database_url).await?;
    Ok(Json(class))
}

/// POST /api/classes/:id/auth - public; returns 200 with a session token when
/// the password matches, 401 otherwise. The token is bound to the client address
/// and User-Agent captured at login time, and only its digest is kept in the
/// server's memory.
pub async fn auth_class(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<AuthClass>,
) -> AppResult<Json<serde_json::Value>> {
    // Bound the password length before running the expensive Argon2 verification
    // so an attacker cannot reach the hasher with an oversized password and
    // exhaust server CPU or memory. The create path enforces the same upper
    // bound, so no legitimate password exceeds it.
    if body.password.chars().count() > MAX_PASSWORD_LENGTH {
        return Err(AppError::BadRequest(format!(
            "password may not exceed {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    // Move the password into the blocking pool, which wraps it in Zeroizing so
    // its heap buffer is wiped once verification is done. The global concurrency
    // cap bounds how many 64 MiB Argon2 workspaces can exist at once, so
    // rotating the class id cannot scale the attacker's memory use.
    let password = body.password;
    // The per-class failure tracker is global, independent of source address, so
    // brute force that rotates addresses still hits the lockout.
    if state.auth_failures.is_blocked(class_id) {
        return Err(AppError::RateLimited);
    }
    let row = sqlx::query("SELECT password_hash FROM classes WHERE id = ?")
        .bind(class_id)
        .fetch_optional(&state.pool)
        .await?;
    // A missing class id still burns one full Argon2 verification against the
    // dummy hash, so response timing does not reveal which ids exist.
    let encoded = match row {
        Some(row) => row.try_get::<String, _>("password_hash")?,
        None => DUMMY_PASSWORD_HASH.to_string(),
    };
    if !verify_password_bounded(&state, password, encoded).await? {
        state.auth_failures.record_failure(class_id);
        return Err(AppError::Unauthorized);
    }
    // Bind the typed operator name to a server-side session token so the audit
    // log records it authoritatively instead of trusting a header. SessionStore
    // copies and truncates the User-Agent it stores, so the borrowed header
    // value is passed straight through.
    let typed_operator: String = body
        .operator
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_OPERATOR_LENGTH)
        .collect();
    let operator_name = if typed_operator.is_empty() {
        "anonymous".to_string()
    } else {
        typed_operator
    };
    state.auth_failures.record_success(class_id);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    // create() is infallible: when the store is full it evicts the session
    // closest to expiry instead of failing the login.
    let token = state
        .sessions
        .create(class_id, operator_name, Some(address.ip()), user_agent);
    Ok(Json(serde_json::json!({
        "authenticated": true,
        "token": token.as_str(),
    })))
}

/// POST /api/logout - revokes the presented session token. Does not require a
/// valid ClassAuth because a stale or expired token must still be discardable.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    if let Some(token) = headers
        .get("x-session-token")
        .and_then(|value| value.to_str().ok())
    {
        // Zeroize the transient copy of the presented token after revocation.
        let token = Zeroizing::new(token.to_string());
        state.sessions.revoke(token.as_str());
    }
    Ok(Json(serde_json::json!({ "logged_out": true })))
}

/// GET /api/classes/:id/audit_logs - returns the most recent operation log
/// entries for the class, including the chain hashes for manual verification.
/// Requires ClassAuth so cross-class visibility stays blocked even if a stale URL
/// is replayed.
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<Vec<AuditLog>>> {
    if auth.class_id != class_id {
        return Err(foreign_resource());
    }
    let rows = sqlx::query(
        "SELECT id, transaction_id, class_id, action, operator, payload_before, \
         payload_after, occurred_at, prev_hash, entry_hash FROM audit_logs \
         WHERE class_id = ? ORDER BY occurred_at DESC, id DESC LIMIT 200",
    )
    .bind(class_id)
    .fetch_all(&state.pool)
    .await?;
    let logs = rows
        .iter()
        .map(|row| {
            Ok(AuditLog {
                id: row.try_get("id")?,
                transaction_id: optional_i64(row, "transaction_id")?,
                class_id: optional_i64(row, "class_id")?,
                action: row.try_get("action")?,
                operator: row.try_get("operator")?,
                payload_before: optional_string(row, "payload_before")?,
                payload_after: optional_string(row, "payload_after")?,
                occurred_at: row.try_get("occurred_at")?,
                prev_hash: row.try_get("prev_hash")?,
                entry_hash: row.try_get("entry_hash")?,
            })
        })
        .collect::<Result<_, AppError>>()?;
    Ok(Json(logs))
}

/// GET /api/classes/:id/audit_logs/chain - verifies the global audit hash chain
/// and reports whether it is intact. Any deleted, edited, or inserted audit row
/// breaks the chain at some row, which this endpoint surfaces.
///
/// This is the most expensive read in the API: it walks the entire audit table,
/// including the fat JSON snapshots, and recomputes a SHA-256 per row. The chain
/// is global by design, since record_audit links every class into one chain, so
/// any authenticated caller triggers a whole-table verification. That is why the
/// scan result is cached and why the endpoint has a dedicated rate limit bucket.
///
/// The two halves of the answer are deliberately treated differently:
///
///   * The external seal check is O(1), one small file read plus one primary-key
///     lookup, so it runs on EVERY request, outside the cache. Removing or
///     rewriting the anchor is therefore reported on the very next request
///     instead of being masked by a cached "verified" verdict.
///   * Only the internal-chain scan is cached, and under a much shorter TTL
///     (AUDIT_CHAIN_CACHE_TTL) than the default, because any caching of it is
///     directly detection latency for out-of-band edits to the audit table.
pub async fn verify_audit_chain(
    State(state): State<AppState>,
    Path(class_id): Path<i64>,
    auth: ClassAuth,
) -> AppResult<Json<crate::audit::AuditChainReport>> {
    if auth.class_id != class_id {
        return Err(foreign_resource());
    }

    // Cheap check first, and never cached: a missing or rewritten seal is
    // reported immediately even if a fresh scan result is already cached.
    let seal_ok = match crate::audit::seal_path_for_database(&state.config.database_url) {
        Some(seal_path) => crate::audit::seal_verifies(&state.pool, &seal_path).await?,
        // Non-SQLite backends have no external anchor; see audit.rs.
        None => true,
    };

    // The cached value is the raw scan result only; verify_chain never sets
    // `truncated`, so no seal verdict can go stale inside the cache.
    let cached_report = state
        .cache
        .get(AUDIT_CHAIN_CACHE_KEY)
        .and_then(|cached| serde_json::from_str::<crate::audit::AuditChainReport>(&cached).ok());
    let mut report = if let Some(cached_report) = cached_report {
        cached_report
    } else {
        let fresh_report = crate::audit::verify_chain(&state.pool).await?;
        if let Ok(serialized) = serde_json::to_string(&fresh_report) {
            state.cache.set_with_ttl(
                AUDIT_CHAIN_CACHE_KEY.to_string(),
                serialized,
                AUDIT_CHAIN_CACHE_TTL,
            );
        }
        fresh_report
    };

    // Compare the anchor against the database so deleting the tail of the log,
    // which an internal chain cannot see, is flagged too.
    if report.verified && !seal_ok {
        report.verified = false;
        report.truncated = true;
    }
    Ok(Json(report))
}
