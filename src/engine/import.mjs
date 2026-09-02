// src/engine/import.mjs — settings import from other launchers.
//
// Minecraft settings are shared through the vanilla game dir: launchers that run
// the game with gameDirectory=%APPDATA%\.minecraft write vanilla-format
// options.txt (including their own mods' key_* keybinds) and servers.dat (NBT)
// there. Importing copies those files into the target instance at import time
// only — never at launch, and never clobbering existing files. Lunar Client
// installs additionally expose allocatedMemory (-> instance default memory) and
// optionsLC.txt (a JSON options mirror, fov in DEGREES) which are merged in.
//
// Formats:
//   options.txt  — properties file: first line 'version:N' header, then key:value lines.
//                  Values may contain ':' (split on the FIRST colon). Unknown key_* lines
//                  (mod keybinds) must round-trip verbatim.
//   optionsLC.txt — JSON mirror written by Lunar into the game dir. Values are strings
//                  (some numbers). fov is in degrees (e.g. "110"); options.txt fov is a
//                  0-2 multiplier where 1.0 == 70deg, so fov_degrees/70.
//   servers.dat  — NBT; parsed via ./nbt.mjs parseServersDat(buf) -> ServerEntry[].

import { promises as fsp } from 'node:fs'
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { parseServersDat } from './nbt.mjs'
import { dataDir } from './config.mjs'

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export class ImportError extends Error {
  constructor(message, status = 400, code = 'BAD_REQUEST') {
    super(message)
    this.name = 'ImportError'
    this.status = status // server.mjs formats thrown errors from err.status
    this.statusCode = status
    this.code = code
  }
}

// ---------------------------------------------------------------------------
// Path helpers (Windows %APPDATA% convention per contract)
// ---------------------------------------------------------------------------

function getAppData() {
  return process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming')
}

/**
 * Import containment guard verbatim: true when `candidate` is not strictly
 * inside `root` after resolution. A strict subpath of `root` is contained;
 * equality with `root` itself counts as escaping. Both paths are
 * path.resolve()d first and the check is lexical — symlinks are not followed,
 * which is the defense-in-depth intent (the two-hop whitelist only needs the
 * source/target dirs to be bounded). Exported for direct unit testing of the
 * guard logic (FINDINGS iteration 39 backlog).
 */
export function pathEscapes(root, candidate) {
  const resolvedRoot = path.resolve(root)
  const resolvedCandidate = path.resolve(candidate)
  return !resolvedCandidate.startsWith(resolvedRoot + path.sep)
}

// ---------------------------------------------------------------------------
// options.txt parse / write
// ---------------------------------------------------------------------------

/**
 * Parse options.txt text. Returns { header, entries, eol } where header is the raw
 * 'version:N' first line (or null), entries are { key, value, raw } in file order,
 * and eol ('\n' or '\r\n') is the file's line ending so untouched files round-trip
 * byte-identical. `raw` keeps every original line verbatim so unknown lines
 * round-trip unchanged; entries updated via setOptionKey() get raw=null and are
 * re-serialized from key/value on write.
 */
export function parseOptionsTxt(text) {
  const src = String(text)
  const eol = src.includes('\r\n') ? '\r\n' : '\n'
  const entries = []
  let header = null
  const lines = src.split(/\r\n|\n/)
  for (const line of lines) {
    if (line === '') continue
    if (header === null && /^version:\d+$/.test(line)) {
      header = line
      continue
    }
    const idx = line.indexOf(':')
    if (idx > 0) entries.push({ key: line.slice(0, idx), value: line.slice(idx + 1), raw: line })
    else entries.push({ key: null, value: null, raw: line })
  }
  return { header, entries, eol }
}

/** Serialize a parsed options.txt; header first, untouched lines verbatim, original EOL. */
export function writeOptionsTxt(parsed) {
  const eol = parsed.eol || '\n'
  const parts = []
  if (parsed.header) parts.push(parsed.header)
  for (const e of parsed.entries) parts.push(e.raw != null ? e.raw : `${e.key}:${e.value}`)
  return parts.length ? parts.join(eol) + eol : ''
}

/** Set/insert a key on a parsed options.txt (re-serializes only that entry). */
export function setOptionKey(parsed, key, value) {
  const entry = parsed.entries.find((e) => e.key === key)
  if (entry) {
    entry.value = String(value)
    entry.raw = null
  } else {
    parsed.entries.push({ key, value: String(value), raw: null })
  }
  return parsed
}

