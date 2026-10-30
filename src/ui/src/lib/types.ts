/* ==========================================================================
   Shared JSON shapes — mirrors of the engine REST API (src/engine routes).
   Field names MUST match the contract exactly; the UI consumes these.
   ========================================================================== */

export const THEMES = ['dark', 'light', 'system'] as const;
export type Theme = typeof THEMES[number];
// Workstream 4 accent ids — same set as the engine THEME_IDS and the mod palettes.
export const ACCENTS = ['oro', 'carmesi', 'amatista', 'esmeralda', 'glaciar'] as const;
export type Accent = typeof ACCENTS[number];
export const LOADERS = ['vanilla', 'fabric', 'neoforge'] as const;
export type Loader = typeof LOADERS[number];
export type OverwritePolicy = 'never' | 'if-older';
export type ImportSourceKind = 'vanilla' | 'fastclient' | 'lunar';
export type JvmSource = 'bundled' | 'fastclient' | 'path' | 'downloaded';
export type ModPreset = 'performance' | 'branding' | 'qol';

export interface ServerStatus {
  host: string;
  online: boolean;
  hostname: string;
  ip: string;
  port: number;
  version: string;
  software: string;
  motd_raw: string[];
  motd_clean: string[];
  players_online: number;
  players_max: number;
  icon?: string;
  fetched_at: number;
  error?: string;
}

export interface VersionEntry {
  id: string;
  type: string;
  release_time: string;
  sha1: string;
  url: string;
}

export interface VersionManifest {
  latest_release: string;
  latest_snapshot: string;
  versions: VersionEntry[];
}

export interface VersionInfo {
  id: string;
  java_major: number;
  supported: boolean;
  reason: string;
}

export interface JvmInfo {
  path: string;
  version: string;
  build: string;
  vendor: string;
  source: JvmSource;
  supported_majors?: number[];
}

export type AccountTokenKind = 'offline' | 'msa';

export interface Account {
  username: string;
  uuid: string;
  token_kind: AccountTokenKind;
  created_at: string;
  last_used: string;
  avatar_color: number | null;
  has_avatar: boolean;
}

export interface MsDeviceFlow {
  flow_id: string;
  user_code: string;
  verification_uri: string;
  /** RFC 8628 verification URI with the code embedded; null when the tenant omits it. */
  verification_uri_complete: string | null;
  expires_in: number;
  interval: number;
}

export interface AppConfig {
  /** Effective default heap ceiling: the Settings value, else the system-RAM recommendation. */
  default_memory_mb: number;
  recommended_memory_mb: number;
  /** True when no default is pinned in Settings (automatic). */
  default_memory_auto: boolean;
  /** Close the launcher once a game reaches its menu (the game keeps running). */
  close_on_game_start: boolean;
  download_concurrency: number;
  aot_auto_train: boolean;
  fast_boot: boolean;
  fullbright_on_launch: boolean;
  jdk_path_override: string;
  discord_enabled: boolean;
  /** Workstream 4: launcher accent id + global reduced-motion mirror. */
  accent?: Accent;
  reduced_motion?: boolean;
  data_dir: string;
}

export interface InstanceSummary {
  name: string;
  version: string;
  loader: Loader;
  loader_version: string | null;
  modpack: string | null;
  modpack_version: string | null;
  /** Effective heap ceiling (the instance's own value, else the automatic default). */
  memory_mb: number;
  /** True when the instance follows the automatic default. */
  memory_auto: boolean;
  mod_count: number;
  enabled_mod_count: number;
  aot_key: string | null;
  aot_cache_exists: boolean;
  imported_from: string | null;
  created_at: string;
  hue: number | null;
  game_dir: string | null;
  has_icon: boolean;
  /** Installs Replay Mod + shader pack + Iris/Sodium pins (Fabric 1.21.11/26.2/26.3). */
  capture_tools?: boolean;
}

export interface ModEntry {
  filename: string;
  project_slug: string;
  version_number: string;
  version_id: string;
  sha1: string;
  size: number;
  enabled: boolean;
  installed: boolean;
}

export interface ModPresetInfo {
  supported: boolean;
  loader: Loader;
  note: string | null;
  branding?: {
    supported: boolean;
    note: string | null;
  };
}

export interface ModrinthProject {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  icon_url: string | null;
  downloads: number;
  game_versions: string[];
  latest_version_number: string;
  installed?: boolean;
}

export interface ModRef {
  project_id: string;
  title: string;
}

/** What a single-mod install brings along (POST install-modrinth reply). */
export interface ModInstallPlan {
  queued: true;
  project: ModRef;
  dependencies: (ModRef & { filename: string })[];
  missing: ModRef[];
  optional: ModRef[];
  conflicts: ModRef[];
}

export interface ServerEntry {
  name: string;
  ip: string;
  hidden: boolean;
  accept_textures: boolean;
  has_icon: boolean;
}

export interface LunarInfo {
  allocated_memory?: number;
  fov_degrees?: number;
  mods_json_toggles?: string[];
}

export interface ImportSource {
  id: string;
  kind: ImportSourceKind;
  label: string;
  path: string;
  options_exists: boolean;
  servers_exists: boolean;
  options_key_count: number;
  servers_entry_count: number;
  lunar?: LunarInfo;
}

