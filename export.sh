#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "Tip: on macOS, prefer the dedicated script:"
  echo "  chmod +x export-macos.sh && ./export-macos.sh"
  echo
fi

echo "========================================"
echo "  HARalyzer Export"
echo "========================================"
echo
echo "Builds a release bundle and copies it to export/"
echo "Recipients do not need Node.js or Rust to run the app."
echo

missing=()
command -v node >/dev/null 2>&1 || missing+=("nodejs")
command -v npm >/dev/null 2>&1 || missing+=("npm")
command -v cargo >/dev/null 2>&1 || missing+=("rust/cargo")

if [ ${#missing[@]} -gt 0 ]; then
  echo "[ERROR] Missing build dependencies: ${missing[*]}"
  echo
  echo "Arch Linux example:"
  echo "  sudo pacman -S nodejs npm base-devel curl webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg patchelf"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo
  echo "See https://tauri.app/start/prerequisites/"
  exit 1
fi

echo "[1/4] Installing npm dependencies..."
npm install

echo
echo "[2/4] Building release (this may take several minutes)..."
npm run tauri build

EXPORT_DIR="$ROOT/export"
BUNDLE_DIR="$ROOT/src-tauri/target/release/bundle"

echo
echo "[3/4] Preparing export folder..."
rm -rf "$EXPORT_DIR"
mkdir -p "$EXPORT_DIR"

COPIED=0
copy_glob() {
  local dir="$1"
  local pattern="$2"
  local label="$3"
  if compgen -G "$dir/$pattern" >/dev/null 2>&1; then
    cp -v "$dir"/$pattern "$EXPORT_DIR/"
    COPIED=1
    echo "  ($label)"
  fi
}

if [[ "$OSTYPE" == "darwin"* ]]; then
  copy_glob "$BUNDLE_DIR/dmg" "*.dmg" "macOS disk image"
  copy_glob "$BUNDLE_DIR/macos" "*.app.tar.gz" "macOS app archive"
elif [[ "$OSTYPE" == "linux-gnu"* ]] || [[ "$OSTYPE" == "linux"* ]]; then
  copy_glob "$BUNDLE_DIR/appimage" "*.AppImage" "Linux AppImage (recommended — no install)"
  copy_glob "$BUNDLE_DIR/deb" "*.deb" "Debian/Ubuntu package"
  copy_glob "$BUNDLE_DIR/rpm" "*.rpm" "Fedora/RHEL package"
else
  copy_glob "$BUNDLE_DIR/nsis" "*.exe" "Windows NSIS installer"
  copy_glob "$BUNDLE_DIR/msi" "*.msi" "Windows MSI installer"
fi

if [ "$COPIED" -eq 0 ]; then
  echo "[ERROR] No bundle artifacts found in $BUNDLE_DIR"
  echo "        Run this script on the target OS, or inspect bundle/ after build."
  exit 1
fi

PLATFORM="unknown"
if [[ "$OSTYPE" == "darwin"* ]]; then
  PLATFORM="macOS"
elif [[ "$OSTYPE" == "linux"* ]]; then
  PLATFORM="Linux"
else
  PLATFORM="Windows"
fi

echo
echo "[4/4] Writing export info..."
cat > "$EXPORT_DIR/README.txt" <<EOF
HARalyzer export
================
Platform: $PLATFORM

DISTRIBUTING
------------
Share the files in this folder. Recipients do not need Node.js, Rust, or
developer tools.

Linux (AppImage):
  chmod +x *.AppImage && ./HARalyzer_*.AppImage

Linux (.deb):
  sudo dpkg -i haralyzer_*.deb

macOS (.dmg):
  Open the .dmg and drag HARalyzer to Applications.

Windows (*-setup.exe):
  Run the installer. WebView2 is installed automatically if missing.

BUILD MACHINE REQUIREMENTS
--------------------------
Node.js 18+, Rust, and Tauri OS prerequisites were only needed to create
this export — not to run the distributed app.
EOF

echo
echo "========================================"
echo "  Export complete!"
echo "========================================"
echo
echo "Artifacts in: $EXPORT_DIR"
ls -la "$EXPORT_DIR"
echo
