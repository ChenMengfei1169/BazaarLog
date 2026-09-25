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
<http://localhost:3000> in Chrome 109 or Firefox ESR 115.

See [docs/build.md](docs/build.md) for prerequisites and step-by-step
instructions, and [docs/windows7-run.md](docs/windows7-run.md) for runtime
configuration.

## Features

- **Class & semester isolation**: each class has its own password; semesters
  partition the ledger. Old semesters auto-archive after a configurable window.
- **Audited writes**: every create / update / delete writes a row to
  `audit_logs` with the JSON snapshot of the before and after states, inside
  the same database transaction as the data change. This covers class creation
  and semester creation as well as transaction writes and semester archival, so
  a ledger cannot gain a class or a semester without leaving a trace. Class
  creation is public and the proof-of-work challenge carries no identity, so
  that entry records the operator as `anonymous`.
- **Dashboard**: total income / expense / balance, counts, and a top-20 item
  ranking. Cached in memory; invalidated on any mutation.
- **Excel export**: a `Summary` sheet plus a full `Transactions` sheet,
  generated server-side by `rust_xlsxwriter`. The workbook is built on the
  blocking pool and the export is bounded to 50,000 rows per semester; a larger
  semester is reported rather than silently truncated.
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

## Security

- **Session tokens**: login returns an opaque 256-bit token that the server
  stores **only as a SHA-256 digest** (never the plaintext), bound to the
  client IP and User-Agent captured at login. The TTL is configurable via
  `BAZAARLOG_SESSION_TTL_HOURS` (default 4 hours). A local process that reads
  the server's memory cannot reconstruct a usable token from the digests, and a
  stolen token is rejected outside its original browser context. Responses are
  sent with `Connection: close` so token-bearing buffers are reclaimed
  promptly instead of lingering in pooled connections.
- **Audit tamper-evidence**: every audit entry is chained to the previous one
  via SHA-256 (`prev_hash` / `entry_hash` columns); deleting or editing any
  row breaks every subsequent link. Because an internal chain cannot see a
  deletion of its own tail, the newest row is additionally anchored in a
  sidecar seal file (`<db>.audit.seal`) that records **the row id and its
  `entry_hash`**. Verification asks whether that row still exists unchanged, so
  it distinguishes "the log grew" from "the anchored row disappeared" — which
  means a truncated tail is still detected after a restart rather than being
  laundered by re-anchoring to the shortened log. `refresh_seal` refuses to
  overwrite an anchor it cannot confirm and logs an error instead. The seal is
  written even for an empty log, and every uncertain state (missing,
  unreadable, unparseable) is reported as *not verified* — the check fails
  closed. `GET /api/classes/:id/audit_logs/chain` reports integrity. The two
  halves of that answer are treated differently: the seal check is O(1) and
  runs on **every** request (so removing the anchor is reported immediately,
  never masked by a cache), while only the expensive full-table chain scan is
  cached — under a deliberately short TTL (5 s) rather than the default
  `BAZAARLOG_CACHE_TTL_SECS`, because any caching of it is directly detection
  latency for out-of-band edits. The UI distinguishes a broken link from a
  truncated tail. On Windows, the ACL of **both** the SQLite database file and
  its seal file is tightened to the current user (`BAZAARLOG_HARDEN_DB_ACL=0`
  to skip); the seal is re-hardened after every rewrite, since replacing the
  file would otherwise restore the directory's default permissions.
- **Brute-force protection**: the auth rate limiter is keyed by class id +
  client IP, and a global per-class failure counter locks out a class after 10
  failed logins in 5 minutes regardless of source address, so IP rotation
  cannot bypass it. Because the class id comes from the URL, an additional
  bucket keyed by source IP alone caps how fast a caller can make the server
  run Argon2 no matter how many class ids they invent.
- **Bounded password work**: Argon2 hashing and verification run on the
  blocking pool (`spawn_blocking`), never on the async runtime's worker
  threads, and a global semaphore caps how many run at once. Each Argon2id
  verification needs 64 MiB, so without the cap a flood of unauthenticated
  attempts against invented class ids could exhaust process memory. A request
  waits briefly for a permit (so a burst of legitimate logins still succeeds)
  and gets `429` only if the cap stays saturated.
- **Password hashing**: Argon2id with 64 MiB memory and 3 iterations; old
  hashes created with the previous weaker parameters keep verifying (the cost
  is read from each hash itself).
- **Class creation**: requires a proof-of-work challenge issued by
  `GET /api/classes/challenge` (single-use, 5-minute TTL), so bulk scripted
  creation costs CPU per attempt. Difficulty via `BAZAARLOG_POW_DIFFICULTY`
  (default 4).
- **Plaintext policy**: the server refuses to bind to a non-loopback address
  over plaintext HTTP unless `BAZAARLOG_ALLOW_PLAINTEXT_LAN=1` is set,
  because the legacy password headers are sniffable on the wire. Put a
  TLS-terminating reverse proxy in front for any network deployment.
- **Legacy password headers** (`X-Class-Id` / `X-Class-Password` /
  `X-Operator`) are disabled by default; set
  `BAZAARLOG_ENABLE_LEGACY_AUTH=1` only for scripts that predate session
  tokens.

### Known trust boundary

The database and the seal file live on the same machine as the server, so an
attacker with full local access (same user, or administrator) can still read or
rewrite them. Run the server under a dedicated low-privilege account and keep
the data directory ACLs tight; consider an externally stored, periodic audit
export for true forensic independence.

## Browser support

The frontend targets ES2018 and is verified against **Chrome 109** and
**Firefox ESR 115** - the two browsers still officially supported on Windows 7.
No polyfills are required for this set.

## License

Provided as-is for the BazaarLog class charity sale system.