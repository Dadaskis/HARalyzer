@echo off
setlocal enabledelayedexpansion

echo ========================================
echo   HARalyzer Setup (Windows)
echo ========================================
echo.

where node >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Node.js not found. Install from https://nodejs.org/
    exit /b 1
)

where npm >nul 2>&1
if errorlevel 1 (
    echo [ERROR] npm not found.
    exit /b 1
)

where cargo >nul 2>&1
if errorlevel 1 (
    echo [WARN] Rust/cargo not found. Install from https://rustup.rs/
    echo        Required for building the Tauri desktop app.
) else (
    echo [OK] Rust/cargo found
)

echo.
echo Installing npm dependencies...
call npm install
if errorlevel 1 (
    echo [ERROR] npm install failed
    exit /b 1
)

echo.
echo ========================================
echo   Setup complete!
echo ========================================
echo.
echo Run the app in development mode:
echo   npm run tauri dev
echo.
echo Build a release:
echo   npm run tauri build
echo.

endlocal
