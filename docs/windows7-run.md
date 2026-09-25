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
   starting BazaarLog database_url=sqlite://bazaarlog.db?mode=rwc host=127.0.0.1 port=3000 is_sqlite=true
   BazaarLog listening; open http://localhost:3000
   ```

3. Open **Chrome 109** or **Firefox ESR 115** (the two browsers still supported
   on Windows 7) and navigate to <http://localhost:3000>.
4. The first visit shows the class selection screen. Click **New class** to
   create the first class, enter a name and a password (at least 8 characters),
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

By default, BazaarLog listens on `127.0.0.1:3000`. To change it, set environment
variables before launching:

```bat
set BAZAARLOG_HOST=0.0.0.0
set BAZAARLOG_PORT=9000
set BAZAARLOG_ALLOW_PLAINTEXT_LAN=1
BazaarLog.exe
```

Setting `BAZAARLOG_HOST=0.0.0.0` exposes the service on the LAN so other
devices on the same network can open <http://<this-pc-ip>:3000>. Be mindful
that BazaarLog has no transport-level security; only use this on a trusted
network.

> **Note**: binding to a non-loopback address over plaintext HTTP is refused
> unless `BAZAARLOG_ALLOW_PLAINTEXT_LAN=1` is also set. The class password and
> the session token travel in request headers and are readable by anyone
> sniffing the network segment, so the server fails closed by default. Put a
> TLS-terminating reverse proxy in front for anything beyond a trusted LAN.

### The port is refused with "access denied" (os error 10013)

On Windows, `WSAEACCES (os error 10013)` means the port is **reserved or held
exclusively**, not simply busy. Hyper-V, WSL2, and Docker Desktop reserve blocks
of TCP ports at boot, and those blocks are re-allocated on every reboot, so the
default port `3000` can work today and fail tomorrow with no process holding it.
Check which ranges are reserved:

```bat
netsh interface ipv4 show excludedportrange protocol=tcp
```

Pick a port outside every listed range and start BazaarLog on it:

```bat
set BAZAARLOG_PORT=8080
BazaarLog.exe
```

`os error 10048` (address already in use) is a different problem: a real process
is listening on the port. Find it with `netstat -ano | findstr :3000` and stop
it, or choose another port.

Note that changing the port also changes the URL to open in the browser, and
the Vite dev server proxy in `frontend/vite.config.ts` still points at `3000`
during frontend development.

### Where the database is written

`bazaarlog.db` and its `.audit.seal` sidecar are created in the **current
working directory**, because the default `BAZAARLOG_DATABASE_URL` is the
relative path `sqlite://bazaarlog.db?mode=rwc`. Launch the exe from a
dedicated data folder rather than from `target\...\release\`, which is wiped by
`cargo clean`. To keep the database somewhere fixed regardless of where the exe
is launched from, set an absolute path:

```bat
set BAZAARLOG_DATABASE_URL=sqlite://C:/BazaarLog/bazaarlog.db?mode=rwc
```

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

| Variable                       | Default                          | Description                                            |
|--------------------------------|----------------------------------|--------------------------------------------------------|
| `BAZAARLOG_DATABASE_URL`       | `sqlite://bazaarlog.db?mode=rwc` | `sqlite://` or `postgres://` connection string.        |
| `BAZAARLOG_HOST`               | `127.0.0.1`                      | Bind address. `0.0.0.0` needs `BAZAARLOG_ALLOW_PLAINTEXT_LAN=1`. |
| `BAZAARLOG_PORT`               | `3000`                           | Listen port.                                           |
| `BAZAARLOG_CACHE_TTL_SECS`     | `30`                             | TTL for the in-memory report (dashboard) cache. The audit-chain scan uses a fixed 5 s TTL instead; see README. |
| `BAZAARLOG_ARCHIVE_DAYS`       | `365`                            | Days after `end_date` before a semester auto-archives. |
| `BAZAARLOG_SESSION_TTL_HOURS`  | `4`                              | Login session lifetime in hours.                       |
| `BAZAARLOG_POW_DIFFICULTY`     | `4`                              | Leading zero hex chars required in the class-creation proof of work (1–16). |
| `BAZAARLOG_ENABLE_LEGACY_AUTH` | `0`                              | Set to `1` to re-enable the legacy `X-Class-Password` headers for pre-token scripts. |
| `BAZAARLOG_ALLOW_PLAINTEXT_LAN`| `0`                              | Set to `1` to allow binding a non-loopback address over plaintext HTTP. |
| `BAZAARLOG_HARDEN_DB_ACL`      | `1`                              | Windows only: tighten the ACL of the database and its `.audit.seal` file to the current user. `0` skips it. |
| `BAZAARLOG_METRICS_TOKEN`      | _(unset)_                        | When set, `/metrics` requires `Authorization: Bearer <token>`; otherwise it is loopback-only. |
| `RUST_LOG`                     | `info`                           | tracing filter (e.g. `debug,sqlx=warn`).               |

## Browser compatibility

The frontend targets ES2018 and uses only the small set of browser APIs that
Chrome 109 and Firefox ESR 115 support. No polyfills are required. Chart.js v4
ships its own compatibility layer and runs natively on both browsers.

If you must support an even older browser, set the Vite `build.target` in
`frontend\vite.config.ts` to `es2015` and add a `core-js` polyfill bundle, but
this is not required for the supported set.