/**
 * Espectral Client config — the launcher-side half of Contract A.
 *
 * The in-game mod (Agent B) and the launcher share ONE config file per
 * instance: `<instanceDir>/config/espectral-client.json`. The launcher
 * seeds it with defaults at launch, reads it for the Client page, and
 * applies PATCHes from the UI. The mod owns the same file at runtime.
 *
 * Hard rules (Contract A):
 *   - NEVER clobber unknown fields — top-level, per-feature, or inside a
 *     macro. Read-modify-write on the RAW file; GET returns a normalized
 *     view, PATCH merges into the raw object.
 *   - Atomic writes: temp file + rename in the same directory.
 *   - `features[id]` is an OBJECT `{ enabled, ...extra }`, never a bare
 *     boolean. PATCH shallow-merges per feature id.
 *   - All features are OWNED and native: the in-game mod applies them live
 *     (fullbright drives gamma, nofog removes fog, zoom eases FOV, macros
 *     run keybind sequences). No third-party jars, no restart.
 *   - Legacy Gamma Utils / Clear Fog jars, if still installed, are inert
 *     leftovers — reconcile is a no-op returning []. PATCH responses still
 *     carry an additive `errors` array (empty when clean) so the UI can
 *     surface per-feature warnings without reverting the persisted intent.
 */
import fs from 'node:fs';
import path from 'node:path';
import { effectiveGameDir, getInstance } from './instances.mjs';
import { PINS_QOL_BY_VERSION } from './mods.mjs';
import { httpError } from './error.mjs';

/**
 * The canonical feature registry — the SAME list the in-game mod ships.
 * Single source of truth: `src/engine/suite-registry.json` (schema 2),
 * shared verbatim with the mod jar by `scripts/sync-suite-registry.mjs`.
 * GET /api/instances/:name/client returns the derived entries verbatim so
 * the UI renders names/descriptions without hardcoding them. Changing the
 * JSON changes the ids, defaults and categories fed to the API — there is
 * no second feature list in this module.
 */
const SUITE_REGISTRY_DOC = JSON.parse(
  fs.readFileSync(new URL('./suite-registry.json', import.meta.url), 'utf8'),
);

/** Master-switch default for `suite.enabled` (schema 2 config root). */
export const SUITE_DEFAULT_ENABLED = SUITE_REGISTRY_DOC.suite?.default_enabled ?? true;

/** Schema version written by seed/PATCH (v1 files upgrade on seed). */
export const CLIENT_CONFIG_SCHEMA = SUITE_REGISTRY_DOC.schema ?? 2;

export const REGISTRY = SUITE_REGISTRY_DOC.features.map((f) => ({
  id: f.id,
  name: f.name_en,
  description: f.description_en,
  kind: f.kind,
  defaultEnabled: f.default_enabled,
  ...(f.keybind !== undefined ? { keybind: f.keybind } : {}),
  category: f.category,
}));

/** Default feature state objects — merged under whatever the file holds. */
export const FEATURE_DEFAULTS = Object.fromEntries(
  SUITE_REGISTRY_DOC.features.map((f) => [
    f.id,
    { enabled: f.default_enabled, ...(isPlainObject(f.state) ? f.state : {}) },
  ]),
);

// --- macro validation limits (Contract A) ---------------------------------
const MACRO_ID_RE = /^[A-Za-z0-9_-]{1,32}$/;
const MAX_MACROS = 32;
const MAX_MACRO_NAME = 40;
const MAX_KEYBIND = 32;
const MAX_ACTIONS = 16;
const MAX_ACTION_TEXT = 256;
const ACTION_TYPES = new Set(['chat', 'command']);

/**
 * Config path follows the effective game dir: with a custom game_dir the game
 * reads <game_dir>/config/espectral-client.json, so GET/PATCH must too.
 * (Instance metadata — icon, instance.json — intentionally stays in the
 * instance dir.)
 */
export function clientConfigPath(instanceName) {
  return path.join(effectiveGameDir(instanceName), 'config', 'espectral-client.json');
}

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function deepEqual(a, b) {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    return ka.every((k) => deepEqual(a[k], b[k]));
  }
  return false;
}

