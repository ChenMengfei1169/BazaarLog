# Performance optimization checklist

Every optimization below is implemented in the shipped `BazaarLog.exe`. Each
entry explains what was done and why it helps.

## Database

1. **Composite indexes on hot paths**
   - `idx_transactions_semester_time (semester_id, occurred_at DESC)` backs the
     default listing sorted by recency.
   - `idx_transactions_class_kind_time (class_id, kind, occurred_at DESC)`
     backs the filtered dashboard queries.
   - `idx_transactions_semester_item (semester_id, item)` backs the item
     ranking aggregation.
   - `idx_audit_logs_transaction (transaction_id, occurred_at DESC)` keeps the
     audit lookup from scanning the whole log.
   - `idx_semesters_class_archived (class_id, archived)` keeps the switcher
     cheap.
   - Without these, every dashboard render would do full table scans as the
     ledger grows.

2. **Connection pool with bounded size**
   - `AnyPoolOptions::new().max_connections(8)` reuses connections and caps
     concurrent DB handles, so a burst of requests does not exhaust Postgres
     `max_connections` or spawn unbounded SQLite writers.

3. **Prepared-statement cache**
   - SQLX caches the prepared statement for each `(sql, connection)` pair, so
     repeated calls to `list_transactions` or `get_report` skip parsing and
     planning on the database side.

4. **Transactional writes with audit on the same transaction**
   - `pool.begin()` is used for create/update/delete. The audit row is inserted
     on the same `&mut AnyConnection`, then `commit()` runs once. Either the
     data change and the audit row land together or neither does - and the
     client never sees a partial mutation.

5. **Single SQL code path for SQLite and PostgreSQL**
   - Both schemas use `TEXT` for RFC3339 timestamps and `INTEGER/SMALLINT` for
     booleans. The handlers run the same parameterized SQL against either
     backend, eliminating per-database branches and the bugs they bring.

## Async runtime

1. **Tokio multithreaded runtime**
   - `#[tokio::main]` runs the scheduler across all CPU cores. Database I/O is
     fully concurrent; a slow `report` request never blocks the listing
     endpoint.

2. **Background archival task**
   - `archive::spawn` runs a sweep at startup and every six hours. The sweep
     issues a single `UPDATE ... WHERE end_date < ?` so it is O(rows matched)
     and does not lock the table for a scan.

## Caching

1. **In-memory TTL cache for reports**
   - `cache::Cache` stores the serialized `Report` JSON keyed by
     `report:semester:{id}`. A second dashboard load within the TTL is a
     HashMap lookup, not a SQL aggregation.
   - Every mutating endpoint calls `cache.invalidate(report_cache_key(...))`
     on commit, so a stale dashboard is never served.
   - The cache is in-process and lock-held briefly; no Redis dependency is
     required. To plug in Redis, replace `Cache::get/set` with a Redis client
     - the call sites are already key/value shaped.

## HTTP layer

1. **gzip compression**
   - `CompressionLayer::new()` from `tower-http` negotiates gzip for clients
     that advertise it. The Excel export and JSON lists compress well, cutting
     wire bytes on slow classroom Wi-Fi.

2. **Hashed-asset immutable caching**
   - The static handler sends `Cache-Control: public, max-age=31536000,
     immutable` for every asset except `index.html`. Vite emits hashed
     filenames, so the browser never re-fetches a JS chunk it already has.

3. **Prometheus metrics**
   - `/metrics` exposes `bazaarlog_http_requests_total{method=...}`. Method
     labels (not path labels) keep cardinality bounded. Wire this into
     Grafana for live visibility.

## Binary

1. **Fat LTO + single codegen unit**
   - `[profile.release] lto = "fat", codegen-units = 1` lets LLVM inline
     across crates and apply whole-program optimization. The binary is smaller
     and the hot paths are tighter.

2. **Symbol stripping**
   - `strip = "symbols"` removes the symbol table from the release binary,
     shrinking it from ~30 MB to ~10 MB. Stack traces still work via the
     PDB emitted alongside if you keep it.

3. **Static CRT linking**
   - `+crt-static` bakes the Visual C++ runtime into the exe. The Win7 target
     machine needs no `vcruntime140.dll` install, and the binary does not pay
     the per-call indirection through the DLL.

4. **Bundled SQLite**
   - `libsqlite3-sys` with the `bundled` feature compiles SQLite from source
     into the binary. No system SQLite lookup, no DLL load at startup, and we
     get a known SQLite version on every platform.
