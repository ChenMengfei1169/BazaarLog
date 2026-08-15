@echo off
REM BazaarLog one-click build script.
REM
REM Builds the embedded frontend and produces a release BazaarLog.exe in
REM backend\target\x86_64-pc-windows-gnu\release\.
REM SQLite is compiled in via the bundled libsqlite3-sys feature; the frontend
REM assets are embedded via rust-embed; MinGW runtime libs are statically
REM linked so the binary runs on Windows 7 with no extra DLLs.
REM
REM Prerequisites:
REM   - Rust stable with the x86_64-pc-windows-gnu target installed
REM       rustup default stable-x86_64-pc-windows-gnu
REM       rustup target add x86_64-pc-windows-gnu
REM   - MinGW-w64 toolchain (provides gcc and linker)
REM   - Node.js 18+ and npm
REM
REM Usage: run from the repository root.

setlocal enableextensions
cd /d "%~dp0"

echo ==^> [1/4] Building frontend
pushd frontend
call npm install --no-audit --no-fund
if errorlevel 1 (
    echo Frontend dependency installation failed.
    popd
    exit /b 1
)
call npm run build
if errorlevel 1 (
    echo Frontend build failed.
    popd
    exit /b 1
)
popd

echo ==^> [2/4] Verifying frontend assets in backend\static
if not exist backend\static\index.html (
    echo backend\static\index.html missing after frontend build.
    exit /b 1
)

echo ==^> [3/4] Building Rust backend release binary
pushd backend
cargo build --release --target x86_64-pc-windows-gnu
if errorlevel 1 (
    echo Rust build failed.
    popd
    exit /b 1
)
popd

echo ==^> [4/4] Done
echo Output: backend\target\x86_64-pc-windows-gnu\release\BazaarLog.exe
echo Copy the exe to a Windows 7 machine and double-click it.
echo Open http://localhost:3000 in Chrome 109 or Firefox ESR 115.

endlocal