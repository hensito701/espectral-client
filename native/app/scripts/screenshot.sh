#!/usr/bin/env bash
# Render one route of the native UI headless (software renderer, no display
# server needed) and write it as PNG.
#
#   native/app/scripts/screenshot.sh '#/settings' /tmp/settings.png
#   native/app/scripts/screenshot.sh '#/' /tmp/home-light.png --theme light --lang en
#   native/app/scripts/screenshot.sh '#/' /tmp/palette.png --open palette --query set
#   native/app/scripts/screenshot.sh '#/instances/Espectral%20default?tab=mods' /tmp/i.png --wait 3000
#   native/app/scripts/screenshot.sh '#/settings' /tmp/s2.png --scroll 600
#
# Options (after the two positional args): --theme dark|light|system, --lang es|en,
# --accent oro|carmesi|amatista|esmeralda|glaciar, --size 1280x800, --wait <ms> (default 1500),
# --scroll <px> (PageScaffold pages), --open palette|wizard|account|pip|gate (repeatable),
# --query <text> (palette).
#
# Data: a persistent fixture data dir ($ESPECTRAL_SHOT_DATA, default
# native/target/shot-data) so onboarding ("Espectral default") runs once and
# your real launcher data is never touched. The gate is skipped unless --open gate.
# Reference screenshots of the Svelte UI: see "Reference screenshots" in UI-PORTING.md.
# Note: the software renderer draws gradient text with its first color stop
# (femtovg, the default on-screen renderer, draws the real gradient).
set -euo pipefail
app_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
native_dir=$(cd "$app_dir/.." && pwd)
if [[ $# -lt 2 ]]; then
  sed -n '2,22p' "${BASH_SOURCE[0]}"
  exit 2
fi
cargo build -q -p espectral-client --manifest-path "$native_dir/Cargo.toml"
data="${ESPECTRAL_SHOT_DATA:-$native_dir/target/shot-data}"
mkdir -p "$data"
ESPECTRAL_DATA_DIR="$data" ESPECTRAL_LOG="${ESPECTRAL_LOG:-warn}" \
  "$native_dir/target/debug/espectral-client" --screenshot "$@"
