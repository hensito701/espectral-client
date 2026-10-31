#!/usr/bin/env bash
# ==========================================================================
# build-linux-deb.sh — build the Espectral Client .deb for the Rust launcher.
#
# The package keeps the exact identity of the Tauri 1.6.x .deb so
# `apt install ./espectral-client_x.y.z_amd64.deb` (and the in-app updater's
# `pkexec dpkg -i`) upgrades it in place:
#
#   Package:      espectral-client          Version: <workspace version>
#   Architecture: amd64                     Maintainer: EspectralClient
#   /usr/bin/espectral-client
#   /usr/lib/Espectral Client/branding/     (the mod jars + branding.json)
#   /usr/share/applications/Espectral Client.desktop
#   /usr/share/icons/hicolor/{32x32,256x256}/apps/espectral-client.png
#
# Old Tauri-only paths (engine/, node-bin/, node_modules/, package.json) are
# simply no longer listed: dpkg removes the files the new package does not own.
#
# Usage:
#   scripts/build-linux-deb.sh                 build + package + sign if key present
#   scripts/build-linux-deb.sh --skip-build    reuse native/target/release/espectral-client
#   BIN=/path/to/espectral-client scripts/build-linux-deb.sh --skip-build   # debug binary smoke test
#   JOBS=2 scripts/build-linux-deb.sh
#
# Output: build/linux/espectral-client_<ver>_amd64.deb (+ .sig)
#
# NOTE on glibc: the release .deb must be built on the OLDEST supported distro
# (Debian 12 / Ubuntu 22.04). A binary built on a newer glibc refuses to start
# on older ones; the script prints the binary's floor so a mismatch is loud.
# ==========================================================================
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

SKIP_BUILD=0
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help) sed -n '2,32p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown argument: $arg" >&2; exit 1 ;;
  esac
done

JOBS="${JOBS:-2}"
PKG=espectral-client
VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' native/Cargo.toml | head -1)"
[ -n "$VERSION" ] || { echo "ERROR: could not read the version from native/Cargo.toml" >&2; exit 1; }

OUT_DIR="$ROOT/build/linux"
BIN="${BIN:-$ROOT/native/target/release/$PKG}"
STAGE="$OUT_DIR/stage"
DEB="$OUT_DIR/${PKG}_${VERSION}_amd64.deb"

need() { command -v "$1" >/dev/null 2>&1 || { echo "ERROR: $1 not found ($2)" >&2; exit 1; }; }
need rustc "install the Rust toolchain: https://rustup.rs"
need dpkg-deb "apt install dpkg-dev"
need strip "apt install binutils"

# ---- build --------------------------------------------------------------
if [ "$SKIP_BUILD" = "1" ]; then
  [ -f "$BIN" ] || { echo "ERROR: $BIN missing and --skip-build was given" >&2; exit 1; }
  echo "[deb] --skip-build: using $BIN"
else
  echo "[deb] cargo build --release -p $PKG -j $JOBS"
  ( cd native && CARGO_INCREMENTAL=0 cargo build --release -p "$PKG" -j "$JOBS" )
  [ -f "$BIN" ] || { echo "ERROR: build produced no $BIN" >&2; exit 1; }
fi

