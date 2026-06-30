# Running BazaarLog on Windows 7

This guide covers the default single-machine SQLite deployment and the optional
PostgreSQL deployment.

## Single-machine mode (recommended)

This is the zero-config mode. No database server is needed; SQLite is compiled
into the exe.

1. Copy `BazaarLog.exe` to any folder on the Windows 7 machine, for example
   `C:\BazaarLog\BazaarLog.exe`.
2. Double-click `BazaarLog.exe`. A console window opens and prints:

   ```
   starting BazaarLog database_url=sqlite://bazaarlog.db?mode=rwc host=127.0.0.1 port=8080 is_sqlite=true
   BazaarLog listening; open http://localhost:8080
   ```

3. Open **Chrome 109** or **Firefox ESR 115** (the two browsers still supported
   on Windows 7) and navigate to <http://localhost:8080>.
4. The first visit shows the class selection screen. Click **New class** to
   create the first class, enter a name and a password (at least 4 characters),
   and optionally an operator name. Click **Create class**.
5. On the main screen, click **New** next to the semester selector, name it
   (e.g. `2026 Spring`), and create it.
6. Switch to the **Transactions** tab to record income and expense entries.
   Switch to the **Report** tab to view the dashboard and export Excel.

The SQLite database file `bazaarlog.db` is created next to the exe. To back up
the data, copy that file while the exe is stopped.

### Stopping the server

Close the console window, or focus it and press `Ctrl+C`.

### Changing the listen port

By default, BazaarLog listens on `127.0.0.1:8080`. To change it, set environment
variables before launching:

```bat
set BAZAARLOG_HOST=0.0.0.0
set BAZAARLOG_PORT=9000
BazaarLog.exe
```

Setting `BAZAARLOG_HOST=0.0.0.0` exposes the service on the LAN so other
devices on the same network can open <http://<this-pc-ip>:8080>. Be mindful
that BazaarLog has no transport-level security; only use this on a trusted
network.

## PostgreSQL mode (optional)

For multi-machine deployments or larger datasets, point BazaarLog at a
PostgreSQL server.

1. Create an empty database and a user, e.g.:

   ```sql
   CREATE DATABASE bazaarlog;
   CREATE USER bazaarlog WITH PASSWORD 'change-me';
   GRANT ALL ON DATABASE bazaarlog TO bazaarlog;
   ```

2. Apply the schema once:

   ```bat
   psql -h db-host -U bazaarlog -d bazaarlog -f backend\migrations\postgres_init.sql
   ```

   (BazaarLog also auto-applies the schema on first connect, but applying it
   explicitly lets you verify the user has DDL privileges.)

3. Launch with the connection string:

   ```bat
   set BAZAARLOG_DATABASE_URL=postgres://bazaarlog:change-me@db-host:5432/bazaarlog
   BazaarLog.exe
   ```

## Environment variables

| Variable                   | Default                          | Description                                            |
|----------------------------|----------------------------------|--------------------------------------------------------|
| `BAZAARLOG_DATABASE_URL`   | `sqlite://bazaarlog.db?mode=rwc` | `sqlite://` or `postgres://` connection string.        |
| `BAZAARLOG_HOST`           | `127.0.0.1`                      | Bind address. Use `0.0.0.0` for LAN access.            |
| `BAZAARLOG_PORT`           | `8080`                           | Listen port.                                           |
| `BAZAARLOG_CACHE_TTL_SECS` | `30`                             | TTL for the in-memory report cache.                    |
| `BAZAARLOG_ARCHIVE_DAYS`   | `365`                            | Days after `end_date` before a semester auto-archives. |
| `RUST_LOG`                 | `info`                           | tracing filter (e.g. `debug,sqlx=warn`).               |

## Browser compatibility

The frontend targets ES2018 and uses only the small set of browser APIs that
Chrome 109 and Firefox ESR 115 support. No polyfills are required. Chart.js v4
ships its own compatibility layer and runs natively on both browsers.

If you must support an even older browser, set the Vite `build.target` in
`frontend\vite.config.ts` to `es2015` and add a `core-js` polyfill bundle, but
this is not required for the supported set.
