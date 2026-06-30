# BazaarLog HTTP API

All endpoints are served by the Rust backend on `http://localhost:3000`. The
embedded frontend is also served from the same origin, so a single
`BazaarLog.exe` provides the full experience.

## Conventions

- Money is always represented as integer cents (`amount_cents`). For example,
  `1234` means `12.34 CNY`.
- Timestamps are RFC3339 UTC strings (e.g. `2026-03-04T08:30:00Z`).
- All mutating requests that touch a class's data require three headers:
  - `X-Class-Id` - the class the caller is operating on
  - `X-Class-Password` - that class's plaintext password (verified server-side
    against an Argolid hash)
  - `X-Operator` - display name recorded in the audit log (defaults to
    `anonymous`)
- Errors are returned as `{"error": "<message>"}` with an appropriate HTTP
  status code. The body never leaks internal details for 5xx errors.

## Endpoints

### Health & metrics

| Method | Path          | Auth | Description                 |
|--------|---------------|------|-----------------------------|
| GET    | `/api/health` | none | Returns `{"status":"ok"}`.  |
| GET    | `/metrics`    | none | Prometheus text exposition. |

### Classes

| Method | Path                          | Auth  | Description                                                                                        |
|--------|-------------------------------|-------|----------------------------------------------------------------------------------------------------|
| GET    | `/api/classes`                | none  | List all class names (no password hashes exposed).                                                 |
| POST   | `/api/classes`                | none  | Create a class. Body: `{name, password}`. Returns the new class.                                   |
| POST   | `/api/classes/:id/auth`       | none  | Verify a password. Body: `{password}`. Returns `{"authenticated":true}` on success, 401 otherwise. |
| GET    | `/api/classes/:id/audit_logs` | class | Returns up to 200 most recent audit log entries.                                                   |

### Semesters

| Method | Path                                  | Auth  | Description                                                       |
|--------|---------------------------------------|-------|-------------------------------------------------------------------|
| GET    | `/api/classes/:id/semesters`          | class | List semesters for the class.                                     |
| POST   | `/api/classes/:id/semesters`          | class | Create a semester. Body: `{name, start_date?, end_date?}`.        |
| POST   | `/api/semesters/:id/archive`          | class | Manually mark a semester archived.                                |

### Transactions

| Method | Path                              | Auth  | Description                                                                                           |
|--------|-----------------------------------|-------|-------------------------------------------------------------------------------------------------------|
| GET    | `/api/semesters/:id/transactions` | class | List transactions. Query: `kind`, `from`, `to`, `q`, `page`, `page_size`.                             |
| POST   | `/api/semesters/:id/transactions` | class | Create a transaction. Body: `{kind, amount_cents, source?, purpose?, item?, operator, occurred_at?}`. |
| GET    | `/api/transactions/:id`           | class | Fetch a single transaction.                                                                           |
| PUT    | `/api/transactions/:id`           | class | Update a transaction (full replacement). Same body shape as POST.                                     |
| DELETE | `/api/transactions/:id`           | class | Delete a transaction. The previous value is preserved in the audit log.                               |

### Reporting

| Method | Path                             | Auth  | Description                                                        |
|--------|----------------------------------|-------|--------------------------------------------------------------------|
| GET    | `/api/semesters/:id/report`      | class | Aggregated dashboard: `{summary, item_ranking}`. Cached in memory. |
| GET    | `/api/semesters/:id/export.xlsx` | class | Excel workbook (`Summary` + `Transactions` sheets) download.       |

## Response models

```jsonc
// Class
{ "id": 1, "name": "Class 3-2", "created_at": "2026-03-04T08:00:00Z" }

// Semester
{
  "id": 1, "class_id": 1, "name": "2026 Spring",
  "archived": false,
  "start_date": "2026-03-01", "end_date": "2026-06-30",
  "created_at": "2026-03-04T08:00:00Z"
}

// Transaction
{
  "id": 1, "semester_id": 1, "class_id": 1,
  "kind": "income", "amount_cents": 1500,
  "source": "Parent of Li Ming", "purpose": null,
  "item": "Handmade bookmark", "operator": "Zhang San",
  "occurred_at": "2026-03-05T09:30:00Z",
  "created_at": "2026-03-05T09:30:05Z"
}

// TransactionList
{ "data": [Transaction, ...], "total": 42, "page": 1, "page_size": 20 }

// Report
{
  "summary": {
    "total_income_cents": 150000,
    "total_expense_cents": 60000,
    "balance_cents": 90000,
    "income_count": 30,
    "expense_count": 12
  },
  "item_ranking": [
    { "item": "Handmade bookmark", "quantity": 25, "total_cents": 7500 }
  ]
}

// AuditLog
{
  "id": 1, "transaction_id": 1, "class_id": 1,
  "action": "create", "operator": "Zhang San",
  "payload_before": null,
  "payload_after": "{\"id\":1,...}",
  "occurred_at": "2026-03-05T09:30:05Z"
}
```