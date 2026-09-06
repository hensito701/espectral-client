#!/usr/bin/env bash
# ==========================================================================
# build-linux-deb.sh — build the EspectralClient Linux .deb on native Ubuntu.
#
# Prerequisites (one-time, root):
#   1. Rust via rustup:            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
#      (must be on PATH; Tauri v2 needs Rust >= 1.77.2)
#   2. System webview + build deps (Ubuntu 24.04 / Debian 12):
#      apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
#        librsvg2-dev libssl-dev build-essential curl wget file \
#        libgtk-3-dev libxdo-dev
#   3. Node 24 + npm deps:         nvm use 24 && npm install
#
# Output:
#   src-tauri/target/release/bundle/deb/espectral-client_<ver>_amd64.deb
#
# NOTE on compat: release .debs are built on the OLDEST supported base
# (Debian 12 / Ubuntu 22.04 container) so the glibc floor stays low — a
# .deb built on newer Ubuntu may refuse to install on Debian stable or
# Ubuntu 24.04 LTS. One .deb covers both Ubuntu 24.04 and Debian.
#
# The bundle is lean: only the engine + the extract-zip runtime deps are
# staged (scripts/stage-tauri-resources.mjs), not the whole node_modules.
# ==========================================================================
set -euo pipefail
cd "$(dirname "$0")/.."

# WebView runtime headers must be present — the tauri bundler links against
# webkit2gtk at compile time and the .deb declares the runtime dep.
if ! pkg-config --exists webkit2gtk-4.1 gtk+-3.0 2>/dev/null; then
  echo "ERROR: webkit2gtk-4.1 / gtk+-3.0 dev headers missing." >&2
  echo "Run: apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev" >&2
  exit 1
fi
if ! command -v rustc >/dev/null 2>&1; then
  echo "ERROR: rustc not on PATH (install via rustup)" >&2
  exit 1
fi

# Guard: tauri.conf.json must be STRICT JSON — Tauri's parser rejects
# // comments ("key must be a string"), and failing at cargo build time
# wastes a full compile cycle. Fail here, early, with the reason.
if ! node -e "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8'))" 2>/dev/null; then
  echo "ERROR: src-tauri/tauri.conf.json is not strict JSON (// comments are not allowed)" >&2
  exit 1
fi

echo "[build] staging engine resources…"
export STAGE_NODE_PLATFORM=linux
node scripts/stage-tauri-resources.mjs

# Updater signing: without TAURI_SIGNING_PRIVATE_KEY, `tauri build` errors out
# after the bundle step ("A public key has been found, but no private key")
# and no .sig files are produced. Key file is gitignored
# (.keys/espectral.key); password empty by design on the build box.
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -f ".keys/espectral.key" ]; then
  export TAURI_SIGNING_PRIVATE_KEY="$(cat .keys/espectral.key)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
fi

echo "[build] tauri build (.deb)…"
npx tauri build --bundles deb

echo "[build] done:"
ls -lh src-tauri/target/release/bundle/deb/*.deb

# The updater manifest is not built here: it must point at the immutable
# release URL and carry the signature of the artifact that actually ships.
# A local build only needs the signed .deb, which the bundler just wrote
# next to the .sig.
