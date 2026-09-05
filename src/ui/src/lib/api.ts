import type {
  Account,
  AppConfig,
  AotStatus,
  ClientInfo,
  ClientPatch,
  CreateInstanceRequest,
  DryRunResult,
  HealthInfo,
  ImportResult,
  ImportSource,
  InstanceDetail,
  InstancePatch,
  InstanceSummary,
  JvmInfo,
  LaunchMode,
  LaunchReply,
  LaunchStat,
  LiveLaunch,
  LogChunk,
  ModEntry,
  ModPreset,
  ModPresetInfo,
  ModrinthProject,
  MsDeviceFlow,
  OptionsPair,
  OverwritePolicy,
  ServerEntry,
  ServerStatus,
  SkinInfo,
  SkinLibraryEntry,
  SkinVariant,
  Theme,
  VersionInfo,
  VersionManifest,
} from './types';
import { t } from './i18n.svelte';
import { API_BASE } from './sse';
export { API_BASE } from './sse';


export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

interface ErrorEnvelope {
  error?: { code?: string; message?: string };
}

const enc = encodeURIComponent;
export const TIMEOUT_MS: { default: number; launch: number } = { default: 20_000, launch: 300_000 } as const;

async function request<T>(path: string, method: string, body?: unknown, timeoutMs = TIMEOUT_MS.default): Promise<T> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  let res: Response;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      method,
      signal: ctrl.signal,
      headers: {
        // H1 mitigation: the engine rejects non-GET requests without this
        // custom header (browsers can't set it cross-origin without a CORS
        // preflight, which the engine's origin allowlist rejects).
        'x-espectral-client': '1',
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      throw new ApiError(0, 'timeout', (t as (k: string, p: unknown) => string)('api.timeout', { ms: timeoutMs }));
    }
    throw new ApiError(0, 'network', err instanceof Error ? err.message : (t as (k: string) => string)('api.network'));
  } finally {
    clearTimeout(timer);
  }

  const text = await res.text();
  let data: unknown = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
  }

  if (!res.ok) {
    const envelope = (data as ErrorEnvelope | null)?.error;
    throw new ApiError(
      res.status,
      envelope?.code ?? 'http',
      envelope?.message ?? `HTTP ${res.status} ${res.statusText}`,
    );
  }
  return data as T;
}

const get = <T>(path: string): Promise<T> => request<T>(path, 'GET');
const post = <T>(path: string, body?: unknown): Promise<T> => request<T>(path, 'POST', body);
const put = <T>(path: string, body?: unknown): Promise<T> => request<T>(path, 'PUT', body);
const patch = <T>(path: string, body?: unknown): Promise<T> => request<T>(path, 'PATCH', body);
const del = <T>(path: string): Promise<T> => request<T>(path, 'DELETE');

