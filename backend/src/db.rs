// Database connection and schema bootstrap. Uses SQLX::Any so the same binary
// can target either bundled SQLite (default, zero-config) or PostgreSQL (set
// BAZAARLOG_DATABASE_URL=postgres://...). The init SQL is split into individual
// statements because the Any driver executes one statement per call.
use sqlx::{AnyPool, Row};

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
    for stmt in split_statements(sql) {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        sqlx::query(stmt).execute(pool).await?;
    }
    Ok(())
}

// Splits a SQL script on ';' boundaries. Safe for these migration files since
// no statement embeds a semicolon inside a string literal.
fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';').map(|s| s.trim().to_string()).collect()
}

// Adds a UNIQUE index on class names so the application-level duplicate check
// in create_class is backed by a real constraint (closing the TOCTOU window).
// The index is only created when the existing data is already unique; a
// database that accumulated duplicates before this migration is left alone
// rather than failing startup.
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
