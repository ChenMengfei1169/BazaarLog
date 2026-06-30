-- BazaarLog PostgreSQL initialization script.
-- Entity-Relationship overview:
--   classes        1---* semesters
--         1---* transactions
--      1---* audit_logs
--   classes        1---* audit_logs (operation scope)
-- Conventions: timestamps are TEXT in RFC3339 UTC (lexicographically sortable,
-- so indexes still serve range scans). Money is stored as BIGINT cents to avoid
-- floating point drift. Booleans are stored as SMALLINT (0/1) to keep a single
-- portable SQL code path with the SQLite build. Destructive operations are
-- retained through audit_logs rather than soft-delete columns.

CREATE TABLE IF NOT EXISTS classes (
    id            BIGSERIAL PRIMARY KEY,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT to_char(CLOCK_TIMESTAMP() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
);

CREATE TABLE IF NOT EXISTS semesters (
    id         BIGSERIAL PRIMARY KEY,
    class_id   BIGINT NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    archived   SMALLINT NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    start_date TEXT,
    end_date   TEXT,
    created_at TEXT NOT NULL DEFAULT to_char(CLOCK_TIMESTAMP() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
    UNIQUE (class_id, name)
);

CREATE TABLE IF NOT EXISTS transactions (
    id           BIGSERIAL PRIMARY KEY,
    semester_id  BIGINT NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
    class_id     BIGINT NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('income', 'expense')),
    amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
    source       TEXT,
    purpose      TEXT,
    item         TEXT,
    operator     TEXT NOT NULL,
    occurred_at  TEXT NOT NULL DEFAULT to_char(CLOCK_TIMESTAMP() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
    created_at   TEXT NOT NULL DEFAULT to_char(CLOCK_TIMESTAMP() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id             BIGSERIAL PRIMARY KEY,
    transaction_id BIGINT REFERENCES transactions(id) ON DELETE SET NULL,
    class_id       BIGINT REFERENCES classes(id) ON DELETE CASCADE,
    action         TEXT NOT NULL CHECK (action IN ('create', 'update', 'delete')),
    operator       TEXT NOT NULL,
    payload_before TEXT,
    payload_after  TEXT,
    occurred_at    TEXT NOT NULL DEFAULT to_char(CLOCK_TIMESTAMP() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')
);

-- Composite indexes for the hot list/filter paths.
CREATE INDEX IF NOT EXISTS idx_transactions_semester_time
    ON transactions (semester_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_class_kind_time
    ON transactions (class_id, kind, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_transactions_semester_item
    ON transactions (semester_id, item);
CREATE INDEX IF NOT EXISTS idx_audit_logs_transaction
    ON audit_logs (transaction_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_semesters_class_archived
    ON semesters (class_id, archived);
