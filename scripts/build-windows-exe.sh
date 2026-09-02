#!/usr/bin/env bash
# ==========================================================================
# build-windows-exe.sh — cross-compile the EspectralClient Windows exe + NSIS
# installer from this WSL2 (or any Linux with the same toolchain).
#
# Prerequisites (one-time, no root needed):
#   1. Rust via rustup:            sh rustup-init.sh -y --profile minimal
#      (must be on PATH; Tauri v2 needs Rust >= 1.77.2)
#   2. mingw-w64 (extracted debs in ~/mingw): gcc-mingw-w64-x86-64-posix,
#      binutils-mingw-w64-x86-64, mingw-w64-x86-64-dev (+ deps).
#      Provides x86_64-w64-mingw32-gcc, windres, ar, dlltool.
#   3. NSIS 3.x (extracted debs in ~/nsis): nsis, nsis-common.
#      Provides makensis. This script expects ~/bin/makensis to be a wrapper
#      that sets NSISDIR to the extracted tree (the tauri bundler resolves
#      stubs via NSISDIR inherited from the child env).
#   4. Rust windows target:        rustup target add x86_64-pc-windows-gnu
#
# Output:
#   src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/EspectralClient_<ver>_x64-setup.exe
#   src-tauri/target/x86_64-pc-windows-gnu/release/espectral-client.exe (portable)
#
# The bundle is lean: only the engine + the extract-zip runtime deps are
# staged (scripts/stage-tauri-resources.mjs), not the 92 MB node_modules.
# ==========================================================================
set -euo pipefail
cd "$(dirname "$0")/.."

# NOTE: this nvm node only powers the Linux-side build toolchain (tauri CLI,
# vite, staging). The SHIPPED Windows node.exe version is pinned separately in
# package.json `espectral.bundledNode` — do NOT bump this path to ship a newer
# runtime (24.16.0 stalls extract-zip at game-launch natives extraction).
export PATH="$HOME/.nvm/versions/node/v24.16.0/bin:$HOME/.cargo/bin:$HOME/mingw/usr/bin:$HOME/bin:$HOME/nsis/usr/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
export NSISDIR="${NSISDIR:-$HOME/nsis/usr/share/nsis}"

# The tauri bundler invokes makensis from PATH; NSISDIR must be inherited by
# the child so stub resolution finds the extracted tree (the binary's baked-in
# default is /usr/share/nsis, which is not writable without root).
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "ERROR: mingw-w64 compiler not on PATH (look in ~/mingw/usr/bin)" >&2
  exit 1
fi
if ! command -v makensis >/dev/null 2>&1; then
  echo "ERROR: makensis not on PATH (look in ~/nsis/usr/bin or ~/bin)" >&2
  exit 1
fi

# Guard: tauri.conf.json must be STRICT JSON — Tauri's parser rejects
# // comments ("key must be a string"), and failing at cargo build time
# wastes a full compile cycle. Fail here, early, with the reason.
if ! node -e "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8'))" 2>/dev/null; then
  echo "ERROR: src-tauri/tauri.conf.json is not strict JSON (// comments are not allowed)" >&2
  exit 1
fi

# Auto-heal: node_modules is normally installed from Windows, so it ships only
# the win32 @tauri-apps/cli native binary, which WSL's node cannot load.
# Restore the Linux CLI package WITHOUT npm when possible — an npm install here
# re-reconciles optional deps and prunes Windows-native binaries (rollup),
# breaking the next Windows-side `npm run build`. First run installs via npm
# and caches the whole package dir in build/tauri-cli-linux/ for later runs.
CLI_DIR="node_modules/@tauri-apps/cli-linux-x64-gnu"
CLI_NODE="$CLI_DIR/cli.linux-x64-gnu.node"
CLI_CACHE="build/tauri-cli-linux"
if [ ! -f "$CLI_NODE" ]; then
  if [ -f "$CLI_CACHE/cli.linux-x64-gnu.node" ] && [ -f "$CLI_CACHE/package.json" ]; then
    echo "[build] restoring Linux tauri CLI from cache…"
    mkdir -p "$CLI_DIR"
    find "$CLI_CACHE" -maxdepth 1 -type f -exec cp {} "$CLI_DIR/" \;
  else
    echo "[build] installing Linux tauri CLI native dep (first run — caches package)…"
    CLI_V="$(node -p "require('./node_modules/@tauri-apps/cli/package.json').version")"
    npm i --no-save --ignore-scripts "@tauri-apps/cli-linux-x64-gnu@$CLI_V"
  fi
