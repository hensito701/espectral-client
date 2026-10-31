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
#   native/target/x86_64-pc-windows-gnu/release/espectral-client.exe (portable)
#
# Usage:
#   scripts/build-windows-exe.sh                build, bundle, sign
#   scripts/build-windows-exe.sh --skip-build   reuse the existing release exe
#   scripts/build-windows-exe.sh --unsigned     allow a missing signing key
#   JOBS=2 scripts/build-windows-exe.sh
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
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --unsigned)   ALLOW_UNSIGNED=1 ;;
    -h|--help)    sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown argument: $arg" >&2; exit 1 ;;
  esac
done

JOBS="${JOBS:-2}"
TARGET=x86_64-pc-windows-gnu
PKG=espectral-client
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' native/Cargo.toml | head -1)"
[ -n "$VERSION" ] || { echo "ERROR: could not read the version from native/Cargo.toml" >&2; exit 1; }

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
BIN="$ROOT/native/target/$TARGET/release/$PKG.exe"
if [ "$SKIP_BUILD" = "1" ]; then
  [ -f "$BIN" ] || { echo "ERROR: $BIN missing and --skip-build was given" >&2; exit 1; }
  echo "[build] --skip-build: using $BIN"
else
  echo "[build] cargo build --release -p $PKG --target $TARGET -j $JOBS"
  ( cd native && cargo build --release -p "$PKG" --target "$TARGET" -j "$JOBS" )
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
echo "[build] makensis -> $OUT"
# -INPUTCHARSET UTF8: the .nsi/.nsh files are UTF-8 (Spanish accents, ©) —
# without it makensis reads them as the build host's ANSI codepage and the
# translated strings arrive mangled on Windows.
makensis -INPUTCHARSET UTF8 -V2 \
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
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || [ -f "$ROOT/.keys/espectral.key" ]; then
  "$ROOT/scripts/sign-update.sh" "$OUT"
elif [ "$ALLOW_UNSIGNED" = "1" ]; then
  echo "[build] WARNING: --unsigned and no key: $OUT.sig was NOT written. The in-app updater will refuse this artifact." >&2
else
  echo "ERROR: no signing key. Set TAURI_SIGNING_PRIVATE_KEY (base64 minisign secret key) or add .keys/espectral.key," >&2
  echo "       or pass --unsigned for a local-only build." >&2
  exit 1
fi

# ---- summary ------------------------------------------------------------
echo
echo "[build] sha256:"
sha256sum "$OUT"
[ -f "$OUT.sig" ] && sha256sum "$OUT.sig"
echo "[build] done."
