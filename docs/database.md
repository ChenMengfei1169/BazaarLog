# BazaarLog database design

## ER overview (text)

```
+----------+        +------------+        +----------------+        +-------------+
| classes  | 1----* | semesters  | 1----* | transactions   | 1----* | audit_logs  |
+----------+        +------------+        +----------------+        +-------------+
| id (PK)  |        | id (PK)    |        | id (PK)        |        | id (PK)     |
| name     |        | class_id   |---> classes.id          |        | transaction_id (FK)|
| password_hash |   | name       |        | semester_id    |---> semesters.id      |
| created_at|       | archived   |        | class_id       |---> classes.id        |
+----------+        | start_date |        | kind           |        | class_id    |---> classes.id
                    | end_date   |        | amount_cents   |        | action      |
                    | created_at |        | source         |        | operator    |
                    +------------+        | purpose        |        | payload_before |
                                          | item           |        | payload_after  |
                                          | operator       |        | occurred_at |
                                          | occurred_at    |        +-------------+
                                          | created_at     |
                                          +----------------+

Relationships:
  classes   1---* semesters        (a class owns many semesters)
  semesters 1---* transactions     (a semester contains many transactions)
  transactions 1---* audit_logs    (each mutation creates an audit row)
  classes   1---* audit_logs       (audit rows also reference the class for
                                    fast per-class audit queries)
```

## Design decisions

- **Money as integer cents** (`amount_cents`). Avoids floating-point drift in
  aggregates; the Excel export converts to decimal CNY only at the I/O boundary.
- **Timestamps as `TEXT` RFC3339 UTC**. Lexicographically sortable, so the
  composite indexes still serve range scans, and the same SQL works on both
  SQLite and PostgreSQL without driver-specific datetime types.
- **Booleans as `SMALLINT` (0/1)** on PostgreSQL. Keeps a single portable SQL
  code path with SQLite (which has no native boolean).
- **Audit log captures `payload_before` and `payload_after`** as JSON text. A
  delete stores only `payload_before`; a creation stores only `payload_after`.
  This preserves a full history without soft-delete columns on
  `transactions`.
- **Destructive operations retained via `audit_logs`** rather than soft-delete
  columns. The `transactions` table stays small and the listing fast; the
  history lives in a separate table that is queried only on demand.

## Indexes

| Index                                          | Purpose                                              |
|------------------------------------------------|------------------------------------------------------|
| `idx_transactions_semester_time`               | Default listing: latest transactions for a semester. |
| `idx_transactions_class_kind_time`             | Filtered dashboard queries by class + kind.          |
| `idx_transactions_semester_item`               | Item ranking aggregation.                            |
| `idx_audit_logs_transaction`                   | Audit lookup for a transaction.                      |
| `idx_semesters_class_archived`                 | Semester switcher listing.                           |

## Initialization scripts

- PostgreSQL: [backend/migrations/postgres_init.sql](../backend/migrations/postgres_init.sql)
- SQLite:     [backend/migrations/sqlite_init.sql](../backend/migrations/sqlite_init.sql)

Both scripts are idempotent (`CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT
EXISTS`). BazaarLog auto-applies the matching script on first connect based on
the `BAZAARLOG_DATABASE_URL` scheme.
