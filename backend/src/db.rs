// Database connection and schema bootstrap. Uses sqlx::Any so the same binary
// can target either bundled SQLite, the default zero-config choice, or
// PostgreSQL via BAZAARLOG_DATABASE_URL=postgres://... The init SQL is split
// into individual statements because the Any driver executes one statement per
// call.
use sqlx::{AnyPool, Connection, Row};

pub async fn connect(url: &str) -> anyhow::Result<AnyPool> {
    // Registers the SQLite and PostgreSQL backends so Any can dispatch on the
    // connection-string scheme at runtime.
    sqlx::any::install_default_drivers();
    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(8)
        .connect(url)
        .await?;
    Ok(pool)
}

pub async fn init_schema(pool: &AnyPool, is_sqlite: bool) -> anyhow::Result<()> {
    let sql = if is_sqlite {
        include_str!("../migrations/sqlite_init.sql")
    } else {
        include_str!("../migrations/postgres_init.sql")
    };
    run_script(pool, sql).await
}

async fn run_script(pool: &AnyPool, sql: &str) -> anyhow::Result<()> {
    for statement in split_statements(sql) {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

/// Splits a SQL script on ';' boundaries. Safe for these migration files since
/// no statement embeds a semicolon inside a string literal.
fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|statement| statement.trim().to_string())
        .collect()
}

