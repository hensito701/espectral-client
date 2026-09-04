/* ==========================================================================
   Shared JSON shapes — mirrors of the engine REST API (src/engine routes).
   Field names MUST match the contract exactly; the UI consumes these.
   ========================================================================== */

export const THEMES = ['dark', 'light', 'system'] as const;
export type Theme = typeof THEMES[number];
export const LOADERS = ['vanilla', 'fabric', 'neoforge'] as const;
export type Loader = typeof LOADERS[number];
export type LaunchMode = 'normal' | 'aot';
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
  expires_in: number;
  interval: number;
}

export interface AppConfig {
  default_memory_mb: number;
  download_concurrency: number;
  aot_auto_train: boolean;
  fast_boot: boolean;
  fullbright_on_launch: boolean;
  jdk_path_override: string;
  discord_enabled: boolean;
  data_dir: string;
}

export interface InstanceSummary {
  name: string;
  version: string;
  loader: Loader;
  loader_version: string | null;
  modpack: string | null;
  modpack_version: string | null;
  memory_mb: number;
  mod_count: number;
  enabled_mod_count: number;
  aot_key: string | null;
  aot_cache_exists: boolean;
  imported_from: string | null;
  created_at: string;
  hue: number | null;
  game_dir: string | null;
  has_icon: boolean;
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
}

export interface AotStatus {
  key: string;
  cache_path: string;
  cache_exists: boolean;
  cache_size_bytes: number;
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

export interface ClientConfig {
  schema: number;
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
  memory_mb: number;
  import_from?: string;
  /** Field name matches the REST contract verbatim (merge optionsLC.json into options.txt). */
  merge_optionslc?: boolean;
  hue?: number | null;
  game_dir?: string | null;
}

export interface InstancePatch {
  memory_mb?: number;
  enabled_mods?: string[];
  jdk_path_override?: string;
  aot_auto_train?: boolean;
  hue?: number | null;
  game_dir?: string | null;
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
