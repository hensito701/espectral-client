#!/usr/bin/env node
// One-off exception import: add a Lunar Client-sourced Microsoft account to
// Espectral Client.
//
// The standard settings importer (import.mjs) deliberately does NOT import
// accounts; this script is the documented exception path for that one case.
//
// Lunar stores a durable MSA OAuth refresh token per account in
// `<gameDir>/.lunarclient/settings/game/accounts.json` (field `refreshToken`).
// That token is bound to the legacy launcher client id
// (00000000402b5328) + login.live.com endpoint, so the engine stores it with
// `microsoft.lunar: true` and refreshes via that exact pair at launch.
//
// Usage:
//   node scripts/import-lunar-account.mjs [--accounts <lunar accounts.json>]
//
// The script reads Lunar's accounts.json, picks the account (by --username
// if given, else the active one), and upserts it into Espectral's config
// (ESPECTRAL_DATA_DIR override supported).
//
// Secrets: never prints tokens. Output is identity + token length only.
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

import { upsertLunarAccount } from '../src/engine/accounts.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '..');

function arg(name, fallback) {
  const i = process.argv.indexOf(name);
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const CANDIDATE_PATHS = [
  process.env.LUNAR_ACCOUNTS ?? '',
  join(process.env.APPDATA ?? '', '.minecraft', '.lunarclient', 'settings', 'game', 'accounts.json'),
].filter(Boolean);
const accountsFile = arg('--accounts', null) ?? CANDIDATE_PATHS.find((p) => existsSync(p));
if (!accountsFile) {
  console.error('Lunar accounts.json not found — pass --accounts <path>');
  process.exit(1);
}
const wantedUsername = arg('--username', null);
const data = JSON.parse(readFileSync(accountsFile, 'utf8'));
const entries = Object.values(data.accounts ?? {});
const chosen = wantedUsername
  ? entries.find((a) => a.username === wantedUsername || a.minecraftProfile?.name === wantedUsername)
  : entries.find((a) => a.localId === data.activeAccountLocalId) ?? entries[0];
if (!chosen) {
  console.error(`no account found in ${accountsFile}${wantedUsername ? ` for '${wantedUsername}'` : ''}`);
  process.exit(1);
}
if (!chosen.refreshToken) {
  console.error(`account '${chosen.username}' has no refreshToken (Lunar must have logged it in first)`);
  process.exit(1);
}

const name = chosen.minecraftProfile?.name ?? chosen.username;
const uuid = (chosen.minecraftProfile?.id ?? '').replace(/(\w{8})(\w{4})(\w{4})(\w{4})(\w{12})/, '$1-$2-$3-$4-$5');
const account = upsertLunarAccount({ username: name, uuid, refreshToken: chosen.refreshToken });
console.log('---');
console.log(`imported account: ${account.username} (${account.uuid})`);
console.log(`token_kind: ${account.token_kind} (msa; Lunar refresh token, len=${chosen.refreshToken.length})`);
console.log('launch will refresh a fresh Minecraft session via the Lunar-bound token automatically.');
