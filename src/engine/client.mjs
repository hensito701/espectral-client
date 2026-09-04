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
import * as resolver from './resolver.mjs';
import { getInstance } from './instances.mjs';
import { PINS_QOL_BY_VERSION } from './mods.mjs';
import { httpError } from './error.mjs';

/**
 * The v1.0.0 feature registry — the SAME list the in-game mod ships
 * (Contract A). GET /api/instances/:name/client returns it verbatim so
 * the UI renders names/descriptions without hardcoding them.
 */
export const REGISTRY = [
  { id: 'fullbright', name: 'Fullbright', description: 'Built-in fullbright — gamma driven live by the mod. No jars, no restart.', kind: 'owned', defaultEnabled: true },
  { id: 'nofog', name: 'No Fog', description: 'Built-in no-fog — fog removed live by the mod (Overworld, Nether, End). No jars, no restart.', kind: 'owned', defaultEnabled: false },
  { id: 'zoom', name: 'Zoom', description: 'Built-in hold-to-zoom on Z — smooth FOV ease, applied live.', kind: 'owned', defaultEnabled: true, keybind: 'key.keyboard.z' },
  { id: 'macros', name: 'Macros', description: 'Keybind macros — run chat/command sequences.', kind: 'owned', defaultEnabled: true },
  { id: 'potionstatus', name: 'Potion Status', description: 'On-screen list of active potion effects with durations.', kind: 'owned', defaultEnabled: false },
  { id: 'coords', name: 'Coords Display', description: 'On-screen XYZ coordinates and facing direction.', kind: 'owned', defaultEnabled: false },
  { id: 'healthstatus', name: 'Health Display', description: 'Numeric health and absorption readout on screen.', kind: 'owned', defaultEnabled: false },
  { id: 'armorstatus', name: 'Armor Status', description: 'On-screen armor durability for each equipped piece.', kind: 'owned', defaultEnabled: false },
  { id: 'fpsping', name: 'FPS / Ping', description: 'On-screen frames-per-second and server latency.', kind: 'owned', defaultEnabled: false },
  { id: 'lowfire', name: 'Low Fire', description: 'Lowers the burning fire overlay so it blocks less of the view.', kind: 'owned', defaultEnabled: false },
  { id: 'clearwater', name: 'Clear Water', description: 'Removes the underwater overlay and water fog for clear vision.', kind: 'owned', defaultEnabled: false },
  { id: 'chatheads', name: 'Chat Heads', description: 'Shows each player head next to their name in chat.', kind: 'owned', defaultEnabled: false },
  { id: 'skin3d', name: '3D Skin Layers', description: 'Renders skin outer layers (hat, jacket, sleeves) as real 3D voxels.', kind: 'owned', defaultEnabled: false },
];

/** Default feature state objects — merged under whatever the file holds. */
export const FEATURE_DEFAULTS = {
  fullbright: { enabled: true, gamma: 15.0 },
  nofog: { enabled: false, radius: null },
  zoom: { enabled: true, fov: 30.0, smooth: true },
  macros: { enabled: true },
  potionstatus: { enabled: false },
  coords: { enabled: false },
  healthstatus: { enabled: false },
  armorstatus: { enabled: false },
  fpsping: { enabled: false },
  lowfire: { enabled: false },
  clearwater: { enabled: false },
  chatheads: { enabled: false },
  skin3d: { enabled: false },
};

// --- macro validation limits (Contract A) ---------------------------------
const MACRO_ID_RE = /^[A-Za-z0-9_-]{1,32}$/;
const MAX_MACROS = 32;
const MAX_MACRO_NAME = 40;
const MAX_KEYBIND = 32;
const MAX_ACTIONS = 16;
const MAX_ACTION_TEXT = 256;
const ACTION_TYPES = new Set(['chat', 'command']);

export function clientConfigPath(instanceName) {
  return path.join(resolver.instanceDir(instanceName), 'config', 'espectral-client.json');
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
 * always an array.
 */
function normalizeConfig(raw) {
  const out = { ...raw, schema: raw.schema ?? 1 };
  const features = {};
  for (const [id, def] of Object.entries(FEATURE_DEFAULTS)) {
    const entry = isPlainObject(raw.features) ? raw.features[id] : undefined;
    features[id] = isPlainObject(entry) ? { ...def, ...entry } : { ...def };
  }
  if (isPlainObject(raw.features)) {
    for (const [id, entry] of Object.entries(raw.features)) {
      if (!(id in features) && isPlainObject(entry)) {
        features[id] = { enabled: false, ...entry };
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
 * client config. Writes only when something actually changed.
 */
export function seedClientConfig(instanceName) {
  try {
    const raw = readRawConfig(instanceName);
    const merged = { ...raw };
    if (merged.schema === undefined) merged.schema = 1;
    const features = isPlainObject(merged.features) ? { ...merged.features } : {};
    let changed = !deepEqual(merged, raw);
    for (const [id, def] of Object.entries(FEATURE_DEFAULTS)) {
      const entry = features[id];
      if (isPlainObject(entry)) {
        const filled = { ...def, ...entry };
        if (!deepEqual(filled, entry)) {
          features[id] = filled;
          changed = true;
        }
      } else {
        features[id] = { ...def };
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
  const { features, macros, ...rest } = patch;
  if (Object.keys(rest).length > 0) {
    throw httpError(400, 'BAD_PATCH', `unknown PATCH fields: ${Object.keys(rest).join(', ')}`);
  }
  if (features !== undefined) {
    if (!isPlainObject(features)) {
      throw httpError(400, 'BAD_FEATURES', 'features must be an object keyed by feature id');
    }
    for (const [id, entry] of Object.entries(features)) {
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
 * Body: `{ features?, macros? }` — both optional. `features` shallow-
 * merges per feature id into the RAW file (unknown ids and extra keys
 * preserved); `macros` replaces the array wholesale after validation.
 * Unknown top-level fields in the file survive. Reconcile is a legacy
 * no-op (always []); the config flags record user intent and the native
 * mod applies them live.
 *
 * Response: the same ClientInfo as GET plus an always-present additive
 * `errors: Array<{ feature, message }>` (empty when clean).
 */
export async function patchClientConfig(instanceName, patch) {
  validatePatch(patch);
  const instance = await getInstance(instanceName);

  const raw = readRawConfig(instanceName);
  if (raw.schema === undefined) raw.schema = 1;

  if (patch.features !== undefined) {
    const features = isPlainObject(raw.features) ? raw.features : {};
    for (const [id, entry] of Object.entries(patch.features)) {
      features[id] = { ...(isPlainObject(features[id]) ? features[id] : {}), ...entry };
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
