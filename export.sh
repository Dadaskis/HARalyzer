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
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  missing+=("webkit2gtk-4.1 (dev)")
fi

if [ ${#missing[@]} -gt 0 ]; then
  echo "[ERROR] Missing build dependencies: ${missing[*]}"
  echo
  echo "Install on Arch Linux:"
  echo "  sudo pacman -S nodejs npm base-devel curl webkit2gtk-4.1 gtk3 librsvg patchelf"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo
  echo "Install on Debian/Ubuntu:"
  echo "  sudo apt install nodejs npm build-essential curl pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev librsvg2-dev patchelf"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo
  echo "Install on Fedora:"
  echo "  sudo dnf install nodejs npm gcc-c++ curl webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel patchelf"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo
  echo "See https://tauri.app/start/prerequisites/"
  exit 1
fi

echo "[1/4] Installing npm dependencies..."
npm install

echo
echo "[2/4] Building release (this may take several minutes)..."
export APPIMAGE_EXTRACT_AND_RUN=1

if ! npm run tauri build -- --bundles deb,rpm; then
  echo "[WARN] Tauri build failed. Trying with all bundles..."
  npm run tauri build
fi

BUNDLE_DIR="$ROOT/src-tauri/target/release/bundle"
APPIMAGE_DIR="$BUNDLE_DIR/appimage"

# If AppImage wasn't built (e.g., linuxdeploy needs FUSE), try appimagetool
if ! compgen -G "$APPIMAGE_DIR/*.AppImage" >/dev/null 2>&1 && [ -d "$APPIMAGE_DIR/HARalyzer.AppDir" ]; then
  APPIMAGETOOL=$(find /tmp -name "appimagetool" -type f 2>/dev/null | head -1)
  if [ -n "$APPIMAGETOOL" ]; then
    echo "[INFO] Building AppImage from existing AppDir via appimagetool..."
    cp "$APPIMAGE_DIR/HARalyzer.AppDir/HARalyzer.png" "$APPIMAGE_DIR/HARalyzer.AppDir/haralyzer.png" 2>/dev/null || true
    APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" "$APPIMAGE_DIR/HARalyzer.AppDir" "$APPIMAGE_DIR/HARalyzer_1.1.0_amd64.AppImage" 2>&1
  else
    echo "[WARN] AppImage not built (linuxdeploy needs FUSE). Install fuse2 and run:"
    echo "  npm run tauri build -- --bundles appimage"
  fi
fi

EXPORT_DIR="$ROOT/export"

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
