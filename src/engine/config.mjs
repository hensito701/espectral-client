/**
 * AppConfig persistence.
 *
 * Single JSON document at <data>/config.json holding the AppConfig subset plus
 * engine-owned state (theme, accounts, active account, cached JVM info).
 * Accounts live here per the B1 contract ("store accounts in data/config.json").
 *
 * Data dir: `ESPECTRAL_DATA_DIR` env override, else `<repo root>/data`.
 * Writes are atomic (.tmp + rename) so a crash never truncates the file.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(__dirname, '..', '..');

/** Resolve the runtime data directory (env override wins). */
export function dataDir() {
  return process.env.ESPECTRAL_DATA_DIR
    ? path.resolve(process.env.ESPECTRAL_DATA_DIR)
    : path.join(REPO_ROOT, 'data');
}

/** Absolute path of the config document. */
export function configPath() {
  return path.join(dataDir(), 'config.json');
}

export const DEFAULT_CONFIG = {
  // AppConfig subset (returned by GET /api/config)
  default_memory_mb: 3072,
  download_concurrency: 6,
  fast_boot: false,
  aot_auto_train: true,
  fullbright_on_launch: false,
  jdk_path_override: null,
  discord_enabled: true,
  // espectral-id service base URL. Canonical host since the espectral.es zone
  // lapsed (2026-10-22); LEGACY_IDENTITY_URLS below are migrated on load.
  identity_url: 'https://id.uhce.top',
  // engine-owned state
  theme: 'dark',
  // Client accent palette id (Workstream 4). Drives <html data-accent> in the
  // launcher UI and is mirrored into personalization.theme at launch. Same id
  // set as the mod's palettes.
  accent: 'oro',
  // Global reduced-motion choice; mirrored into personalization.reduced_motion
  // at launch (per-instance configs also PATCH it directly for live reload).
  reduced_motion: false,
  active_username: null,
  accounts: [],
  jvm: null,
  first_launch_setup_complete: false,
  first_launch_import_source: null,
};

let cached = null;

/**
 * Identity-service hosts that predate the `uhce.top` move (the espectral.es
 * zone lapsed ~2026-10-22). A persisted config.json carrying one of these is
 * migrated to `DEFAULT_CONFIG.identity_url` on load, so existing installs keep
 * working without hand-editing JSON. Trailing slashes are tolerated.
 */
export const LEGACY_IDENTITY_URLS = ['https://id.espectral.es'];

/** True when `url` is exactly a bare legacy identity host. */
export function isLegacyIdentityUrl(url) {
  if (typeof url !== 'string') return false;
  return LEGACY_IDENTITY_URLS.includes(url.trim().replace(/\/+$/, '').toLowerCase());
}

/** Load (and lazily create) the config document. Returns the live object. */
export function loadConfig() {
  if (cached) return cached;
  let raw = {};
  try {
    raw = JSON.parse(fs.readFileSync(configPath(), 'utf8'));
  } catch {
    raw = {}; // missing or corrupt -> start from defaults
  }
  cached = { ...DEFAULT_CONFIG, ...raw };
  if (!Array.isArray(cached.accounts)) cached.accounts = [];
  // Only a *legacy* value is rewritten — any other custom identity_url is the
  // user's own (self-hosted service) and is left alone. Persist immediately so
  // the file stops naming a dead host even if nothing else is ever saved.
  const migratedIdentity = isLegacyIdentityUrl(cached.identity_url);
  if (migratedIdentity) cached.identity_url = DEFAULT_CONFIG.identity_url;
  if (migratedIdentity || !fs.existsSync(configPath())) persist(cached);
  return cached;
}

/** Merge `patch` into the live config and persist. Returns the live object. */
export function saveConfig(patch = {}) {
  const current = loadConfig();
  for (const [k, v] of Object.entries(patch)) {
    if (v === undefined) continue;
    current[k] = v;
  }
  persist(current);
  return current;
}

function persist(config) {
  try {
    fs.mkdirSync(path.dirname(configPath()), { recursive: true });
    const tmp = configPath() + '.tmp';
    // H2: never serialize the in-memory MC access token (or the transient
    // XBL/XSTS chain material) to disk. ensureMinecraftToken keeps the fresh
    // token on the live config object for same-session reuse, but it must not
    // survive a saveConfig. The durable secret is the refresh token only.
    // NOTE: the replacer MUST be the 2nd arg — JSON.stringify(value, replacer,
    // space) ignores anything past the 3rd, so the previous `null, 2, replacer`
    // silently serialized access_token to disk.
    const json = JSON.stringify(
      config,
      (key, value) =>
        key === 'access_token' || key === 'xbl_token' || key === 'xsts_token' || key === 'user_hash'
          ? undefined
          : value,
      2
    );
    fs.writeFileSync(tmp, json, 'utf8');
    fs.renameSync(tmp, configPath());
  } catch (e) {
    console.warn('[config] could not persist ' + configPath() + ':', e.message);
  }
}

/** The AppConfig shape exposed by GET /api/config (no engine-internal state). */
export function appConfig() {
  const c = loadConfig();
  return {
    default_memory_mb: c.default_memory_mb,
    accent: typeof c.accent === 'string' ? c.accent : 'oro',
    reduced_motion: c.reduced_motion === true,
    download_concurrency: c.download_concurrency,
    aot_auto_train: c.aot_auto_train,
    fast_boot: c.fast_boot === true,
    fullbright_on_launch: c.fullbright_on_launch === true,
    jdk_path_override: c.jdk_path_override ?? null,
    discord_enabled: c.discord_enabled !== false,
    data_dir: dataDir(),
  };
}