fi
ROLLUP_DIR="node_modules/@rollup/rollup-linux-x64-gnu"
if [ ! -f "$ROLLUP_DIR/package.json" ]; then
  if [ -f "$CLI_CACHE/rollup-linux-x64-gnu/package.json" ]; then
    echo "[build] restoring Linux rollup native from cache…"
    mkdir -p "$ROLLUP_DIR"
    cp "$CLI_CACHE/rollup-linux-x64-gnu"/* "$ROLLUP_DIR/"
  else
    echo "[build] installing Linux rollup native (first run — caches package)…"
    R_V="$(node -p "require('./node_modules/rollup/package.json').version")"
    npm i --no-save --ignore-scripts "@rollup/rollup-linux-x64-gnu@$R_V"
  fi
fi
if [ -f "$ROLLUP_DIR/package.json" ] && [ ! -f "$CLI_CACHE/rollup-linux-x64-gnu/package.json" ]; then
  mkdir -p "$CLI_CACHE/rollup-linux-x64-gnu"
  cp "$ROLLUP_DIR"/* "$CLI_CACHE/rollup-linux-x64-gnu/"
fi
ESBUILD_DIR="node_modules/@esbuild/linux-x64"
if [ ! -f "$ESBUILD_DIR/package.json" ]; then
  if [ -f "$CLI_CACHE/esbuild-linux-x64/package.json" ]; then
    echo "[build] restoring Linux esbuild from cache…"
    mkdir -p "$ESBUILD_DIR"
    cp -r "$CLI_CACHE/esbuild-linux-x64"/* "$ESBUILD_DIR/" 2>/dev/null || true
    find "$CLI_CACHE/esbuild-linux-x64" -mindepth 1 -maxdepth 1 -type d -exec cp -r {} "$ESBUILD_DIR/" \;
  else
    echo "[build] installing Linux esbuild (first run — caches package)…"
    E_V="$(node -p "require('./node_modules/esbuild/package.json').version")"
    npm i --no-save --ignore-scripts "@esbuild/linux-x64@$E_V"
  fi
fi
if [ -f "$ESBUILD_DIR/package.json" ] && [ ! -f "$CLI_CACHE/esbuild-linux-x64/package.json" ]; then
  mkdir -p "$CLI_CACHE/esbuild-linux-x64"
  cp -r "$ESBUILD_DIR"/* "$CLI_CACHE/esbuild-linux-x64/"
fi
echo "[build] staging engine resources…"
node scripts/stage-tauri-resources.mjs

# Updater signing: without TAURI_SIGNING_PRIVATE_KEY, `tauri build` errors out
# after the bundle step ("A public key has been found, but no private key")
# and no .sig files are produced. The CLI wants the KEY CONTENT (the _PATH
# variant is not honored by tauri build in cli 2.11.4). Key file is
# gitignored (.keys/espectral.key); password empty by design on the build box.
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -f ".keys/espectral.key" ]; then
  export TAURI_SIGNING_PRIVATE_KEY="$(cat .keys/espectral.key)"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
  echo "[build] updater signing: using .keys/espectral.key"
fi

echo "[build] tauri build (x86_64-pc-windows-gnu)…"
npx tauri build --target x86_64-pc-windows-gnu

echo "[build] done:"
ls -lh src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/*.exe
ls -lh src-tauri/target/x86_64-pc-windows-gnu/release/espectral-client.exe

# The updater manifest is not built here: it must point at the immutable
# release URL and carry the signature of the artifact that actually ships.
# A local build only needs the signed installer, which the bundler just wrote
# next to the exe.
