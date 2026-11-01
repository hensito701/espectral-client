#!/usr/bin/env bash
# Render one route of the native UI headless (software renderer, no display
# server needed) and write it as PNG.
#
#   native/app/scripts/screenshot.sh '#/settings' /tmp/settings.png
#   native/app/scripts/screenshot.sh '#/' /tmp/home-light.png --theme light --lang en
#   native/app/scripts/screenshot.sh '#/' /tmp/palette.png --open palette --query set
#   native/app/scripts/screenshot.sh '#/instances/Espectral%20default?tab=mods' /tmp/i.png --wait 3000
#   native/app/scripts/screenshot.sh '#/settings' /tmp/s2.png --scroll 600
#   native/app/scripts/screenshot.sh '#/' /tmp/w.png --open wizard --click-text 'Siguiente'
#
# Options (after the two positional args): --theme dark|light|system, --lang es|en,
# --accent oro|carmesi|amatista|esmeralda|glaciar, --size 1280x800, --wait <ms> (default 1500),
# --scroll <px> (PageScaffold pages), --open palette|wizard|account|pip|gate (repeatable),
# --query <text> (palette). Input, in order, after half of --wait: --click X,Y,
# --click-text <label> (the visible element showing that text; exit 3 when none),
# --type <text>, --key Return|Escape|Tab|…, --nav <route> (all repeatable).
#
# Data: a persistent fixture data dir ($ESPECTRAL_SHOT_DATA, default
# native/target/shot-data) so onboarding ("Espectral default") runs once and
# your real launcher data is never touched. The gate is skipped unless --open gate.
# For promo/README shots, `node scripts/showcase-data.mjs <dir>` builds a clean
# one (readable instance names, one offline account) to point $ESPECTRAL_SHOT_DATA at.
# Screenshot mode starts no listener, so it runs next to a launcher or engine.
# Reference screenshots of the Svelte UI: see "Reference screenshots" in UI-PORTING.md.
# Note: the software renderer draws gradient text with its first color stop
# (femtovg, the default on-screen renderer, draws the real gradient).
set -euo pipefail
app_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
native_dir=$(cd "$app_dir/.." && pwd)
if [[ $# -lt 2 ]]; then
  sed -n '2,28p' "${BASH_SOURCE[0]}"
  exit 2
fi
# Live-preview build (own target dir, never shipped): it loads ui/*.slint when
# it starts, so after the first build a .slint edit costs ~10 s of cargo (the
# small live UI crate) instead of recompiling the ~60 MB generated module.
# Rust changes (new properties/callbacks) rebuild as usual. Set
# ESPECTRAL_SHOT_COMPILED=1 to shoot the normal compiled build instead.
if [[ -n "${ESPECTRAL_SHOT_COMPILED:-}" ]]; then
  cargo build -q -p espectral-client --manifest-path "$native_dir/Cargo.toml"
  bin="$native_dir/target/debug/espectral-client"
else
  SLINT_LIVE_PREVIEW=1 cargo build -q -p espectral-client --features live-preview \
    --manifest-path "$native_dir/Cargo.toml" --target-dir "$native_dir/target/live"
  bin="$native_dir/target/live/debug/espectral-client"
fi
data="${ESPECTRAL_SHOT_DATA:-$native_dir/target/shot-data}"
mkdir -p "$data"
ESPECTRAL_DATA_DIR="$data" ESPECTRAL_LOG="${ESPECTRAL_LOG:-warn}" \
  "$bin" --screenshot "$@"
