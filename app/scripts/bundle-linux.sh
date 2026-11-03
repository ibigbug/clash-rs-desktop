#!/usr/bin/env bash
# Build deb + AppImage on Linux. Needs the Tauri system packages.
set -euo pipefail
cd "$(dirname "$0")/.."

pnpm build
node scripts/fetch-core.mjs
cargo build --release -p clash-rs-desktop-service
target_dir="${CARGO_TARGET_DIR:-../target}"
mkdir -p src-tauri/linux/bin
install -m 755 "$target_dir/release/clash-rs-desktop-service" src-tauri/linux/bin/

pnpm tauri build --bundles deb,appimage --config src-tauri/tauri.bundle-linux.json "$@"
