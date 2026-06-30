// Database connection and schema bootstrap. Uses SQLX::Any so the same binary
// can target either bundled SQLite (default, zero-config) or PostgreSQL (set
// BAZAARLOG_DATABASE_URL=postgres://...). The init SQL is split into individual
// statements because the Any driver executes one statement per call.
use sqlx::AnyPool;

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