// Route table — method + path co-located, grep-friendly keys keep `/api/...` searchable.
const R = {
  health: ['GET', '/api/health'] as const,
  servers: ['GET', '/api/servers'] as const,
  versions: ['GET', '/api/versions'] as const,
  versionResolve: (id: string) => ['GET', `/api/versions/${enc(id)}/resolve`] as const,
  jvm: ['GET', '/api/jvm'] as const,
  accounts: ['GET', '/api/accounts'] as const,
  accountsCreate: ['POST', '/api/accounts'] as const,
  accountsActive: ['POST', '/api/accounts/active'] as const,
  msDeviceCode: ['POST', '/api/accounts/microsoft/device-code'] as const,
  msClientId: ['GET', '/api/accounts/microsoft/client-id'] as const,
  msClientIdPut: ['PUT', '/api/accounts/microsoft/client-id'] as const,
  msPoll: ['POST', '/api/accounts/microsoft/poll'] as const,
  msLogout: ['POST', '/api/accounts/microsoft/logout'] as const,
  openFolder: ['POST', '/api/open-folder'] as const,
  config: ['GET', '/api/config'] as const,
  configPatch: ['PATCH', '/api/config'] as const,
  theme: ['GET', '/api/theme'] as const,
  themePut: ['PUT', '/api/theme'] as const,
  instances: ['GET', '/api/instances'] as const,
  instancesPost: ['POST', '/api/instances'] as const,
  getInstance: (n: string) => ['GET', `/api/instances/${enc(n)}`] as const,
  deleteInstance: (n: string) => ['DELETE', `/api/instances/${enc(n)}`] as const,
  patchInstance: (n: string) => ['PATCH', `/api/instances/${enc(n)}`] as const,
  importSources: ['GET', '/api/import/sources'] as const,
  importProfile: (n: string) => ['POST', `/api/instances/${enc(n)}/import`] as const,
  importMrpack: ['POST', '/api/instances/import-mrpack'] as const,
  instanceIcon: (n: string) => ['POST', `/api/instances/${enc(n)}/icon`] as const,
  pickFile: ['POST', '/api/pick-file'] as const,
  pickFolder: ['POST', '/api/pick-folder'] as const,
  accountAvatar: (u: string) => ['POST', `/api/accounts/${enc(u)}/avatar`] as const,
  accountSkin: (u: string) => ['GET', `/api/accounts/${enc(u)}/skin`] as const,
  accountSkinPng: (u: string) => ['GET', `/api/accounts/${enc(u)}/skin.png`] as const,
  accountSkinUpload: (u: string) => ['POST', `/api/accounts/${enc(u)}/skin`] as const,
  accountSkinReset: (u: string) => ['DELETE', `/api/accounts/${enc(u)}/skin`] as const,
  librarySkins: ['GET', '/api/skins'] as const,
  librarySkin: (id: string) => ['PATCH', `/api/skins/${enc(id)}`] as const,
  librarySkinApply: (id: string) => ['POST', `/api/skins/${enc(id)}/apply`] as const,
  libraryImportVanilla: ['POST', '/api/skins/import-vanilla'] as const,
  accountAvatarColor: (u: string) => ['POST', `/api/accounts/${enc(u)}/avatar-color`] as const,
  accountPatch: (u: string) => ['PATCH', `/api/accounts/${enc(u)}`] as const,
  mods: (n: string) => ['GET', `/api/instances/${enc(n)}/mods`] as const,
  modsDir: (n: string) => ['GET', `/api/instances/${enc(n)}/mods-dir`] as const,
  modToggle: (n: string, f: string, on: boolean) => ['POST', `/api/instances/${enc(n)}/mods/${enc(f)}/${on ? 'enable' : 'disable'}`] as const,
  modsInstall: (n: string) => ['POST', `/api/instances/${enc(n)}/mods/install`] as const,
  modsPreset: (n: string) => ['GET', `/api/instances/${enc(n)}/mods/preset`] as const,
  modrinthSearch: ['GET', '/api/modrinth/search'] as const,
  modrinthInstall: (n: string) => ['POST', `/api/instances/${enc(n)}/mods/install-modrinth`] as const,
  instanceServers: (n: string) => ['GET', `/api/instances/${enc(n)}/servers`] as const,
  instanceServersPut: (n: string) => ['PUT', `/api/instances/${enc(n)}/servers`] as const,
  instanceOptions: (n: string) => ['GET', `/api/instances/${enc(n)}/options`] as const,
  instanceOptionsImport: (n: string) => ['POST', `/api/instances/${enc(n)}/options/import`] as const,
  aot: (n: string) => ['GET', `/api/instances/${enc(n)}/aot`] as const,
  train: (n: string) => ['POST', `/api/instances/${enc(n)}/train`] as const,
  client: (n: string) => ['GET', `/api/instances/${enc(n)}/client`] as const,
  clientPatch: (n: string) => ['PATCH', `/api/instances/${enc(n)}/client`] as const,
  launch: (n: string) => ['POST', `/api/instances/${enc(n)}/launch`] as const,
  stopInstance: (n: string) => ['POST', `/api/instances/${enc(n)}/stop`] as const,
  launchLog: (k: string, c: number) => ['GET', `/api/launch/${enc(k)}/log?cursor=${c}`] as const,
  launchStats: (l: number) => ['GET', `/api/stats/launches?limit=${l}`] as const,
  shutdown: ['POST', '/api/shutdown'] as const,
  launches: ['GET', '/api/launches'] as const,
} as const;

/* ---------- one wrapper per endpoint ---------- */