/** Number of key:value entries (excludes the version:N header). */
export function countOptionsKeys(text) {
  return parseOptionsTxt(text).entries.reduce((n, e) => n + (e.key != null ? 1 : 0), 0)
}

/** options.txt -> [ [k, v], ... ] pairs for the instance options endpoint / UI table. */
export function readOptionsTxt(text) {
  return parseOptionsTxt(text)
    .entries.filter((e) => e.key != null)
    .map((e) => [e.key, e.value])
}

// ---------------------------------------------------------------------------
// Lunar enrichment helpers
// ---------------------------------------------------------------------------

/** Read Lunar's JSON options mirror (optionsLC.txt) from a game dir; null if absent/unparseable. */
function readOptionsLc(gameDir) {
  const p = path.join(gameDir, 'optionsLC.txt')
  if (!existsSync(p)) return null
  try {
    const obj = JSON.parse(readFileSync(p, 'utf8'))
    return obj && typeof obj === 'object' && !Array.isArray(obj) ? obj : null
  } catch {
    return null
  }
}

/**
 * Lunar mods.json lives at <lunar settings>/game/<profileName>/mods.json. The active
 * profile is the one Lunar last wrote (settings/game/* dirs carry per-profile state;
 * the Default profile's mods.json typically has the newest mtime).
 * Returns the top-level mod names whose toggle is not explicitly disabled.
 */
