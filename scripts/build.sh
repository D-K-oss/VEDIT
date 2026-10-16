#!/usr/bin/env bash
# ============================================================
#  VEDIT — Linux / macOS build script
#
#  Usage:  ./scripts/build.sh
#  Output: target/release/vedit  (uses system ffmpeg from PATH)
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== VEDIT build (linux/macOS) ==="

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust not found. Install from https://rustup.rs" >&2
  exit 1
fi

if ! command -v ffmpeg >/dev/null 2>&1 || ! command -v ffprobe >/dev/null 2>&1; then
  echo "NOTE: ffmpeg/ffprobe not found on PATH — install them for media support:" >&2
  echo "  Debian/Ubuntu: sudo apt install ffmpeg" >&2
  echo "  Fedora:        sudo dnf install ffmpeg" >&2
  echo "  macOS (brew):  brew install ffmpeg" >&2
fi

echo "[1/2] cargo build --release -p vedit-gui"
cargo build --release -p vedit-gui

echo "[2/2] done"
echo "Run with:  ./target/release/vedit"