/// Adds the audit hash-chain columns to an existing database. Fresh databases
/// get them from the migration script; older databases are upgraded here. The
/// ALTER runs before [`crate::audit::backfill_chain`] so every pre-existing row
/// can be chained.
pub async fn ensure_audit_chain_columns(pool: &AnyPool, is_sqlite: bool) -> anyhow::Result<()> {
    let statements: &[&str] = if is_sqlite {
        &[
            "ALTER TABLE audit_logs ADD COLUMN prev_hash TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE audit_logs ADD COLUMN entry_hash TEXT NOT NULL DEFAULT ''",
        ]
    } else {
        &[
            "ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS prev_hash TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS entry_hash TEXT NOT NULL DEFAULT ''",
        ]
    };
    for statement in statements {
        match sqlx::query(statement).execute(pool).await {
            Ok(_) => {}
            Err(sqlx::Error::Database(database_error)) => {
                // SQLite reports a duplicate column when the column already
                // exists; PostgreSQL's IF NOT EXISTS makes this unreachable.
                let message = database_error.message().to_string();
                if !message.to_lowercase().contains("duplicate column") {
                    return Err(sqlx::Error::Database(database_error).into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Removes the foreign key from audit_logs.transaction_id so deleting a
/// transaction can never rewrite historical audit rows. The FK was declared
/// ON DELETE SET NULL, which silently nulled the create and update audit entries
/// and broke their chain hashes every time a transaction was deleted, so a
/// completely normal delete made the audit chain report "unverified". Audit rows
/// are append-only evidence: they keep their transaction_id forever, and the
/// delete handler appends a new entry with transaction_id = NULL instead.
///
/// Fresh databases already use the FK-less schema from the migration script;
/// this only upgrades existing databases. SQLite cannot drop a constraint in
/// place, so the table is rebuilt; PostgreSQL drops the auto-named constraint
/// directly.
pub async fn ensure_audit_log_immutable(pool: &AnyPool, is_sqlite: bool) -> anyhow::Result<()> {
    if !is_sqlite {
        sqlx::query(
            "ALTER TABLE audit_logs DROP CONSTRAINT IF EXISTS audit_logs_transaction_id_fkey",
        )
        .execute(pool)
        .await?;
        return Ok(());
    }
    // Detect the old SQLite schema: its stored CREATE TABLE text references
    // transactions through the FK on transaction_id. The FK-less schema never
    // contains that text, so the check is exact for databases created by this
    // project's migration files.
    let row = sqlx::query(
        "SELECT COUNT(*) AS transaction_reference_count FROM sqlite_master \
         WHERE type = 'table' AND name = 'audit_logs' \
         AND instr(lower(sql), 'references transactions') > 0",
    )
    .fetch_one(pool)
    .await?;
    let transaction_reference_count: i64 = row.try_get("transaction_reference_count")?;
    if transaction_reference_count == 0 {
        return Ok(());
    }
    // Rebuild the table on a single connection. Row ids must be preserved
    // verbatim because the chain hashes commit each row's id; the chain columns
    // are copied through, the transaction index recreated, and the
    // AUTOINCREMENT sequence re-anchored to the current max id.
    let mut connection = pool.acquire().await?;
    let mut transaction = connection.begin().await?;
    let rebuild_statements: &[&str] = &[
        "CREATE TABLE audit_logs_rebuilt ( \
             id             INTEGER PRIMARY KEY AUTOINCREMENT, \
             transaction_id INTEGER, \
             class_id       INTEGER REFERENCES classes(id) ON DELETE CASCADE, \
             action         TEXT NOT NULL CHECK (action IN ('create', 'update', 'delete')), \
             operator       TEXT NOT NULL, \
             payload_before TEXT, \
             payload_after  TEXT, \
             occurred_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), \
             prev_hash      TEXT NOT NULL DEFAULT '', \
             entry_hash     TEXT NOT NULL DEFAULT '' \
         )",
        "INSERT INTO audit_logs_rebuilt \
             (id, transaction_id, class_id, action, operator, payload_before, \
              payload_after, occurred_at, prev_hash, entry_hash) \
         SELECT id, transaction_id, class_id, action, operator, payload_before, \
              payload_after, occurred_at, prev_hash, entry_hash \
         FROM audit_logs",
        "DROP TABLE audit_logs",
        "ALTER TABLE audit_logs_rebuilt RENAME TO audit_logs",
        "CREATE INDEX IF NOT EXISTS idx_audit_logs_transaction \
             ON audit_logs (transaction_id, occurred_at DESC)",
        "DELETE FROM sqlite_sequence WHERE name = 'audit_logs'",
        "INSERT INTO sqlite_sequence (name, seq) \
             SELECT 'audit_logs', COALESCE(MAX(id), 0) FROM audit_logs",
    ];
    for statement in rebuild_statements {
        sqlx::query(statement).execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    tracing::info!("rebuilt audit_logs without the transaction_id foreign key");
    Ok(())
}

/// Adds a UNIQUE index on class names so the application-level duplicate check
/// in create_class is backed by a real constraint, which closes the
/// time-of-check-to-time-of-use window. The index is only created when the
/// existing data is already unique; a database that accumulated duplicates
/// before this migration is left alone rather than failing startup.
pub async fn ensure_class_name_unique(pool: &AnyPool) -> anyhow::Result<()> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS duplicate_count FROM ( \
         SELECT name FROM classes GROUP BY name HAVING COUNT(*) > 1 \
         ) AS duplicate_groups",
    )
    .fetch_one(pool)
    .await?;
    let duplicate_count: i64 = row.try_get("duplicate_count")?;
    if duplicate_count > 0 {
        tracing::warn!(
            duplicates = duplicate_count,
            "skipping unique class-name index because existing data contains duplicates"
        );
        return Ok(());
    }
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_classes_name ON classes(name)")
        .execute(pool)
        .await?;
    Ok(())
}

/// Resolves a Windows system tool by absolute path under System32.
///
/// Using a bare program name would let CreateProcess search the directory of the
/// running executable and then the current working directory before System32, so
/// a file planted next to BazaarLog.exe would be executed with the server's
/// privileges. That is not a theoretical precondition here: the documented
/// deployment is "copy the exe to any folder and double-click it", and the
/// target environment is a shared classroom PC (CWE-426, CWE-427).
#[cfg(target_os = "windows")]
fn system32_tool(file_name: &str) -> Option<std::path::PathBuf> {
    let root = std::env::var_os("SystemRoot").map_or_else(
        || std::path::PathBuf::from(r"C:\Windows"),
        std::path::PathBuf::from,
    );
    let candidate = root.join("System32").join(file_name);
    candidate.is_file().then_some(candidate)
}

/// Tightens one file's ACL to the current Windows user only. Best effort.
/// Returns true when the ACL is known to be tight, or when there is nothing to
/// do on this platform; false when the tightening could not be confirmed.
#[cfg(target_os = "windows")]
pub fn harden_file_acl(path: &std::path::Path) -> bool {
    use std::process::Command;

    if !path.exists() {
        // Nothing to protect yet; callers re-apply this after every write.
        return true;
    }
    let (Some(whoami_exe), Some(icacls_exe)) =
        (system32_tool("whoami.exe"), system32_tool("icacls.exe"))
    else {
        tracing::warn!(
            "could not locate whoami.exe / icacls.exe under System32; leaving the ACL unchanged"
        );
        return false;
    };
    let whoami_output = match Command::new(&whoami_exe).output() {
        Ok(output) if output.status.success() => output.stdout,
        Ok(output) => {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&output.stderr),
                "whoami failed; leaving the ACL unchanged"
            );
            return false;
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to run whoami; leaving the ACL unchanged");
            return false;
        }
    };
    let current_user = String::from_utf8_lossy(&whoami_output).trim().to_string();
    if current_user.is_empty() {
        return false;
    }
    // Step 1: grant full control to the current user. This must succeed first:
    // removing inherited ACEs before the grant is in place would leave the file
    // with an empty DACL and lock the server out of its own data.
    let grant_succeeded = match Command::new(&icacls_exe)
        .arg(path)
        .arg("/grant:r")
        .arg(format!("{current_user}:(F)"))
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            tracing::warn!(
                path = %path.display(),
                stderr = %String::from_utf8_lossy(&output.stderr),
                "failed to grant the current user full control; leaving the ACL unchanged"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to run icacls; leaving the ACL unchanged"
            );
            false
        }
    };
    if !grant_succeeded {
        return false;
    }
    // Step 2: drop inherited ACEs, typically broad grants such as Authenticated
    // Users, leaving only the current user's explicit grant.
    match Command::new(&icacls_exe)
        .arg(path)
        .arg("/inheritance:r")
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            tracing::warn!(
                path = %path.display(),
                stderr = %String::from_utf8_lossy(&output.stderr),
                "failed to remove inherited ACEs; the file keeps its inherited permissions"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to remove inherited ACEs"
            );
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn harden_file_acl(_path: &std::path::Path) -> bool {
    // ACL tightening is a Windows deployment concern; other platforms rely on
    // the file permissions already in place.
    true
}

/// Best-effort tightening of the SQLite database file and its audit seal file to
/// the current Windows user only, so other local accounts cannot read or modify
/// either.
///
/// Both files need this. The seal is the external anchor that makes truncation
/// of the audit log tail detectable, so a world-writable seal would let another
/// local user forge a tamper warning or, by deleting the anchor, silently
/// disable truncation detection. Runs at startup and never fails it. Set
/// BAZAARLOG_HARDEN_DB_ACL=0 to skip. The seal is re-hardened by
/// [`crate::audit::refresh_seal`] after every rewrite, because replacing the
/// file installs a new file with the directory's default ACL.
#[cfg(target_os = "windows")]
pub fn harden_database_file_acl(database_url: &str) {
    let enabled = std::env::var("BAZAARLOG_HARDEN_DB_ACL").map_or(true, |value| value == "1");
    if !enabled || !database_url.starts_with("sqlite") {
        return;
    }
    let raw_path = database_url
        .trim_start_matches("sqlite://")
        .split('?')
        .next()
        .unwrap_or(database_url);
    let database_path = std::path::Path::new(raw_path);
    if !database_path.exists() {
        return;
    }
    if harden_file_acl(database_path) {
        tracing::info!("tightened ACL of the database file to the current user");
    }
    // The seal lives next to the database and is covered by the same policy.
    if let Some(seal_path) = crate::audit::seal_path_for_database(database_url) {
        if harden_file_acl(&seal_path) {
            tracing::info!("tightened ACL of the audit seal file to the current user");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn harden_database_file_acl(_database_url: &str) {
    // ACL tightening is a Windows deployment concern; other platforms rely on
    // the file permissions already in place.
}
