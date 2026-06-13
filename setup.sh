#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "On macOS, use setup-macos.sh instead:"
  echo "  chmod +x setup-macos.sh && ./setup-macos.sh"
  exit 0
fi

echo "========================================"
echo "  HARalyzer Setup (Linux)"
echo "========================================"
echo

missing=()

command -v node >/dev/null 2>&1 || missing+=("nodejs")
command -v npm >/dev/null 2>&1 || missing+=("npm")
command -v cargo >/dev/null 2>&1 || missing+=("rust")
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  missing+=("webkit2gtk-4.1")
fi

if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing dependencies: ${missing[*]}"
  echo
  echo "Install on Arch Linux:"
  echo "  sudo pacman -S nodejs npm base-devel curl"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "  sudo pacman -S webkit2gtk-4.1 gtk3 librsvg patchelf"
  echo
  echo "Install on Debian/Ubuntu:"
  echo "  sudo apt install nodejs npm build-essential curl pkg-config"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libappindicator3-dev librsvg2-dev patchelf"
  echo
  echo "Install on Fedora:"
  echo "  sudo dnf install nodejs npm gcc-c++ curl"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "  sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel patchelf"
  echo
  read -r -p "Continue with npm install anyway? [y/N] " ans
  if [[ ! "$ans" =~ ^[Yy]$ ]]; then
    exit 1
  fi
else
  echo "[OK] node, npm, cargo, and webkit2gtk-4.1 found"
fi

echo
echo "Installing npm dependencies..."
npm install

echo
echo "========================================"
echo "  Setup complete!"
echo "========================================"
echo
echo "Run the app in development mode:"
echo "  npm run tauri dev"
echo
echo "Build a release:"
echo "  npm run tauri build"
echo