export interface ImportSkipped {
  file: string;
  reason: string;
}

export interface ImportResult {
  copied: string[];
  skipped: ImportSkipped[];
  servers_parsed: ServerEntry[];
  options_keys: number;
  lunar?: LunarInfo;
}

export interface AotProof {
  log_path: string;
  using_aot_linked_classes: boolean;
  /** JVM's own refusal line when the cache was passed but not used. */
  refusal?: string | null;
}

export interface AotStatus {
  key: string;
  cache_path: string;
  cache_exists: boolean;
  cache_size_bytes: number;
  /** Classpath drift vs the cache stamp; null = unverifiable. */
  stale?: boolean | null;
  /** False for loaders with no AOT tier (NeoForge). */
  ready_to_train?: boolean;
  trained_at?: string;
  proof?: AotProof;
}

export interface DryRunResult {
  argv: string[];
  classpath: string[];
  classpath_missing: string[];
  natives_dir: string;
  assets_ok: boolean;
  java_path: string;
  warnings: string[];
}

export interface LaunchReply {
  key: string;
  pid?: number;
}

export interface LaunchStat {
  key: string;
  instance: string;
  version: string;
  started_at: number;
  menu_at: number | null;
  menu_ms: number | null;
  played_ms: number | null;
  spawn_ms?: number | null;
  boot_ms?: number | null;
  phases?: Record<string, number> | null;
}

export interface LaunchExitEvent {
  key: string;
  instance?: string;
  account?: string;
  spawn_ms?: number | null;
  boot_ms?: number | null;
  phases?: Record<string, number> | null;
  menu_at?: number | null;
  error?: string | null;
  code?: number | null;
  signal?: string | null;
}

/* ---------- Espectral Client config (Contracts A/B) ---------- */

export type ClientFeatureKind = 'owned' | 'managed';

export interface ClientFeatureState {
  enabled: boolean;
  [k: string]: unknown;
}

export interface ClientMacroAction {
  type: 'chat' | 'command';
  text: string;
}

export interface ClientMacro {
  id: string;
  name: string;
  keybind: string;
  actions: ClientMacroAction[];
}

/** Top-level personalization block of espectral-client.json (Workstream 4). */
export interface ClientPersonalization {
  theme?: Accent;
  hud_theme?: 'cristal' | 'oro' | 'minimo' | 'pixel';
  background?: string | null;
  reduced_motion?: boolean;
  /** Title-screen line under the wordmark; null = default ("Good luck, have fun!"). */
  tagline?: string | null;
  [k: string]: unknown;
}

export interface ClientConfig {
  schema: number;
  personalization?: ClientPersonalization;
  /** Suite master switch (schema 2). Absent on old payloads — readers default to true. */
  suite?: { enabled: boolean };
  features: Record<string, ClientFeatureState>;
  macros: ClientMacro[];
}

export interface ClientRegistryEntry {
  id: string;
  name: string;
  description: string;
  kind: ClientFeatureKind;
  defaultEnabled: boolean;
  keybind?: string;
  /** Registry category id (schema 2, additive). Absent on old payloads. */
  category?: string;
}

export interface ClientReconcileError {
  feature: string;
  message: string;
}

export interface ClientInfo {
  config: ClientConfig;
  registry: ClientRegistryEntry[];
  supported: boolean;
  errors?: ClientReconcileError[];
}

/** PATCH /api/instances/:name/client — every field optional, merge semantics. */
export type ClientPatch = Partial<{
  features: Record<string, { enabled?: boolean; [k: string]: unknown }>;
  macros: ClientMacro[];
  suite: { enabled?: boolean };
  personalization: ClientPersonalization;
}>;

/** GET /api/launches — live/recent launch buffers, newest first. */
export interface LiveLaunch {
  key: string;
  instance: string;
  version: string;
  account: string;
  running: boolean;
  started_at: number;
  menu_at: number | null;
  ended_at: number | null;
  /** True for launches started by an agent (MCP) — shown as a badge in launch lists. */
  agent?: boolean;
}

export type OptionsPair = [string, string];

export interface InstanceDetail {
  summary: InstanceSummary;
  mods: ModEntry[];
  servers: ServerEntry[];
  options: OptionsPair[];
  aot: AotStatus;
}

export interface CreateInstanceRequest {
  name: string;
  version: string;
  loader: Loader;
  loader_version?: string | null;
  modpack?: string | null;
  modpack_version?: string | null;
  /** Omit for automatic memory. */
  memory_mb?: number;
  import_from?: string;
  /** Field name matches the REST contract verbatim (merge optionsLC.json into options.txt). */
  merge_optionslc?: boolean;
  hue?: number | null;
  game_dir?: string | null;
}

export interface InstancePatch {
  /** null = back to automatic. */
  memory_mb?: number | null;
  /** Toggles Replay Mod (+ shader pack, Iris/Sodium pins) for capture tools. */
  capture_tools?: boolean;
  enabled_mods?: string[];
  jdk_path_override?: string;
  aot_auto_train?: boolean;
  hue?: number | null;
  game_dir?: string | null;
  /** Rename: desired new instance name (dir move + instance.json update). */
  name?: string;
  /** Alias of `name` for the rename intent. */
  new_name?: string;
}

