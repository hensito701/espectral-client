#!/usr/bin/env bash
# ==========================================================================
# sign-update.sh — sign a release artifact in the format the updater expects.
#
# The signature is byte-for-byte what `tauri signer sign <file>` produces, so
# both the last Tauri clients and the Rust client verify the same file with
# the same public key:
#
#   <file>.sig = base64( minisign signature box text )
#
# and the box text is the four minisign lines
#
#   untrusted comment: signature from tauri secret key
#   <base64 of alg||keyid||signature>
#   trusted comment: timestamp:<unix>\tfile:<basename>      (real TAB)
#   <base64 of the global signature>
#
# It is prehashed (minisign's default; tauri's `sign(None, …)` too), and it is
# base64 of the *whole box*, not the raw signature — that is what
# `minisign_verify::Signature::decode` (used by native/engine/src/updater.rs)
# expects after decoding.
#
# Key:
#   $TAURI_SIGNING_PRIVATE_KEY           base64 of a minisign secret key file
#                                        ("untrusted comment: minisign
#                                        encrypted secret key" + base64 line)
#   $TAURI_SIGNING_PRIVATE_KEY_PASSWORD  its password ("" for a passwordless
#                                        key, which is what .keys/espectral.key
#                                        is). Falls back to an empty password.
#   .keys/espectral.key                  same content, used when the env var
#                                        is unset.
#
# Usage: scripts/sign-update.sh <file> [<file>...]
#        SCRIPTS_ALLOW_UNSIGNED=1 scripts/sign-update.sh <file>   # warn, no .sig
# ==========================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY_FILE="$ROOT/.keys/espectral.key"

die() { echo "ERROR: $*" >&2; exit 1; }
warn() { echo "WARNING: $*" >&2; }

[ $# -ge 1 ] || die "usage: $0 <file> [<file>...]"

for f in "$@"; do
  [ -f "$f" ] || die "not a file: $f"
done

# ---- key material -------------------------------------------------------
KEY_B64="${TAURI_SIGNING_PRIVATE_KEY:-}"
if [ -z "$KEY_B64" ] && [ -f "$KEY_FILE" ]; then
  KEY_B64="$(tr -d '\r\n' < "$KEY_FILE")"
  echo "[sign] using $KEY_FILE"
fi
if [ -z "$KEY_B64" ]; then
  if [ "${SCRIPTS_ALLOW_UNSIGNED:-}" = "1" ]; then
    warn "no signing key (TAURI_SIGNING_PRIVATE_KEY unset and $KEY_FILE missing) — no .sig written"
    exit 0
  fi
  die "no signing key: set TAURI_SIGNING_PRIVATE_KEY (base64 of the minisign secret key) or put it in $KEY_FILE (set SCRIPTS_ALLOW_UNSIGNED=1 to skip signing)"
fi
command -v minisign >/dev/null 2>&1 || die "minisign not found (dnf/apt install minisign)"
command -v base64 >/dev/null 2>&1 || die "base64 not found"

PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

# Decode the key to a private, self-deleting file. Never leave it in /tmp
# readable by others, never echo it.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/espectral-sign.XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT INT TERM

KEYPATH="$WORK/secret.key"
if ! printf '%s' "$KEY_B64" | base64 -d > "$KEYPATH" 2>/dev/null; then
  die "TAURI_SIGNING_PRIVATE_KEY is not valid base64"
fi
chmod 600 "$KEYPATH"
[ -s "$KEYPATH" ] || die "decoded signing key is empty"
head -c 40 "$KEYPATH" | grep -q "minisign" \
  || die "decoded key is not a minisign secret key (want 'untrusted comment: minisign encrypted secret key')"

# ---- sign ---------------------------------------------------------------
status=0
for f in "$@"; do
  base="$(basename "$f")"
  box="$WORK/$base.minisig"
  ts="$(date +%s)"
  trusted_comment="timestamp:$ts	file:$base"     # the tab is part of the format
  rm -f "$box"

  # A passwordless key still asks on stdin; an empty line satisfies it and is
  # harmless for an encrypted-with-empty-password key.
  if ! printf '%s\n' "$PASSWORD" | minisign -S -s "$KEYPATH" -m "$f" \
        -x "$box" -c "signature from tauri secret key" -t "$trusted_comment" >"$WORK/minisign.log" 2>&1; then
    echo "ERROR: minisign failed for $f:" >&2
    sed 's/^/  /' "$WORK/minisign.log" >&2
    status=1
    continue
  fi
  [ -s "$box" ] || { echo "ERROR: minisign produced no signature for $f" >&2; status=1; continue; }

  base64 -w0 "$box" > "$f.sig"
  echo "[sign] $f.sig ← $base ($(stat -c%s "$f.sig") bytes)"
done
exit $status
