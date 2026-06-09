#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" == "Darwin" ]]; then
  echo "On macOS, use setup-macos.sh instead:"
  echo "  chmod +x setup-macos.sh && ./setup-macos.sh"
  exit 0
fi

echo "========================================"
echo "  HARalyzer Setup (Arch Linux)"
echo "========================================"
echo

missing=()

command -v node >/dev/null 2>&1 || missing+=("nodejs")
command -v npm >/dev/null 2>&1 || missing+=("npm")
command -v cargo >/dev/null 2>&1 || missing+=("rust")

if [ ${#missing[@]} -gt 0 ]; then
  echo "Missing dependencies: ${missing[*]}"
  echo
  echo "Install on Arch Linux with:"
  echo "  sudo pacman -S nodejs npm base-devel curl"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo
  echo "Tauri also requires these (Arch):"
  echo "  sudo pacman -S webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg patchelf"
  echo
  read -r -p "Continue with npm install anyway? [y/N] " ans
  if [[ ! "$ans" =~ ^[Yy]$ ]]; then
    exit 1
  fi
else
  echo "[OK] node, npm, and cargo found"
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
