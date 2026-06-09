@echo off
setlocal enabledelayedexpansion

cd /d "%~dp0"

echo ========================================
echo   HARalyzer Export (Windows)
echo ========================================
echo.
echo Builds a release installer and copies it to export/
echo Recipients only need to run the setup .exe — no Node.js or Rust required.
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
    echo [ERROR] Rust/cargo not found. Install from https://rustup.rs/
    exit /b 1
)

echo [1/4] Installing npm dependencies...
call npm install
if errorlevel 1 (
    echo [ERROR] npm install failed
    exit /b 1
)

echo.
echo [2/4] Building release (this may take several minutes)...
call npm run tauri build
if errorlevel 1 (
    echo [ERROR] tauri build failed
    echo        Ensure Tauri prerequisites are installed:
    echo        https://tauri.app/start/prerequisites/
    exit /b 1
)

set "EXPORT_DIR=%~dp0export"
set "BUNDLE_DIR=%~dp0src-tauri\target\release\bundle"

echo.
echo [3/4] Preparing export folder...
if exist "%EXPORT_DIR%" rmdir /s /q "%EXPORT_DIR%"
mkdir "%EXPORT_DIR%"

set "COPIED=0"

if exist "%BUNDLE_DIR%\nsis\*.exe" (
    for %%F in ("%BUNDLE_DIR%\nsis\*.exe") do (
        copy /Y "%%~fF" "%EXPORT_DIR%\" >nul
        echo   Copied: %%~nxF
        set "COPIED=1"
    )
)

if exist "%BUNDLE_DIR%\msi\*.msi" (
    for %%F in ("%BUNDLE_DIR%\msi\*.msi") do (
        copy /Y "%%~fF" "%EXPORT_DIR%\" >nul
        echo   Copied: %%~nxF
        set "COPIED=1"
    )
)

if "!COPIED!"=="0" (
    echo [ERROR] No installer found in %BUNDLE_DIR%
    echo        Expected NSIS .exe or MSI under bundle\nsis or bundle\msi
    exit /b 1
)

echo.
echo [4/4] Writing export info...
(
    echo HARalyzer export
    echo ================
    echo Platform: Windows x64
    echo.
    echo DISTRIBUTING
    echo ------------
    echo Share the *-setup.exe file with others. They do not need Node.js, Rust,
    echo or any developer tools. The installer embeds the WebView2 bootstrapper
    echo and will install the Microsoft WebView2 runtime if it is missing.
    echo.
    echo An internet connection may be required on first install if WebView2 is
    echo not already present ^(Windows 10/11 usually has it^).
    echo.
    echo Optional: the .msi installer is an alternative if present in this folder.
    echo.
    echo BUILD MACHINE REQUIREMENTS
    echo --------------------------
    echo Node.js 18+, Rust, and Tauri Windows prerequisites were only needed to
    echo create this export — not to run the installed app.
) > "%EXPORT_DIR%\README.txt"

echo.
echo ========================================
echo   Export complete!
echo ========================================
echo.
echo Installers are in:
echo   %EXPORT_DIR%
echo.
dir /b "%EXPORT_DIR%"
echo.

endlocal