export const getHealth = (): Promise<HealthInfo> => get(R.health[1]);
export const getServers = async (): Promise<ServerStatus[]> => (await get<{ servers: ServerStatus[] }>(R.servers[1])).servers;
export const getVersions = (): Promise<VersionManifest> => get(R.versions[1]);
export const resolveVersion = (id: string): Promise<VersionInfo> => get(R.versionResolve(id)[1]);
export const getJvm = (): Promise<JvmInfo> => get(R.jvm[1]);
export const getAccounts = (): Promise<Account[]> => get(R.accounts[1]);
export const createAccount = (username: string): Promise<Account> => post(R.accountsCreate[1], { username });
export const setActiveAccount = (username: string): Promise<{ active: string }> => post(R.accountsActive[1], { username });
export const startMsLogin = (): Promise<MsDeviceFlow> => post(R.msDeviceCode[1]);
export const getMsClientId = (): Promise<{ client_id: string }> => get(R.msClientId[1]);
export const setMsClientId = (clientId: string): Promise<{ client_id: string }> => put(R.msClientIdPut[1], { client_id: clientId });

// retry_after (seconds) is present when the engine wants us to back off and
// keep polling (slow_down / transient) instead of aborting the sign-in (M8).
export const pollMsLogin = (flowId: string): Promise<Account | { pending: true; retry_after?: number }> =>
  post(R.msPoll[1], { flow_id: flowId });
export const logoutMsAccount = (username: string): Promise<{ deleted: string }> => post(R.msLogout[1], { username });
export const openFolder = (path: string): Promise<{ ok: true }> => post(R.openFolder[1], { path });
export const getConfig = (): Promise<AppConfig> => get(R.config[1]);
export const patchConfig = (configPatch: Partial<AppConfig>): Promise<AppConfig> => patch(R.configPatch[1], { patch: configPatch });
export const getTheme = (): Promise<Theme> => get(R.theme[1]);
export const setTheme = (theme: Theme): Promise<{ theme: Theme }> => put(R.themePut[1], { theme });
export const listInstances = (): Promise<InstanceSummary[]> => get(R.instances[1]);
export const createInstance = (req: CreateInstanceRequest): Promise<InstanceSummary> => post(R.instancesPost[1], req);
export const getInstance = (name: string): Promise<InstanceDetail> => get(R.getInstance(name)[1]);
export const deleteInstance = (name: string): Promise<{ deleted: string }> => del(R.deleteInstance(name)[1]);
export const patchInstance = (name: string, instancePatch: InstancePatch): Promise<InstanceSummary> =>
  patch(R.patchInstance(name)[1], instancePatch);
export const getImportSources = (): Promise<ImportSource[]> => get(R.importSources[1]);
export const importProfile = (name: string, source_id: string, overwrite_policy: OverwritePolicy): Promise<ImportResult> =>
  post(R.importProfile(name)[1], { source_id, overwrite_policy });
export const pickMrpackFile = (): Promise<{ path: string | null }> =>
  post(R.pickFile[1], {
    title: 'Elegir modpack (.mrpack)',
    filter: 'Modrinth modpack (*.mrpack)|*.mrpack|Todos los archivos (*.*)|*.*',
  });
export const importMrpack = (path: string, memoryMb?: number): Promise<{ summary: InstanceSummary; already_exists?: boolean }> =>
  post(R.importMrpack[1], { path, memory_mb: memoryMb });
export const pickFolder = (title: string): Promise<{ path: string | null }> =>
  post(R.pickFolder[1], { title });
export const iconUrl = (name: string): string =>
  `${API_BASE}/api/instances/${enc(name)}/icon`;
export const uploadInstanceIcon = (name: string, image_base64: string): Promise<{ ok: true }> =>
  post(R.instanceIcon(name)[1], { image_base64 });
export const removeInstanceIcon = (name: string): Promise<{ removed: true }> =>
  del(R.instanceIcon(name)[1]);
export const avatarUrl = (username: string): string =>
  `${API_BASE}/api/accounts/${enc(username)}/avatar`;
export const uploadAccountAvatar = (username: string, image_base64: string): Promise<{ ok: true }> =>
  post(R.accountAvatar(username)[1], { image_base64 });
export const removeAccountAvatar = (username: string): Promise<{ removed: true }> =>
  del(R.accountAvatar(username)[1]);
export const getAccountSkin = (username: string): Promise<SkinInfo> =>
  get(R.accountSkin(username)[1]);
export const skinPngUrl = (username: string): string =>
  `${API_BASE}/api/accounts/${enc(username)}/skin.png`;