export interface VersionTarget {
  version: string;
  loader: Loader;
}

/** One jar's fate in a version/platform change (engine planModsForTarget). */
export interface VersionChangeMod {
  filename: string;
  title: string;
  enabled: boolean;
  action: 'keep' | 'update' | 'disable' | 'idle' | 'add';
  reason?: 'unknown' | 'no_build';
  to?: { filename: string; version_number: string };
}

export interface VersionChangePlan {
  from: { version: string; loader: Loader };
  to: { version: string; loader: Loader; loader_version: string | null };
  /** Target released before the current version (worlds may not load back). */
  downgrade: boolean;
  /** saves/ will be copied to the instance's backups/ folder first. */
  backup_worlds: boolean;
  /** An Espectral menu build exists for the target version + loader. */
  espectral_menu: boolean;
  mods: VersionChangeMod[];
}

export interface VersionChangeResult {
  summary: InstanceSummary;
  backup: string | null;
  mods: {
    updated: { from: string; to: string }[];
    added: string[];
    disabled: string[];
    errors: { filename: string; error: string }[];
  };
}

export interface LogChunk {
  lines: string[];
  cursor: number;
  running: boolean;
}

export interface HealthInfo {
  ok: boolean;
  version: string;
}

export type SkinVariant = 'classic' | 'slim';

export interface SkinInfo {
  username: string;
  variant: SkinVariant;
  has_skin: boolean;
  cape: boolean;
}

export interface SkinLibraryEntry {
  id: string;
  name: string;
  variant: SkinVariant;
  created_at: string;
  source: 'upload' | 'vanilla';
}

/* ---------- Espectral identity & supporter cosmetics (Workstream 4) ----------
   Mirrors of GET /api/identity and GET/PUT /api/cosmetics. Field names match
   the engine contract exactly. */

export interface IdentityDiscord {
  id: string;
  username: string;
  avatar: string | null;
}

export interface IdentityAccount {
  /** Mojang UUID, 32 lowercase hex without dashes. */
  uuid: string;
  username: string;
  /** True once the join/hasJoined handshake proved ownership. */
  verified: boolean;
  discord: IdentityDiscord | null;
  /** Backend-derived flags, e.g. 'supporter', 'mvp'. */
  flags: string[];
  /** Unix seconds when the current signed grant expires, or null. */
  grant_exp: number | null;
}

export interface IdentityState {
  service_url: string;
  accounts: IdentityAccount[];
  error?: string;
}

export interface CosmeticItemName {
  es: string;
  en: string;
}

export interface CosmeticItem {
  id: string;
  slot: string;
  requires: string;
  name: CosmeticItemName;
}

export interface CosmeticEmote {
  id: string;
  requires: string;
  name: CosmeticItemName;
}

/** Exclusive rule: items in `<slot>` cannot be equipped together with any listed id. */
export type CosmeticExclusiveRule = [slot: string, itemIds: string[]];

export interface CosmeticsCatalog {
  schema: number;
  slots: string[];
  exclusive: CosmeticExclusiveRule[];
  items: CosmeticItem[];
  emotes: CosmeticEmote[];
}

export interface CosmeticsAccountState {
  /** Affirmative consent: publish presence so peers see ⚛ + cosmetics. */
  share: boolean;
  /** Loadout = { slotId: itemId | null }. */
  loadout: Record<string, string | null>;
  /** Player names whose cosmetics/badges are hidden for this viewer. */
  hidden_players: string[];
  /** Local-only emotes enabled. */
  emotes: boolean;
  flags: string[];
}

export interface CosmeticsState {
  catalog: CosmeticsCatalog;
  accounts: Record<string, CosmeticsAccountState>;
}

/** PUT /api/cosmetics/:uuid — every field optional, merge semantics. */
export interface CosmeticsPatch {
  share?: boolean;
  loadout?: Record<string, string | null>;
  hidden_players?: string[];
  emotes?: boolean;
}

/* ---------- Menu background packs (Workstream 4, free) ---------- */

/** pack.json shape mirrored from <dataDir>/backgrounds/<id>/. */
export interface BackgroundPack {
  id: string;
  name: string;
  type: 'image' | 'frames';
  /** Frames per second (0 for still images). */
  fps: number;
  /** 1 for still images, N for frame packs. */
  frame_count: number;
  width: number;
  height: number;
}

/* ---------- Agent access (MCP) ---------- */

/** Ready-to-paste MCP server entry (Claude Code / Cursor / Codex format). */
export interface AgentMcpEntry {
  command: string;
  args: string[];
  env: Record<string, string>;
}

/** GET/PUT /api/agent-access, POST /api/agent-access/rotate. */
export interface AgentAccess {
  enabled: boolean;
  /** host[:port] entries where agent act-tier tools are allowed. */
  allowed_servers: string[];
  /** Absolute path of the 0600 token file (the token itself is never returned). */
  token_path: string;
  mcp: AgentMcpEntry;
}
