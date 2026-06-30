# BazaarLog

A self-contained class charity sale ledger. Record income and expense
transactions for a class bazaar, switch between classes and semesters, view an
aggregated dashboard, export Excel, and audit every mutation - all from a
single `BazaarLog.exe` that runs on Windows 7.

> **Note**: This project was developed entirely by AI.

## What's in this repo

```
BazaarLog/
├── backend/                    Rust + Axum + sqlx backend
│   ├── Cargo.toml
│   ├── build.rs                ensures static/ exists for rust-embed
│   ├── .cargo/config.toml      pins +crt-static and the MSVC target
│   ├── migrations/
│   │   ├── sqlite_init.sql     default, zero-config database
│   │   └── postgres_init.sql   optional production database
│   └── src/
│       ├── main.rs             entry point, wires everything together
│       ├── config.rs           env-driven configuration
│       ├── db.rs               connection + schema bootstrap
│       ├── auth.rs             Argon2id per-class password auth
│       ├── cache.rs            in-memory TTL cache (Redis fallback)
│       ├── archive.rs          background archival of stale semesters
│       ├── excel.rs            rust_xlsxwriter report export
│       ├── metrics.rs          Prometheus /metrics endpoint
│       ├── error.rs            unified AppError -> HTTP mapping
│       ├── models.rs           request/response DTOs
│       ├── state.rs            shared AppState
│       └── handlers/
│           ├── mod.rs          router + shared helpers
│           ├── classes.rs      class CRUD + auth + audit log view
│           ├── semesters.rs    semester CRUD + archive
│           ├── transactions.rs transaction CRUD (audited)
│           ├── reports.rs      dashboard + Excel export
│           └── static_assets.rs rust-embed SPA fallback
├── frontend/                   React 18 + TypeScript + Vite + Tailwind
│   ├── package.json
│   ├── vite.config.ts          build target es2018, outputs to backend/static
│   └── src/
│       ├── main.tsx            React root
│       ├── App.tsx             top-level shell
│       ├── api.ts              fetch wrapper with class auth headers
│       ├── types.ts            DTOs mirroring the backend
│       ├── format.ts           money / date formatters
│       └── components/
│           ├── ClassLogin.tsx
│           ├── SemesterSwitcher.tsx
│           ├── TransactionsPage.tsx
│           ├── ReportPage.tsx
│           ├── AuditPage.tsx
│           └── Feedback.tsx
├── docs/
│   ├── api.md                  HTTP API reference
│   ├── build.md                how to build BazaarLog.exe
│   ├── windows7-run.md         how to run on Windows 7
│   ├── performance.md          optimization checklist
│   └── database.md             ER + schema overview
└── build.bat                   one-click build script
```

## Quick start

On a Windows 10/11 build machine with Rust (MSVC), Visual Studio Build Tools,
and Node.js 18+ installed:

```bat
build.bat
```

The output is `backend\target\x86_64-pc-windows-msvc\release\BazaarLog.exe`.
Copy that single file to a Windows 7 machine, double-click it, and open
<http://localhost:8080> in Chrome 109 or Firefox ESR 115.

See [docs/build.md](docs/build.md) for prerequisites and step-by-step
instructions, and [docs/windows7-run.md](docs/windows7-run.md) for runtime
configuration.

## Features

- **Class & semester isolation**: each class has its own password; semesters
  partition the ledger. Old semesters auto-archive after a configurable window.
- **Audited transactions**: every create / update / delete writes a row to
  `audit_logs` with the JSON snapshot of the before and after states, inside
  the same database transaction as the data change.
- **Dashboard**: total income / expense / balance, counts, and a top-20 item
  ranking. Cached in memory; invalidated on any mutation.
- **Excel export**: a `Summary` sheet plus a full `Transactions` sheet,
  generated server-side by `rust_xlsxwriter`.
- **Charts**: Chart.js pie (income vs expense) and bar (top items), styled in
  black/white/gray per the design spec.
- **Prometheus metrics**: `GET /metrics` exposes `bazaarlog_http_requests_total`.
- **Zero-config single-machine mode**: bundled SQLite, embedded frontend,
  statically linked CRT - the exe is the entire deployment.
- **Optional PostgreSQL**: set `BAZAARLOG_DATABASE_URL=postgres://...` to scale
  up; the same binary handles both backends.

## Documentation

- [Database design](docs/database.md)
- [HTTP API](docs/api.md)
- [Build guide](docs/build.md)
- [Windows 7 run guide](docs/windows7-run.md)
- [Performance checklist](docs/performance.md)

## Browser support

The frontend targets ES2018 and is verified against **Chrome 109** and
**Firefox ESR 115** - the two browsers still officially supported on Windows 7.
No polyfills are required for this set.

## License

Provided as-is for the BazaarLog class charity sale system.