/** Read the RAW file; `{}` when missing or unparseable (never throws). */
function readRawConfig(instanceName) {
  try {
    const text = fs.readFileSync(clientConfigPath(instanceName), 'utf8');
    const parsed = JSON.parse(text);
    return isPlainObject(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

/**
 * Normalized view of the config for GET/launch: every default feature id
 * present as `{ enabled, ...extra }` (file wins over defaults), unknown
 * feature ids preserved, unknown top-level fields preserved, macros
 * always an array, and the schema-2 `suite.enabled` master switch present
 * (registry default when absent — the switch gates every owned feature).
 * A bare-boolean feature entry is tolerated as `{ enabled: <bool> }`.
 */
function coerceFeatureEntry(def, entry) {
  if (typeof entry === 'boolean') return { ...def, enabled: entry };
  if (isPlainObject(entry)) return { ...def, ...entry };
  return { ...def };
}

function normalizeConfig(raw) {
  const out = { ...raw, schema: raw.schema ?? CLIENT_CONFIG_SCHEMA };
  const rawSuite = isPlainObject(raw.suite) ? raw.suite : {};
  out.suite = {
    ...rawSuite,
    enabled: typeof rawSuite.enabled === 'boolean' ? rawSuite.enabled : SUITE_DEFAULT_ENABLED,
  };
  const features = {};
  for (const [id, def] of Object.entries(FEATURE_DEFAULTS)) {
    const entry = isPlainObject(raw.features) ? raw.features[id] : undefined;
    features[id] = coerceFeatureEntry(def, entry);
  }
  if (isPlainObject(raw.features)) {
    for (const [id, entry] of Object.entries(raw.features)) {
      if (id in features) continue;
      if (isPlainObject(entry)) {
        features[id] = { enabled: false, ...entry };
      } else if (typeof entry === 'boolean') {
        features[id] = { enabled: entry };
      }
    }
  }
  out.features = features;
  out.macros = Array.isArray(raw.macros) ? raw.macros : [];
  return out;
}

/** Load + normalize. Never throws; missing file -> pure defaults. */
export function loadClientConfig(instanceName) {
  return normalizeConfig(readRawConfig(instanceName));
}

/** Atomic write: temp file + rename in the same directory. */
function writeConfigAtomic(instanceName, obj) {
  const file = clientConfigPath(instanceName);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(obj, null, 2) + '\n', 'utf8');
  fs.renameSync(tmp, file);
}

/**
 * `supported` = fabric loader AND version in the pinned QoL set — the
 * versions where the native client suite runs.
 */
export function isSupported(instance) {
  return instance.loader === 'fabric'
    && Object.prototype.hasOwnProperty.call(PINS_QOL_BY_VERSION, instance.version);
}

/** GET /api/instances/:name/client -> ClientInfo. */
export async function getClientInfo(instanceName) {
  const instance = await getInstance(instanceName);
  return {
    config: loadClientConfig(instanceName),
    registry: REGISTRY,
    supported: isSupported(instance),
  };
}

/**
 * Seed defaults into the instance config at launch. MERGES into an
 * existing file (file wins per feature id / existing macros); never
 * overwrites wholesale, never throws — a launch must not fail over the
 * client config. Writes only when something actually changed. Upgrades a
 * v1 file in place (sets `schema: 2`, adds `suite.enabled` from the
 * registry) without touching stored feature flags.
 */
export function seedClientConfig(instanceName) {
  try {
    const raw = readRawConfig(instanceName);
    const merged = { ...raw };
    if (merged.schema === undefined || merged.schema === 1) merged.schema = CLIENT_CONFIG_SCHEMA;
    const suiteEntry = isPlainObject(merged.suite) ? { ...merged.suite } : {};
    if (typeof suiteEntry.enabled !== 'boolean') suiteEntry.enabled = SUITE_DEFAULT_ENABLED;
    merged.suite = suiteEntry;
    const features = isPlainObject(merged.features) ? { ...merged.features } : {};
    let changed = !deepEqual(merged, raw);
    for (const [id, def] of Object.entries(FEATURE_DEFAULTS)) {
      const entry = features[id];
      const filled = coerceFeatureEntry(def, entry);
      const base = typeof entry === 'boolean' ? { enabled: entry } : entry;
      if (!deepEqual(filled, base)) {
        features[id] = filled;
        changed = true;
      } else if (typeof entry === 'boolean') {
        // Tolerated on read, but persist the canonical object form.
        features[id] = filled;
        changed = true;
      }
    }
    merged.features = features;
    if (!Array.isArray(merged.macros)) {
      merged.macros = [];
      changed = true;
    }
    if (changed) writeConfigAtomic(instanceName, merged);
  } catch (e) {
    console.warn(`[client] could not seed espectral-client.json: ${e.message}`);
  }
}

// --- PATCH validation -------------------------------------------------------

function badMacro(i, message) {
  throw httpError(400, 'BAD_MACRO', `macros[${i}]: ${message}`);
}

function validateMacro(m, i, seenIds) {
  if (!isPlainObject(m)) badMacro(i, 'must be an object');
  if (typeof m.id !== 'string' || !MACRO_ID_RE.test(m.id)) {
    badMacro(i, 'id must match ^[A-Za-z0-9_-]{1,32}$');
  }
  if (seenIds.has(m.id)) badMacro(i, `duplicate id '${m.id}'`);
  seenIds.add(m.id);
  if (typeof m.name !== 'string' || m.name.length === 0 || m.name.length > MAX_MACRO_NAME) {
    badMacro(i, 'name must be a non-empty string of at most 40 chars');
  }
  if (typeof m.keybind !== 'string' || m.keybind.length === 0 || m.keybind.length > MAX_KEYBIND) {
    badMacro(i, 'keybind must be a non-empty string of at most 32 chars');
  }
  if (!Array.isArray(m.actions) || m.actions.length === 0 || m.actions.length > MAX_ACTIONS) {
    badMacro(i, `actions must be a non-empty array of at most ${MAX_ACTIONS} entries`);
  }
  m.actions.forEach((a, j) => {
    if (!isPlainObject(a) || !ACTION_TYPES.has(a.type)) {
      badMacro(i, `actions[${j}].type must be 'chat' or 'command'`);
    }
    if (typeof a.text !== 'string' || a.text.length === 0 || a.text.length > MAX_ACTION_TEXT) {
      badMacro(i, `actions[${j}].text must be a non-empty string of at most 256 chars`);
    }
  });
}

function validatePatch(patch) {
  if (!isPlainObject(patch)) {
    throw httpError(400, 'BAD_PATCH', 'PATCH body must be a JSON object');
  }
  const { features, macros, suite, ...rest } = patch;
  if (Object.keys(rest).length > 0) {
    throw httpError(400, 'BAD_PATCH', `unknown PATCH fields: ${Object.keys(rest).join(', ')}`);
  }
  if (suite !== undefined) {
    if (!isPlainObject(suite)) {
      throw httpError(400, 'BAD_PATCH', 'suite must be an object ({ enabled, ... })');
    }
    if ('enabled' in suite && typeof suite.enabled !== 'boolean') {
      throw httpError(400, 'BAD_PATCH', 'suite.enabled must be a boolean');
    }
  }
  if (features !== undefined) {
    if (!isPlainObject(features)) {
      throw httpError(400, 'BAD_FEATURES', 'features must be an object keyed by feature id');
    }
    for (const [id, entry] of Object.entries(features)) {
      if (typeof entry === 'boolean') continue;
      if (!isPlainObject(entry)) {
        throw httpError(400, 'BAD_FEATURES', `features.${id} must be an object ({ enabled, ... })`);
      }
      if ('enabled' in entry && typeof entry.enabled !== 'boolean') {
        throw httpError(400, 'BAD_FEATURES', `features.${id}.enabled must be a boolean`);
      }
    }
  }
  if (macros !== undefined) {
    if (!Array.isArray(macros)) {
      throw httpError(400, 'BAD_MACROS', 'macros must be an array');
    }
    if (macros.length > MAX_MACROS) {
      throw httpError(400, 'TOO_MANY_MACROS', `at most ${MAX_MACROS} macros per instance`);
    }
    const seenIds = new Set();
    macros.forEach((m, i) => validateMacro(m, i, seenIds));
  }
}

/**
 * Legacy no-op: fullbright/nofog used to be third-party jars toggled by
 * rename (mods.mjs setModEnabled). Both are native owned features now, so
 * there is nothing to reconcile — leftover gamma-utils/clear-fog jars are
 * simply inert. Always returns [] (kept so PATCH keeps its additive
 * `errors` shape for future per-feature warnings).
 */
async function reconcileManagedJars() {
  return [];
}

/**
 * PATCH /api/instances/:name/client.
 *
 * Body: `{ features?, macros?, suite? }` — all optional. `features`
 * shallow-merges per feature id into the RAW file (unknown ids and extra
 * keys preserved; a bare boolean means `{ enabled }`); `macros` replaces
 * the array wholesale after validation; `suite` shallow-merges into the
 * schema-2 master switch. Unknown top-level fields in the file survive.
 * Reconcile is a legacy no-op (always []); the config flags record user
 * intent and the native mod applies them live.
 *
 * Response: the same ClientInfo as GET plus an always-present additive
 * `errors: Array<{ feature, message }>` (empty when clean).
 */
export async function patchClientConfig(instanceName, patch) {
  validatePatch(patch);
  const instance = await getInstance(instanceName);

  const raw = readRawConfig(instanceName);
  if (raw.schema === undefined || raw.schema === 1) raw.schema = CLIENT_CONFIG_SCHEMA;

  if (patch.suite !== undefined) {
    raw.suite = { ...(isPlainObject(raw.suite) ? raw.suite : {}), ...patch.suite };
  }
  if (patch.features !== undefined) {
    const features = isPlainObject(raw.features) ? raw.features : {};
    for (const [id, entry] of Object.entries(patch.features)) {
      const base = isPlainObject(features[id])
        ? features[id]
        : typeof features[id] === 'boolean' ? { enabled: features[id] } : {};
      features[id] = { ...base, ...(typeof entry === 'boolean' ? { enabled: entry } : entry) };
    }
    raw.features = features;
  }
  if (patch.macros !== undefined) {
    raw.macros = patch.macros;
  }

  writeConfigAtomic(instanceName, raw);

  const config = loadClientConfig(instanceName);
  const errors = await reconcileManagedJars();
  return {
    config,
    registry: REGISTRY,
    supported: isSupported(instance),
    errors,
  };
}
