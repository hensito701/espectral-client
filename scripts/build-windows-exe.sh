#!/usr/bin/env bash
# ==========================================================================
# build-windows-exe.sh — cross-compile the Espectral Client Windows binary and
# bundle the NSIS installer (from WSL2 or any Linux with the same toolchain).
#
# Prerequisites (one-time, no root needed unless noted):
#   1. Rust via rustup, on PATH, with the Windows target:
#        rustup target add x86_64-pc-windows-gnu
#   2. mingw-w64 (x86_64): x86_64-w64-mingw32-gcc and -windres, plus ar/dlltool.
#      Either system packages (gcc-mingw-w64-x86-64, binutils-mingw-w64-x86-64)
#      or debs extracted into ~/mingw. windres is also what build.rs uses for
#      the exe's icon/version resource, so a missing one fails the build early.
#   3. NSIS 3.x with its plugins: system `makensis`, or an extracted tree in
#      ~/nsis (then ~/bin/makensis must set NSISDIR to it).
#
# Output:
#   build/windows/EspectralClient_<ver>_x64-setup.exe (+ .sig)
#   ${CARGO_TARGET_DIR:-native/target}/x86_64-pc-windows-gnu/<profile>/espectral-client.exe
#
# Usage:
#   scripts/build-windows-exe.sh                   build, bundle, sign (release)
#   scripts/build-windows-exe.sh --skip-build      reuse the existing release exe
#   scripts/build-windows-exe.sh --unsigned        allow a missing signing key
#   scripts/build-windows-exe.sh --fast --unsigned             zlib-compressed (dev)
#   scripts/build-windows-exe.sh --profile release-fast --unsigned  no-LTO build
#
# Knobs (environment):
#   JOBS=2              stage 2 (app + Slint UI) cargo -j; it is the RAM peak
#   DEP_JOBS=<n>        stage 1 (deps + engine + skin3d) cargo -j, capped at 6
#   CARGO_TARGET_DIR    target dir; relative values resolve against the repo
#                       root. One per worktree, never shared.
#   WIN_LINKER=ld       keep mingw's GNU ld instead of the lld auto-detect
#   RUSTFLAGS or CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS set: the caller
#                       owns the linker flags, auto-detect is skipped
#
# Linker: lld through mingw gcc (faster link, identical code) when a throwaway
# C link probe succeeds, else mingw ld. The flag goes into the target-specific
# rustflags so host build scripts and proc-macros keep the default linker.
#
# --fast and --profile release-fast are dev-only installers: they are NEVER
# signed and both refuse to run without --unsigned, so a release artifact can
# never come from them. The default run stays release + /SOLID lzma + signing.
#
# The signing key comes from $TAURI_SIGNING_PRIVATE_KEY or .keys/espectral.key;
# the .sig is in the same format `tauri signer sign` writes (see
# scripts/sign-update.sh).
# ==========================================================================
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

SKIP_BUILD=0
ALLOW_UNSIGNED=0
FAST_COMPRESS=0
PROFILE=release
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1 ;;
    --unsigned)   ALLOW_UNSIGNED=1 ;;
    --fast)       FAST_COMPRESS=1 ;;
    --profile)
      [ $# -ge 2 ] || { echo "ERROR: --profile needs a value (release|release-fast)" >&2; exit 1; }
      PROFILE="$2"
      shift
      ;;
    -h|--help)    sed -n '2,47p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; exit 1 ;;
  esac
  shift
done
case "$PROFILE" in
  release|release-fast) ;;
  *) echo "ERROR: --profile must be release or release-fast (got '$PROFILE')" >&2; exit 1 ;;
esac

# A dev-optimized installer must never be publishable: no signing key is used
# and the build itself is an explicit decision (--unsigned).
if [ "$FAST_COMPRESS" = "1" ] || [ "$PROFILE" != "release" ]; then
  if [ "$ALLOW_UNSIGNED" != "1" ]; then
    echo "ERROR: --fast / --profile release-fast builds are dev-only and are NEVER signed;" >&2
    echo "       pass --unsigned to build one (release installers use the default release profile)." >&2
    exit 1
  fi
  SIGNABLE=0
else
  SIGNABLE=1
fi