function readLunarModToggles(lunarSettingsDir) {
  const gameDir = path.join(lunarSettingsDir, 'game')
  if (!existsSync(gameDir)) return []
  let best = null // { mtimeMs, file }
  for (const entry of readdirSync(gameDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue
    const file = path.join(gameDir, entry.name, 'mods.json')
    if (!existsSync(file)) continue
    try {
      const mtimeMs = statSync(file).mtimeMs
      if (!best || mtimeMs > best.mtimeMs) best = { mtimeMs, file }
    } catch {
      // unreadable mods.json — skip
    }
  }
  if (!best) return []
  try {
    const obj = JSON.parse(readFileSync(best.file, 'utf8'))
    if (!obj || typeof obj !== 'object') return []
    const toggles = []
    for (const [name, cfg] of Object.entries(obj)) {
      if (name === 'version') continue
      if (cfg && typeof cfg === 'object' && cfg.enabled === false) continue
      toggles.push(name)
    }
    return toggles
  } catch {
    return []
  }
}

/**
 * Whitelist of optionsLC.txt keys merged into options.txt on a lunar import.
 * Excluded on purpose: 'lastLaunchedVersion' (not an option), 'key_*' mod keybinds
 * (already carried verbatim inside options.txt itself), the duplicate 'maxFPS'
 * (options.txt uses 'maxFps'), and resource pack lists (Lunar-specific packs would
 * not resolve on a vanilla client).
 */
const LC_MERGE_KEYS = new Set([
  // video / quality
  'ao', 'biomeBlendRadius', 'chunkSectionFadeInTime', 'cutoutLeaves', 'enableVsync',
  'entityDistanceScaling', 'entityShadows', 'forceUnicodeFont', 'japaneseGlyphVariants',
  'fov', 'fovEffectScale', 'darknessEffectScale', 'glintSpeed', 'glintStrength',
  'damageTiltStrength', 'gamma', 'renderDistance', 'simulationDistance', 'guiScale',
  'maxFps', 'mouseSensitivity', 'rawMouseInput', 'menuBackgroundBlurriness', 'particles',
  'mipmapLevels', 'maxAnisotropyBit', 'screenEffectScale', 'fullscreen', 'exclusiveFullscreen',
  'graphicsPreset', 'graphicsMode', 'renderClouds', 'cloudRange', 'textureFiltering',
  'preferredGraphicsBackend', 'improvedTransparency', 'vignette', 'weatherRadius',
  'prioritizeChunkUpdates', 'panoramaScrollSpeed', 'rotateWithMinecart', 'highContrast',
  'highContrastBlockOutline',
  // input / controls
  'invertXMouse', 'invertYMouse', 'toggleAttack', 'toggleUse', 'sprintWindow',
  'allowCursorChanges', 'saveChatDrafts', 'touchscreen', 'discrete_mouse_scroll', 'bobView',
  'toggleCrouch', 'toggleSprint', 'autoJump', 'mouseWheelSensitivity', 'useNativeTransport',
  'inactivityFpsLimit', 'soundDevice',
  // accessibility / chat / UI
  'narratorHotkey', 'narrator', 'chatOpacity', 'chatScale', 'chatWidth', 'chatHeightFocused',
  'chatHeightUnfocused', 'chatLineSpacing', 'textBackgroundOpacity', 'backgroundForChatOnly',
  'chatDelay', 'chatVisibility', 'chatColors', 'chatLinks', 'chatLinksPrompt', 'showSubtitles',
  'reducedDebugInfo', 'hideServerAddress', 'advancedItemTooltips', 'pauseOnLostFocus',
  'overrideWidth', 'overrideHeight', 'notificationDisplayTime', 'glDebugVerbosity',
  'skipMultiplayerWarning', 'skipRealms32bitWarning', 'hideMatchedNames', 'joinedFirstServer',
  'hideBundleTutorial', 'syncChunkWrites', 'showAutosaveIndicator', 'allowServerListing',
  'onlyShowSecureChat', 'telemetryOptInExtra', 'onboardAccessibility', 'startedCleanly',
  'snooperEnabled', 'darkMojangStudiosBackground', 'hideLightningFlashes', 'hideSplashTexts',
  'realmsNotifications', 'operatorItemsTab', 'autoSuggestions', 'musicToast', 'musicFrequency',
  'inGameNotification', 'sharePresence',
  // gameplay / misc
  'difficulty', 'mainHand', 'attackIndicator', 'tutorialStep', 'heldItemTooltips', 'lang',
  'lastServer',
])

/**
 * Merge Lunar's optionsLC mirror into a parsed options.txt. fov is converted from
 * degrees to the options.txt 0-2 multiplier (1.0 == 70deg) => fov_degrees / 70;
 * every other key is passed through raw (stringified).
 */
function applyLunarMerge(parsed, lc) {
  for (const key of LC_MERGE_KEYS) {
    if (!(key in lc)) continue
    const v = lc[key]
    if (v == null) continue
    if (key === 'fov') {
      const degrees = Number(v)
      if (!Number.isFinite(degrees) || degrees <= 0) continue
      setOptionKey(parsed, 'fov', String(degrees / 70))
    } else {
      setOptionKey(parsed, key, typeof v === 'string' ? v : String(v))
    }
  }
}

// ---------------------------------------------------------------------------
// Source detection
// ---------------------------------------------------------------------------

/** Build an ImportSource for a directory holding vanilla-format options.txt/servers.dat. */
async function buildSource(id, kind, label, dir) {
  const optionsPath = path.join(dir, 'options.txt')
  const serversPath = path.join(dir, 'servers.dat')
  const optionsExists = existsSync(optionsPath)
  const serversExists = existsSync(serversPath)
  let options_key_count = 0
  if (optionsExists) {
    try {
      options_key_count = countOptionsKeys(readFileSync(optionsPath, 'utf8'))
    } catch {
      options_key_count = 0
    }
  }
  let servers_entry_count = 0
  if (serversExists) {
    try {
      servers_entry_count = parseServersDat(readFileSync(serversPath)).length
    } catch {
      servers_entry_count = 0
    }
  }
  return {
    id,
    kind,
    label,
    path: dir,
    options_exists: optionsExists,
    servers_exists: serversExists,
    options_key_count,
    servers_entry_count,
  }
}

async function makeVanillaSource() {
  return buildSource('vanilla', 'vanilla', 'Minecraft (vanilla)', path.join(getAppData(), '.minecraft'))
}

async function makeFastClientSources() {
  const profilesDir = path.join(getAppData(), '.fastclient', 'profiles')
  if (!existsSync(profilesDir)) return []
  const out = []
  const names = readdirSync(profilesDir, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort()
  for (const name of names) {
    out.push(await buildSource(`fastclient:${name}`, 'fastclient', `FastClient · ${name}`, path.join(profilesDir, name)))
  }
  return out
}

export async function makeLunarSource() {
  // Lunar lives under the user's home (~/.lunarclient); some docs say %APPDATA%\.lunarclient —
  // probe both, first one with a launcher.json wins.
  const candidates = [path.join(getAppData(), '.lunarclient'), path.join(os.homedir(), '.lunarclient')]
  let settingsDir = null
  for (const candidate of candidates) {
    if (existsSync(path.join(candidate, 'settings', 'launcher.json'))) {
      settingsDir = path.join(candidate, 'settings')
      break
    }
  }
  if (!settingsDir) return null

  let launcher
  try {
    launcher = JSON.parse(readFileSync(path.join(settingsDir, 'launcher.json'), 'utf8'))
  } catch {
    return null
  }
  const settings = (launcher && typeof launcher === 'object' && launcher.settings) || {}
  let gameDir = settings.gameDirectory
  if (typeof gameDir !== 'string' || gameDir === '') return null
  // Relative gameDirectory (profiles.db stores '.minecraft') resolves against %APPDATA%.
  if (!path.isAbsolute(gameDir)) gameDir = path.resolve(getAppData(), gameDir)
  // Containment: the "two-hop whitelist" (options.txt + servers.dat) is only a
  // meaningful bound if the source dir is actually the Lunar game dir. launcher.json
  // is a local file, but defense-in-depth — only accept gameDirectory that stays
  // under the Lunar install root; an uncontained path would turn the import API
  // into an arbitrary-file copy/read oracle (import copies options.txt/servers.dat
  // from ANY dir, then GET /api/instances/:name/servers reads servers.dat back).
  const lunarRoot = settingsDir.replace(/[\\/]+settings[\\/]?$/, '')
  const absGameDir = path.resolve(gameDir)
  if (absGameDir !== lunarRoot && !absGameDir.startsWith(lunarRoot + path.sep)) {
    return null
  }

  const lunar = {}
  const mem = Number(settings.allocatedMemory)
  if (Number.isFinite(mem) && mem > 0) lunar.allocated_memory = mem
  const lc = readOptionsLc(gameDir)
  if (lc) {
    const fov = Number(lc.fov)
    if (Number.isFinite(fov)) lunar.fov_degrees = fov
  }
  const toggles = readLunarModToggles(settingsDir)
  if (toggles.length > 0) lunar.mods_json_toggles = toggles

  const source = await buildSource('lunar', 'lunar', 'Lunar Client', gameDir)
  if (Object.keys(lunar).length > 0) source.lunar = lunar
  return source
}

/**
 * Detect import sources -> ImportSource[]:
 *   1. vanilla %APPDATA%\.minecraft
 *   2. each %APPDATA%\.fastclient\profiles\<name> (id 'fastclient:<name>')
 *   3. lunar (launcher.json gameDirectory + allocatedMemory, optionsLC.txt fov,
 *      mods.json toggles from the active profile)
 */
export async function detectImportSources() {
  const sources = []
  sources.push(await makeVanillaSource())
  sources.push(...(await makeFastClientSources()))
  const lunar = await makeLunarSource()
  if (lunar) sources.push(lunar)
  return sources
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/** The FC two-hop whitelist — exactly these two files, nothing else. */
const IMPORT_FILES = ['options.txt', 'servers.dat']

/**
 * Import options.txt + servers.dat from a detected source into an instance game dir.
 *
 * @param {object} instance - { name, dir, memory_mb?, merge_optionslc?, ...instance.json }
 * @param {string} sourceId - ImportSource.id from detectImportSources()
 * @param {'never'|'if-older'} overwritePolicy - 'never' (default) never touches existing
 *   target files; 'if-older' replaces when source mtime > target mtime and backs up
 *   servers.dat -> servers.dat.bak before overwriting.
 * @param {object} [opts] - optional { mergeOptionsLc?: boolean } override; default is
 *   instance.merge_optionslc === true (the creation-time flag).
 * @returns {Promise<ImportResult>}
 */
export async function importProfile(instance, sourceId, overwritePolicy = 'never', opts = {}) {
  if (overwritePolicy !== 'never' && overwritePolicy !== 'if-older') {
    throw new ImportError('overwrite_policy must be "never" or "if-older"', 400, 'BAD_OVERWRITE_POLICY')
  }
  if (!instance || typeof instance !== 'object' || typeof instance.name !== 'string' || !instance.name) {
    throw new ImportError('instance is required', 400, 'BAD_INSTANCE')
  }
  const sources = await detectImportSources()
  const source = sources.find((s) => s.id === sourceId)
  if (!source) throw new ImportError(`import source not found: ${sourceId}`, 404, 'SOURCE_NOT_FOUND')

  const targetDir = instance.dir || path.join(dataDir(), 'instances', instance.name)
  await fsp.mkdir(targetDir, { recursive: true })

  // Defense-in-depth: never let a source drag in a file that isn't under its own
  // directory (malformed/path-traversal sourceIds or a tampered launcher.json
  // gameDirectory). Without this, the "whitelist" (options.txt/servers.dat) would
  // let an attacker write arbitrary bytes into the instance dir via servers.dat.
  const sourceRoot = path.resolve(source.path)
  const dstRoot = path.resolve(targetDir)
  const copied = []
  const skipped = []
  for (const file of IMPORT_FILES) {
    const src = path.join(sourceRoot, file)
    if (pathEscapes(sourceRoot, src)) {
      throw new ImportError(`import source path escapes its directory: ${sourceId}`, 400, 'SOURCE_PATH_ESCAPE')
    }
    const dst = path.join(dstRoot, file)
    if (pathEscapes(dstRoot, dst)) {
      throw new ImportError(`import target path escapes its directory`, 400, 'TARGET_PATH_ESCAPE')
    }
    if (!existsSync(src)) {
      skipped.push({ file, reason: 'not present in source' })
      continue
    }
    if (existsSync(dst)) {
      if (overwritePolicy === 'never') {
        skipped.push({ file, reason: 'target already exists and overwrite_policy is "never"' })
        continue
      }
      const srcMtime = statSync(src).mtimeMs
      const dstMtime = statSync(dst).mtimeMs
      if (srcMtime <= dstMtime) {
        skipped.push({ file, reason: 'source is not newer than target' })
        continue
      }
      if (file === 'servers.dat') await fsp.copyFile(dst, `${dst}.bak`) // backup before overwrite
      await fsp.copyFile(src, dst)
      copied.push(file)
    } else {
      await fsp.copyFile(src, dst)
      copied.push(file)
    }
  }

  // Lunar enrichments (only meaningful when the source itself is lunar)
  let lunar
  if (source.kind === 'lunar') {
    lunar = source.lunar ? { ...source.lunar } : undefined
    const mergeLc = opts.mergeOptionsLc ?? opts.merge_optionslc ?? instance.merge_optionslc === true
    if (mergeLc && copied.includes('options.txt')) {
      const lc = readOptionsLc(source.path)
      if (lc) {
        const optionsPath = path.join(targetDir, 'options.txt')
        const parsed = parseOptionsTxt(readFileSync(optionsPath, 'utf8'))
        applyLunarMerge(parsed, lc)
        await fsp.writeFile(optionsPath, writeOptionsTxt(parsed), 'utf8')
      }
    }
    // allocatedMemory -> instance default memory when unset
    if (source.lunar && source.lunar.allocated_memory && !instance.memory_mb) {
      instance.memory_mb = source.lunar.allocated_memory
      await persistInstanceMemory(targetDir, instance)
    }
  }

  // Parse servers.dat into ServerEntry[] for the result + UI preview: the instance's
  // current state (target), falling back to the source file when nothing was imported.
  let servers_parsed = []
  const targetDat = path.join(targetDir, 'servers.dat')
  const datPath = existsSync(targetDat) ? targetDat : path.join(source.path, 'servers.dat')
  if (existsSync(datPath)) {
    try {
      servers_parsed = parseServersDat(readFileSync(datPath))
    } catch {
      servers_parsed = []
    }
  }

  const optionsPath = path.join(targetDir, 'options.txt')
  const options_keys = existsSync(optionsPath) ? countOptionsKeys(readFileSync(optionsPath, 'utf8')) : 0

  return { copied, skipped, servers_parsed, options_keys, lunar }
}

/** Persist a memory_mb change back into instance.json, preserving every other field. */
async function persistInstanceMemory(targetDir, instance) {
  const p = path.join(targetDir, 'instance.json')
  if (!existsSync(p)) return // instance.json not created yet — the create flow sets memory itself
  try {
    const json = JSON.parse(readFileSync(p, 'utf8'))
    if (json && typeof json === 'object') {
      json.memory_mb = instance.memory_mb
      // Atomic write — same .tmp+rename pattern as instances.mjs writeJson:
      // a crash mid-write must not leave a truncated instance.json that 404s
      // every later read (L22).
      const tmp = `${p}.tmp`
      await fsp.writeFile(tmp, JSON.stringify(json, null, 2), 'utf8')
      await fsp.rename(tmp, p)
    }
  } catch {
    // never let a metadata write fail the import
  }
}
