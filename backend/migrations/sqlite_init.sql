-- BazaarLog SQLite initialization script (single-machine bundled mode).
-- Mirrors the PostgreSQL schema with SQLite portable types. Money is stored as
-- INTEGER cents. Timestamps are TEXT in RFC3339 UTC.

CREATE TABLE IF NOT EXISTS classes (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS semesters (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    class_id   INTEGER NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    archived   INTEGER NOT NULL DEFAULT 0,
    start_date TEXT,
    end_date   TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (class_id, name)
);

CREATE TABLE IF NOT EXISTS transactions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    semester_id  INTEGER NOT NULL REFERENCES semesters(id) ON DELETE CASCADE,
    class_id     INTEGER NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('income', 'expense')),
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    source       TEXT,
    purpose      TEXT,
    item         TEXT,
    operator     TEXT NOT NULL,
    occurred_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_id INTEGER REFERENCES transactions(id) ON DELETE SET NULL,
    class_id       INTEGER REFERENCES classes(id) ON DELETE CASCADE,
    action         TEXT NOT NULL CHECK (action IN ('create', 'update', 'delete')),
    operator       TEXT NOT NULL,
    payload_before TEXT,
    payload_after  TEXT,
    occurred_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

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