JOBS="${JOBS:-2}"
UI_JOBS="$JOBS"
TARGET=x86_64-pc-windows-gnu
PKG=espectral-client
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' native/Cargo.toml | head -1)"
[ -n "$VERSION" ] || { echo "ERROR: could not read the version from native/Cargo.toml" >&2; exit 1; }

if [ "$PROFILE" = "release" ]; then
  PROFILE_FLAGS=(--release)
else
  PROFILE_FLAGS=(--profile "$PROFILE")
fi

# Respect an explicit CARGO_TARGET_DIR everywhere; a relative
# value is resolved against the repo root so the binary path and cargo agree
# even though cargo runs from native/.
case "${CARGO_TARGET_DIR:-}" in
  "") TARGET_DIR="$ROOT/native/target" ;;
  /*) TARGET_DIR="$CARGO_TARGET_DIR" ;;
  *)  TARGET_DIR="$ROOT/$CARGO_TARGET_DIR" ;;
esac

# Toolchain hints (WSL layouts; harmless when the tools are on PATH already).
export PATH="$HOME/.cargo/bin:$HOME/mingw/usr/bin:$HOME/bin:$HOME/nsis/usr/bin:$PATH"
if [ -z "${NSISDIR:-}" ]; then
  if [ -d "$HOME/nsis/usr/share/nsis" ]; then NSISDIR="$HOME/nsis/usr/share/nsis"; else NSISDIR=/usr/share/nsis; fi
fi
export NSISDIR
# build.rs picks its own windres, but cargo's linker for the gnu target must be
# the cross compiler (not `cc`).
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
export CARGO_INCREMENTAL=0

need() { command -v "$1" >/dev/null 2>&1 || { echo "ERROR: $1 not found ($2)" >&2; exit 1; }; }
need rustc   "install the Rust toolchain (rustup)"
need "$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER" "install mingw-w64 x86_64 (gcc-mingw-w64-x86-64) or look in ~/mingw/usr/bin"
need makensis "install NSIS 3.x (apt/fedora nsis) or extract it into ~/nsis"
command -v x86_64-w64-mingw32-windres >/dev/null 2>&1 \
  || echo "[build] WARNING: x86_64-w64-mingw32-windres not on PATH; build.rs needs it (or \$WINDRES) for the icon/version resource" >&2

# ---- linker -------------------------------------------------------------
# GNU ld links the exe single-threaded; lld through mingw's gcc is much faster.
# It is used only when a throwaway C program really links (missing ld.lld, a
# mingw CRT quirk, ... fall back to ld), and WIN_LINKER=ld disables it.
if [ -n "${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS:-}" ] || [ -n "${RUSTFLAGS:-}" ]; then
  echo "[build] linker: caller-provided flags (RUSTFLAGS / CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS); auto-detect skipped"
elif [ "${WIN_LINKER:-}" = "ld" ]; then
  echo "[build] linker: mingw ld (WIN_LINKER=ld)"
else
  probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/espectral-lld-probe.XXXXXX")"
  printf 'int main(void) { return 0; }\n' > "$probe_dir/probe.c"
  if "$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER" -fuse-ld=lld "$probe_dir/probe.c" \
       -o "$probe_dir/probe.exe" >/dev/null 2>&1 && [ -s "$probe_dir/probe.exe" ]; then
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=lld"
    echo "[build] linker: lld (mingw gcc -fuse-ld=lld; WIN_LINKER=ld forces mingw ld)"
  else
    echo "[build] linker: mingw ld (lld probe failed; WIN_LINKER=ld forces mingw ld)"
  fi
  rm -rf "$probe_dir"
fi

# ---- branding + icon inputs ---------------------------------------------
BRANDING_SRC="$ROOT/assets/branding"
[ -f "$BRANDING_SRC/branding.json" ] || {
  echo "ERROR: $BRANDING_SRC/branding.json missing — run 'npm run deploy:mod'" >&2
  echo "       (or scripts/update-branding-manifest.mjs) after building the mod jars" >&2
  exit 1
}
ICON="$ROOT/native/app/icons/icon.ico"
[ -f "$ICON" ] || { echo "ERROR: $ICON missing — run 'node scripts/make-icons.mjs'" >&2; exit 1; }

# ---- build --------------------------------------------------------------
BIN="$TARGET_DIR/$TARGET/$PROFILE/$PKG.exe"
if [ "$SKIP_BUILD" = "1" ]; then
  [ -f "$BIN" ] || { echo "ERROR: $BIN missing and --skip-build was given" >&2; exit 1; }
  echo "[build] --skip-build: using $BIN"
else
  # Stage 1 has a low RAM peak per job, so it gets the full CPU parallelism;
  # stage 2 (the Slint UI crate) is bounded by JOBS instead.
  DEP_JOBS="${DEP_JOBS:-$(nproc 2>/dev/null || echo 4)}"
  if [ "$DEP_JOBS" -gt 6 ]; then DEP_JOBS=6; fi
  echo "[build] stage 1/2: espectral-engine + espectral-skin3d (profile $PROFILE, -j $DEP_JOBS)"
  ( cd native && cargo build "${PROFILE_FLAGS[@]}" -p espectral-engine -p espectral-skin3d \
      --target "$TARGET" --target-dir "$TARGET_DIR" -j "$DEP_JOBS" )
  echo "[build] stage 2/2: $PKG (profile $PROFILE, -j $UI_JOBS)"
  ( cd native && cargo build "${PROFILE_FLAGS[@]}" -p "$PKG" \
      --target "$TARGET" --target-dir "$TARGET_DIR" -j "$UI_JOBS" )
  [ -f "$BIN" ] || { echo "ERROR: build produced no $BIN" >&2; exit 1; }
fi

# ---- stage branding (makensis wants a plain dir; assets/branding is one) --
STAGE="$ROOT/build/windows-stage"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -a "$BRANDING_SRC" "$STAGE/branding"
find "$STAGE/branding" -name '.DS_Store' -delete 2>/dev/null || true

OUT_DIR="$ROOT/build/windows"
OUT="$OUT_DIR/EspectralClient_${VERSION}_x64-setup.exe"
mkdir -p "$OUT_DIR"
rm -f "$OUT" "$OUT.sig"

# ---- bundle -------------------------------------------------------------
MAKENSIS_FLAGS=()
if [ "$FAST_COMPRESS" = "1" ]; then MAKENSIS_FLAGS+=(-DFAST_COMPRESS); fi
echo "[build] makensis -> $OUT"
# -INPUTCHARSET UTF8: the .nsi/.nsh files are UTF-8 (Spanish accents, ©) —
# without it makensis reads them as the build host's ANSI codepage and the
# translated strings arrive mangled on Windows.
makensis -INPUTCHARSET UTF8 -V2 \
  "${MAKENSIS_FLAGS[@]}" \
  -DVERSION="$VERSION" \
  -DSRC_EXE="$BIN" \
  -DSRC_BRANDING="$STAGE/branding" \
  -DICON="$ICON" \
  -DOUTFILE="$OUT" \
  "$ROOT/native/packaging/windows/installer.nsi"

[ -f "$OUT" ] || { echo "ERROR: makensis produced no $OUT" >&2; exit 1; }
echo "[build] installer: $OUT ($(du -h "$OUT" | cut -f1))"
echo "[build] portable exe: $BIN ($(du -h "$BIN" | cut -f1))"

# ---- sign ---------------------------------------------------------------
if [ "$SIGNABLE" = "1" ]; then
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || [ -f "$ROOT/.keys/espectral.key" ]; then
    "$ROOT/scripts/sign-update.sh" "$OUT"
  elif [ "$ALLOW_UNSIGNED" = "1" ]; then
    echo "[build] WARNING: --unsigned and no key: $OUT.sig was NOT written. The in-app updater will refuse this artifact." >&2
  else
    echo "ERROR: no signing key. Set TAURI_SIGNING_PRIVATE_KEY (base64 minisign secret key) or add .keys/espectral.key," >&2
    echo "       or pass --unsigned for a local-only build." >&2
    exit 1
  fi
else
  echo "[build] dev build (--fast / release-fast): never signed; $OUT.sig NOT written."
fi

# ---- summary ------------------------------------------------------------
echo
echo "[build] sha256:"
sha256sum "$OUT"
[ -f "$OUT.sig" ] && sha256sum "$OUT.sig"
echo "[build] done."