# ---- glibc floor --------------------------------------------------------
# Highest versioned glibc symbol the binary needs. Read with objdump so it
# works on any distro (eu-readelf/`strings` fallbacks), and warn loudly when it
# exceeds what Debian 12 (2.36) / Ubuntu 22.04 (2.35) ship.
glibc_floor() {
  local bin="$1" v=""
  v=$(objdump -T "$bin" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1) || true
  [ -n "$v" ] || v=$(strings -a "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1) || true
  printf '%s' "$v"
}
GLIBC="$(glibc_floor "$BIN")"
echo "[deb] binary needs glibc >= ${GLIBC:-unknown} (Debian 12 ships 2.36, Ubuntu 22.04 ships 2.35)"
if [ -n "$GLIBC" ]; then
  newest_ok=$(printf '%s\n%s\n' "$GLIBC" "2.36" | sort -V | tail -1)
  if [ "$newest_ok" != "2.36" ]; then
    echo "[deb] WARNING: glibc $GLIBC is newer than Debian 12 (2.36) / Ubuntu 22.04 (2.35)." >&2
    echo "[deb] WARNING: release .debs must be built on the OLDEST supported distro or they will not start there." >&2
  fi
fi

# ---- stage the tree -----------------------------------------------------
rm -rf "$STAGE"
install -d "$STAGE/DEBIAN" \
  "$STAGE/usr/bin" \
  "$STAGE/usr/lib/Espectral Client/branding" \
  "$STAGE/usr/share/applications" \
  "$STAGE/usr/share/icons/hicolor/32x32/apps" \
  "$STAGE/usr/share/icons/hicolor/256x256/apps" \
  "$STAGE/usr/share/mime/packages" \
  "$STAGE/usr/share/doc/$PKG"

install -m 0755 "$BIN" "$STAGE/usr/bin/$PKG"

# Branding: the jars + branding.json the engine installs into each instance.
# The manifest pins each jar's sha1, so a missing jar must fail the build
# rather than ship an installer that silently lacks the menu.
BRANDING_SRC="$ROOT/assets/branding"
[ -f "$BRANDING_SRC/branding.json" ] || {
  echo "ERROR: $BRANDING_SRC/branding.json missing — run 'npm run deploy:mod' or scripts/update-branding-manifest.mjs" >&2
  exit 1
}
cp -a "$BRANDING_SRC/." "$STAGE/usr/lib/Espectral Client/branding/"
# Drop editor/OS noise that must never land in a package.
find "$STAGE/usr/lib/Espectral Client/branding" -name '.DS_Store' -delete 2>/dev/null || true

# Icons: same pixel sizes/paths as the Tauri .deb.
install -m 0644 native/app/ui/assets/favicon.png "$STAGE/usr/share/icons/hicolor/32x32/apps/$PKG.png"
install -m 0644 native/app/ui/assets/logo_circle.png "$STAGE/usr/share/icons/hicolor/256x256/apps/$PKG.png"

install -m 0644 "native/packaging/linux/espectral-client.desktop" \
  "$STAGE/usr/share/applications/Espectral Client.desktop"
install -m 0644 native/packaging/linux/espectral-client-mime.xml \
  "$STAGE/usr/share/mime/packages/$PKG.xml"
install -m 0644 native/packaging/linux/copyright "$STAGE/usr/share/doc/$PKG/copyright"

install -m 0755 native/packaging/linux/postinst "$STAGE/DEBIAN/postinst"
install -m 0755 native/packaging/linux/postrm   "$STAGE/DEBIAN/postrm"

# ---- control ------------------------------------------------------------
INSTALLED_KB=$(du -sk --exclude=DEBIAN "$STAGE" | cut -f1)

# Depends: what the Slint/winit + femtovg (OpenGL) binary actually needs.
#   libc6            the ELF interpreter/loader (>= the versioned symbols above)
#   libfontconfig1   system font enumeration (software renderer + fontconfig)
#   libfreetype6     pulled in by fontconfig, but linked directly by the text stack
#   libxkbcommon0    keyboard handling (X11 and Wayland)
#   libgl1 | libegl1 GL/EGL used through dlopen by femtovg/glutin
#   libx11-6, libxcursor1, libxi6, libxrandr2  X11 windowing (winit)
#   libwayland-client0  Wayland windowing (winit)
# libxcb1/libXau6 come with libx11-6; libxkbcommon-x11 is loaded when present
# (Recommends). fonts-dejavu-core guarantees a UI font on a bare system.
DEPENDS="libc6 (>= ${GLIBC:-2.35}), libfontconfig1, libfreetype6, libxkbcommon0, libgl1 | libegl1, libx11-6, libxcursor1, libxi6, libxrandr2, libwayland-client0"
RECOMMENDS="libxkbcommon-x11-0, libglx-mesa0, fonts-dejavu-core, xdg-utils"

cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Architecture: amd64
Installed-Size: $INSTALLED_KB
Maintainer: EspectralClient
Priority: optional
Section: games
Depends: $DEPENDS
Recommends: $RECOMMENDS
Homepage: https://espectral.es
Description: Espectral Client launcher
 Lanzador de Minecraft de Espectral con JDK gestionado y arranque AOT.
 .
 Espectral Client is an open source Minecraft launcher with a managed
 JDK and AOT-accelerated startup, built for the Espectral instances.
 .
 Minecraft is a trademark of Mojang Synergies AB. This launcher is not
 affiliated with or endorsed by Mojang or Microsoft.
EOF

# md5sums for every packaged file (dpkg uses it to verify/upgrade cleanly).
# Paths in md5sums are relative to the package root; hash from there so the
# entries match what dpkg expects (`bin/espectral-client`, not `usr/bin/...`).
( cd "$STAGE" && find usr -type f -print0 | sort -z | sed -z 's|^|./|' | xargs -0 md5sum | sed 's|\./usr/||' > DEBIAN/md5sums )

# ---- package ------------------------------------------------------------
mkdir -p "$OUT_DIR"
rm -f "$DEB"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB" >/dev/null
echo "[deb] $DEB ($(du -h "$DEB" | cut -f1))"

# ---- sign ---------------------------------------------------------------
if [ -x "$ROOT/scripts/sign-update.sh" ] && { [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || [ -f "$ROOT/.keys/espectral.key" ]; }; then
  "$ROOT/scripts/sign-update.sh" "$DEB"
else
  echo "[deb] WARNING: no updater signing key (TAURI_SIGNING_PRIVATE_KEY / .keys/espectral.key) — $DEB.sig NOT written" >&2
fi

# ---- summary ------------------------------------------------------------
echo
echo "[deb] done:"
dpkg-deb -f "$DEB" Package Version Architecture Maintainer Depends
echo "$DEB"