export const uploadAccountSkin = (
  username: string,
  image_base64: string,
  variant: SkinVariant,
): Promise<{ ok: true; variant: SkinVariant }> =>
  post(R.accountSkinUpload(username)[1], { image_base64, variant });
export const resetAccountSkin = (username: string): Promise<{ reset: true }> =>
  del(R.accountSkinReset(username)[1]);
export const listLibrarySkins = async (): Promise<SkinLibraryEntry[]> =>
  (await get<{ skins: SkinLibraryEntry[] }>(R.librarySkins[1])).skins;
export const librarySkinPngUrl = (id: string): string =>
  `${API_BASE}/api/skins/${enc(id)}.png`;
export const saveLibrarySkin = (
  name: string,
  image_base64: string,
  variant: SkinVariant,
): Promise<SkinLibraryEntry> =>
  post(R.librarySkins[1], { name, image_base64, variant });
export const patchLibrarySkin = (
  id: string,
  skinPatch: { name?: string; variant?: SkinVariant },
): Promise<SkinLibraryEntry> =>
  patch(R.librarySkin(id)[1], skinPatch);
export const deleteLibrarySkin = (id: string): Promise<{ removed: true }> =>
  del(R.librarySkin(id)[1]);
export const applyLibrarySkin = (
  id: string,
  username: string,
): Promise<{ ok: true; variant: SkinVariant }> =>
  post(R.librarySkinApply(id)[1], { username });
export const importVanillaSkins = (): Promise<{ imported: number; skipped: number; total: number }> =>
  post(R.libraryImportVanilla[1]);

/**
 * Persist an account accent color. Tries the PATCH account shape first and
 * falls back to the dedicated avatar-color POST when the engine answers
 * 404/405 (the engine half owns the route; either shape is accepted).
 */
export const setAccountAvatarColor = async (
  username: string,
  avatar_color: number | null,
): Promise<void> => {
  try {
    await patch(R.accountPatch(username)[1], { avatar_color });
  } catch (e) {
    const code = e instanceof ApiError ? e.code : '';
    const msg = e instanceof Error ? e.message : String(e);
    if (code === 'NOT_FOUND' || code === 'METHOD_NOT_ALLOWED' || /404|405/.test(msg)) {
      await post(R.accountAvatarColor(username)[1], { avatar_color });
      return;
    }
    throw e;
  }
};
/**
 * Thrown when neither createImageBitmap nor <img> can decode the file.
 * Carries the file's type/extension label so callers can render a localized
 * message naming the format (e.g. AVIF) instead of a bare generic failure.
 */
export class UnsupportedImageError extends Error {
  readonly label: string;
  readonly fileName: string;

  constructor(file: File) {
    const type = (file.type || '').trim();
    const ext = /\.([A-Za-z0-9]{1,10})$/.exec(file.name.trim())?.[1]?.toLowerCase() ?? '';
    const label = type || (ext ? `.${ext}` : 'unknown format');
    super(`unsupported image (${label}): ${file.name} — use PNG, JPEG or WebP`);
    this.name = 'UnsupportedImageError';
    this.label = label;
    this.fileName = file.name;
  }
}

function drawToPngDataUrl(source: CanvasImageSource, width: number, height: number, maxDim: number): string {
  const scale = Math.min(1, maxDim / Math.max(width, height));
  const w = Math.max(1, Math.round(width * scale));
  const h = Math.max(1, Math.round(height * scale));
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('canvas 2d unavailable');
  ctx.drawImage(source, 0, 0, w, h);
  return canvas.toDataURL('image/png');
}

function loadViaObjectUrlImg(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const objUrl = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(objUrl);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(objUrl);
      reject(new UnsupportedImageError(file));
    };
    img.src = objUrl;
  });
}

/**
 * Decode any user-picked image file and return a PNG data URL (downscaled to
 * maxDim). Tries createImageBitmap first (broader codec support than
 * WebView2 <img>: AVIF, some WebP, …) with the object-URL <img> path as
 * fallback. The engine only accepts PNG (decodePngImage sniffs PNG magic),
 * so the canvas re-encode keeps that contract whatever the input format was.
 * Final failure throws UnsupportedImageError naming the file's type/extension.
 */
