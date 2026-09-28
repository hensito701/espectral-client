#!/usr/bin/env bash
# Idempotent deploy of espectral-id to OVM3.
#
#   bash services/espectral-id/deploy/deploy.sh
#
# Prereqs on this workstation:
#   - ssh alias `ovm3` (ops workspace), sudo -n on the remote side
#   - secrets env file, e.g.
#       ESPECTRAL_ID_ENV=E:/NPv2/espectral-backup-2026-08-05/local-workspace/connections/.env
#
# What it does (all idempotent):
#   1. Syncs app/, requirements.txt into /home/ubuntu/espectral-id/
#   2. Refreshes the bundled catalog from src/engine/cosmetics-catalog.json
#   3. Builds/updates the venv
#   4. Writes .env from the secrets file ONLY if absent or forced
#      (VALUES=refresh to overwrite), mode 0600, never printed
#   5. Installs/enables the systemd unit, restarts the service
#   6. Installs the nginx vhost, nginx -t, reloads nginx
# DNS + TLS + firewall are documented in the README; this script does not
# touch Cloudflare, certbot, or iptables.
set -euo pipefail

HOST=${ESPECTRAL_ID_HOST:-ovm3}
REMOTE=/home/ubuntu/espectral-id
ENV_FILE=${ESPECTRAL_ID_ENV:-}
VALUES=${VALUES:-keep}   # keep | refresh

ROOT="$(cd "$(dirname "$0")/.." && pwd)"          # services/espectral-id
REPO="$(cd "$ROOT/../.." && pwd)"

# Refresh bundled catalog from the repo source of truth.
cp "$REPO/src/engine/cosmetics-catalog.json" "$ROOT/app/cosmetics-catalog.json"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/app"
cp -r "$ROOT/app/." "$tmp/app/"
cp "$ROOT/requirements.txt" "$tmp/"
cp "$ROOT/deploy/espectral-id.service" "$tmp/"
cp "$ROOT/deploy/nginx-id.espectral.es.conf" "$tmp/"

echo "== syncing $tmp -> $HOST:$REMOTE"
ssh "$HOST" "mkdir -p '$REMOTE' '$REMOTE/data'"
tar -C "$tmp" -cf - app requirements.txt | ssh "$HOST" "tar -C '$REMOTE' -xf -"

if [[ -n "$ENV_FILE" ]]; then
  # Build remote .env over ssh stdin without echoing secrets.
  python - "$ENV_FILE" <<'PY' | ssh "$HOST" "umask 077; cat > '$REMOTE/.env.new' && chmod 600 '$REMOTE/.env.new'"
import re, sys
src = {}
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    line = line.strip()
    if line and "=" in line and not line.startswith("#"):
        k, _, v = line.partition("=")
        src[k.strip()] = v.strip().strip('"').strip("'")
keys = ["DISCORD_TOKEN", "DISCORD_CLIENT_ID", "DISCORD_CLIENT_SECRET",
        "ESPECTRAL_ID_SIGNING_KEY"]
missing = [k for k in keys if not src.get(k)]
if missing:
    sys.stderr.write("missing keys in env file: " + ",".join(missing) + "\n")
    sys.exit(1)
out = {**src}
print("\n".join(f"{k}={out[k]}" for k in keys))
print("ESPECTRAL_ID_DB=/home/ubuntu/espectral-id/data/espectral-id.db")
print("ESPECTRAL_ID_HOST=127.0.0.1")
print("ESPECTRAL_ID_PORT=8091")
PY
  if [[ "$VALUES" == "refresh" ]]; then
    ssh "$HOST" "mv '$REMOTE/.env.new' '$REMOTE/.env'"
    echo "== .env refreshed"
  else
    ssh "$HOST" "test -f '$REMOTE/.env' || mv '$REMOTE/.env.new' '$REMOTE/.env'; rm -f '$REMOTE/.env.new'"
    echo "== .env kept existing (VALUES=refresh to overwrite)"
  fi
else
  echo "== ESPECTRAL_ID_ENV not set; leaving existing .env untouched"
fi

echo "== venv + deps"
ssh "$HOST" "set -e
  test -x '$REMOTE/venv/bin/python' || python3.12 -m venv '$REMOTE/venv'
  '$REMOTE/venv/bin/pip' -q install --upgrade pip
  '$REMOTE/venv/bin/pip' -q install -r '$REMOTE/requirements.txt'"

echo "== systemd unit + nginx vhost"
ssh "$HOST" "set -e
  sudo -n install -m 644 -o root -g root /dev/stdin /etc/systemd/system/espectral-id.service" \
    < "$ROOT/deploy/espectral-id.service"
ssh "$HOST" "set -e
  sudo -n install -m 644 -o root -g root /dev/stdin /etc/nginx/sites-available/id.espectral.es" \
    < "$ROOT/deploy/nginx-id.espectral.es.conf"
ssh "$HOST" "set -e
  sudo -n ln -sfn /etc/nginx/sites-available/id.espectral.es /etc/nginx/sites-enabled/id.espectral.es
  sudo -n systemctl daemon-reload
  sudo -n systemctl enable espectral-id
  sudo -n systemctl restart espectral-id"

echo "== nginx -t + reload"
ssh "$HOST" "set -e
  if sudo -n nginx -t; then sudo -n systemctl reload nginx; else echo 'nginx -t FAILED; not reloading' >&2; exit 1; fi"

echo "== done. health check: ssh $HOST curl -s http://127.0.0.1:8091/v1/healthz"
