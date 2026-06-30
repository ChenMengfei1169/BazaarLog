# Building BazaarLog.exe

This guide produces a single-file, self-contained `BazaarLog.exe` that runs
natively on Windows 7 SP1 and above. The build machine can be Windows 10/11;
the output binary targets Windows 7.

## 1. Install the build toolchain (one-time)

### Rust (GNU target)

1. Install `rustup` from <https://rustup.rs>.
2. Run the following in a terminal:

   ```bat
   rustup default stable-x86_64-pc-windows-gnu
   rustup target add x86_64-pc-windows-gnu
   ```

### MinGW-w64 toolchain

Install MinGW-w64 (provides the `gcc` compiler and linker). Options include:

- Via MSYS2: `pacman -S mingw-w64-x86_64-toolchain`
- Or download a standalone distribution (e.g. WinLibs) and add its `bin`
  directory to `PATH`.

MinGW is used to compile `libsqlite3-sys` (bundled source) and link the final
binary. The GNU target statically links `libgcc` / `libstdc++` /
`libwinpthread` by default, so no MinGW runtime DLLs are needed on the target
machine.

### Node.js

Install Node.js 18+ (LTS) from <https://nodejs.org/>. npm is included.

## 2. One-click build

Run this at the repository root:

```bat
build.bat
```

The script will:

1. Install frontend dependencies and run `npm run build`. Vite writes its
   output to `backend\static\` so `rust-embed` can embed it at compile time.
2. Verify that `backend\static\index.html` exists (confirming the frontend
   built successfully).
3. Run `cargo build --release --target x86_64-pc-windows-gnu` inside the
   `backend\` directory.

After the script finishes, the binary is at:

```
backend\target\x86_64-pc-windows-gnu\release\BazaarLog.exe
```

That single file is the complete deliverable: SQLite is compiled from source
via `libsqlite3-sys`'s `bundled` feature; frontend assets are embedded via
`rust-embed`; the MinGW runtime is statically linked, so the target machine
needs no additional DLLs.

## 3. Manual build (if you prefer to run each step yourself)

```bat
REM Frontend
cd frontend
npm install --no-audit --no-fund
npm run build
cd ..\backend

REM Backend
cargo build --release --target x86_64-pc-windows-gnu
```

## 4. Why the binary runs on a clean Windows 7 machine

- **Statically linked MinGW runtime**: the GNU target statically links
  `libgcc` / `libstdc++` / `libwinpthread` into the exe by default. No MinGW
  DLLs are needed on the target machine.
- **Embedded SQLite**: `libsqlite3-sys` with the `bundled` feature compiles
  SQLite from source. No system SQLite DLL is required.
- **Embedded frontend**: `rust-embed` bakes everything under
  `backend\static\` into the binary as bytes. No external web server or
  asset directory is needed.
- **Single-machine mode avoids TLS**: the default `sqlite://` connection
  string does not pull in any system certificate store.
- **Tokio + rusts**: when configured with a `postgres://` URL, connections
  use `rustls` (a pure-Rust TLS stack), avoiding the `schannel` bindings
  that are problematic on Windows 7.

## 5. Verify the binary

Copy `BazaarLog.exe` to a folder on the target machine and double-click it.
A console window opens showing the listen address. Open
<http://localhost:8080> in Chrome 109 or Firefox ESR 115. On first run,
`bazaarlog.db` is created in the current directory and the database schema
is initialized.

To stop the server, close the console window or press `Ctrl+C`.