export async function fileToPngDataUrl(file: File, maxDim = 256): Promise<string> {
  if (typeof createImageBitmap === 'function') {
    let bmp: ImageBitmap | null = null;
    try {
      bmp = await createImageBitmap(file);
    } catch {
      bmp = null;
    }
    if (bmp) {
      try {
        return drawToPngDataUrl(bmp, bmp.width, bmp.height, maxDim);
      } finally {
        bmp.close();
      }
    }
  }
  try {
    const img = await loadViaObjectUrlImg(file);
    return drawToPngDataUrl(img, img.naturalWidth || img.width, img.naturalHeight || img.height, maxDim);
  } catch (err) {
    if (err instanceof UnsupportedImageError) throw err;
    if (err instanceof Error && err.message === 'canvas 2d unavailable') throw err;
    throw new UnsupportedImageError(file);
  }
}
export const listMods = (name: string): Promise<ModEntry[]> => get(R.mods(name)[1]);
export const setModEnabled = (name: string, filename: string, enabled: boolean): Promise<{ filename: string; enabled: boolean }> =>
  post(R.modToggle(name, filename, enabled)[1]);
export const installMods = (name: string, preset: ModPreset = 'performance'): Promise<{ queued: true }> =>
  post(R.modsInstall(name)[1], { preset });
export const getModsPresetInfo = (name: string): Promise<ModPresetInfo> => get(R.modsPreset(name)[1]);
export const getModsDir = (name: string): Promise<{ path: string }> => get(R.modsDir(name)[1]);
export const searchModrinth = (
  query: string,
  version: string,
  loader?: string,
  instance?: string,
): Promise<{ results: ModrinthProject[] }> => {
  let qs = `q=${encodeURIComponent(query)}&version=${encodeURIComponent(version)}`;
  if (loader) qs += `&loader=${encodeURIComponent(loader)}`;
  if (instance) qs += `&instance=${encodeURIComponent(instance)}`;
  return get(`${R.modrinthSearch[1]}?${qs}`);
};
export const installModrinthMod = (name: string, projectId: string): Promise<{ queued: true }> =>
  post(R.modrinthInstall(name)[1], { project_id: projectId });
export const getInstanceServers = (name: string): Promise<ServerEntry[]> => get(R.instanceServers(name)[1]);
export const putInstanceServers = (name: string, servers: ServerEntry[]): Promise<{ count: number }> =>
  put(R.instanceServersPut(name)[1], { servers });
export const getInstanceOptions = (name: string): Promise<OptionsPair[]> => get(R.instanceOptions(name)[1]);
export const importInstanceOptions = (name: string, source: string): Promise<{ copied: boolean; count: number; options: OptionsPair[] }> =>
  post(R.instanceOptionsImport(name)[1], { source });
export const getAotStatus = (name: string): Promise<AotStatus> => get(R.aot(name)[1]);
export const trainAot = (name: string): Promise<{ key: string; deferred?: boolean }> => post(R.train(name)[1]);
export const getInstanceClient = (name: string): Promise<ClientInfo> => get(R.client(name)[1]);
export const patchInstanceClient = (name: string, clientPatch: ClientPatch): Promise<ClientInfo> =>
  patch(R.clientPatch(name)[1], clientPatch);
export const launchInstance = (
  name: string,
  options: { mode: LaunchMode; dry_run: boolean; account?: string },
): Promise<LaunchReply | DryRunResult> =>
  request<LaunchReply | DryRunResult>(R.launch(name)[1], 'POST', options, TIMEOUT_MS.launch);
export const stopInstance = (name: string): Promise<{ ok: true; instance: string }> =>
  post(R.stopInstance(name)[1]);
export const shutdownEngine = (): Promise<{ ok: true }> => post(R.shutdown[1]);
export const getLaunchLog = (key: string, cursor = 0): Promise<LogChunk> => get(R.launchLog(key, cursor)[1]);
export const getLaunchStats = (limit = 1): Promise<{ launches: LaunchStat[] }> => get(R.launchStats(limit)[1]);
export const getLaunches = async (): Promise<LiveLaunch[]> => (await get<{ launches: LiveLaunch[] }>(R.launches[1])).launches;

// SSE moved to ./sse — re-export to preserve `from './api'` import path.
export { subscribeEvents, SSE_EVENT_NAMES } from './sse';
export type { SseEvent } from './sse';
