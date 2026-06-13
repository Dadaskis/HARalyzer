#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

echo "========================================"
echo "  HARalyzer Setup (macOS)"
echo "========================================"
echo

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "[ERROR] This script is for macOS only."
  echo "        On Linux use: chmod +x setup.sh && ./setup.sh"
  echo "        On Windows use: setup.bat"
  exit 1
fi

echo "This installs npm dependencies for local development and building."
echo "End users who receive a .dmg from export/ do not need any of this."
echo

missing=()
warnings=()

if ! xcode-select -p >/dev/null 2>&1; then
  warnings+=("Xcode Command Line Tools")
fi

command -v node >/dev/null 2>&1 || missing+=("Node.js")
command -v npm >/dev/null 2>&1 || missing+=("npm")
command -v cargo >/dev/null 2>&1 || missing+=("Rust (cargo)")

if [ ${#warnings[@]} -gt 0 ]; then
  echo "[WARN] Missing: ${warnings[*]}"
  echo
  echo "Install Xcode Command Line Tools (required by Tauri on macOS):"
  echo "  xcode-select --install"
  echo
  echo "If a dialog appears, click Install and wait for it to finish."
  echo
fi

if [ ${#missing[@]} -gt 0 ]; then
  echo "[WARN] Missing: ${missing[*]}"
  echo
  echo "Recommended install with Homebrew (https://brew.sh):"
  echo "  /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
  echo "  brew install node"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "  source \"\$HOME/.cargo/env\""
  echo
  echo "Or install Node.js from https://nodejs.org/ and Rust from https://rustup.rs/"
  echo
  read -r -p "Continue with npm install anyway? [y/N] " ans
  if [[ ! "$ans" =~ ^[Yy]$ ]]; then
    exit 1
  fi
else
  echo "[OK] node, npm, and cargo found"
  if xcode-select -p >/dev/null 2>&1; then
    echo "[OK] Xcode Command Line Tools found"
  fi
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
echo "Build a shareable .dmg for others:"
echo "  chmod +x export-macos.sh && ./export-macos.sh"
echo
echo "Tauri macOS prerequisites:"
echo "  https://tauri.app/start/prerequisites/"
echo
