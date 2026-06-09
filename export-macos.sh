#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

echo "========================================"
echo "  HARalyzer Export (macOS)"
echo "========================================"
echo
echo "Builds a release .dmg (and .app) and copies them to export/"
echo "Recipients do not need Node.js or Rust to run the app."
echo

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[ERROR] This script must be run on macOS."
  echo "        macOS apps cannot be cross-compiled from Windows or Linux."
  echo "        Use export.bat on Windows or export.sh on Linux instead."
  exit 1
fi

missing=()
command -v node >/dev/null 2>&1 || missing+=("Node.js")
command -v npm >/dev/null 2>&1 || missing+=("npm")
command -v cargo >/dev/null 2>&1 || missing+=("Rust (cargo)")

if ! xcode-select -p >/dev/null 2>&1; then
  echo "[ERROR] Xcode Command Line Tools are required."
  echo "        Run: xcode-select --install"
  exit 1
fi

if [ ${#missing[@]} -gt 0 ]; then
  echo "[ERROR] Missing build dependencies: ${missing[*]}"
  echo
  echo "Run setup first:"
  echo "  chmod +x setup-macos.sh && ./setup-macos.sh"
  echo
  echo "Or install manually with Homebrew:"
  echo "  brew install node"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

ARCH="$(uname -m)"
echo "Building for: macOS ($ARCH)"
echo

echo "[1/4] Installing npm dependencies..."
npm install

echo
echo "[2/4] Building release (this may take several minutes)..."
npm run tauri build

EXPORT_DIR="$ROOT/export"
BUNDLE_DIR="$ROOT/src-tauri/target/release/bundle"
DMG_DIR="$BUNDLE_DIR/dmg"
MACOS_DIR="$BUNDLE_DIR/macos"

echo
echo "[3/4] Preparing export folder..."
rm -rf "$EXPORT_DIR"
mkdir -p "$EXPORT_DIR"

COPIED=0

if compgen -G "$DMG_DIR"/*.dmg >/dev/null 2>&1; then
  cp -v "$DMG_DIR"/*.dmg "$EXPORT_DIR/"
  COPIED=1
  echo "  (macOS disk image — share this with others)"
fi

if compgen -G "$MACOS_DIR"/*.app >/dev/null 2>&1; then
  for app in "$MACOS_DIR"/*.app; do
    app_name="$(basename "$app")"
    echo "  Copying $app_name ..."
    cp -R "$app" "$EXPORT_DIR/"
    COPIED=1
  done
  echo "  (application bundle — for local testing or advanced distribution)"
fi

if compgen -G "$MACOS_DIR"/*.app.tar.gz >/dev/null 2>&1; then
  cp -v "$MACOS_DIR"/*.app.tar.gz "$EXPORT_DIR/"
  COPIED=1
fi

if [ "$COPIED" -eq 0 ]; then
  echo "[ERROR] No bundle artifacts found under:"
  echo "        $BUNDLE_DIR"
  echo "        Expected .dmg in bundle/dmg and/or .app in bundle/macos"
  exit 1
fi

echo
echo "[4/4] Writing export info..."
cat > "$EXPORT_DIR/README.txt" <<EOF
HARalyzer export (macOS)
========================
Architecture: $ARCH

DISTRIBUTING
------------
Share the .dmg file with others. They do not need Node.js, Rust, or
developer tools.

Install from .dmg:
  1. Double-click the .dmg file
  2. Drag HARalyzer into Applications
  3. Eject the disk image

First launch (unsigned / local builds):
  macOS may block apps that are not notarized. If you see a security
  warning, right-click HARalyzer in Applications and choose Open,
  then confirm Open in the dialog. After the first launch, double-click
  works normally.

Open HAR files:
  Use File > Open HAR in the app, drag-and-drop a .har onto the window,
  or double-click a .har file if HARalyzer is the default app for .har.

Optional .app folder:
  The HARalyzer.app bundle is included for testing. You can run it
  directly from this folder without installing to Applications.

BUILD MACHINE REQUIREMENTS
--------------------------
Node.js 18+, Rust, and Xcode Command Line Tools were only needed to
create this export — not to run the distributed app.

Build again on a Mac:
  chmod +x export-macos.sh && ./export-macos.sh
EOF

echo
echo "========================================"
echo "  Export complete!"
echo "========================================"
echo
echo "Artifacts in: $EXPORT_DIR"
ls -la "$EXPORT_DIR"
echo
