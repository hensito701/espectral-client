/**
 * Miscellaneous routes: health, theme, config, jvm, accounts.
 * JVM discovery is delegated to jvm.mjs (B4) via dynamic import — absent until
 * the launch slice lands, in which case GET /api/jvm returns 503 UNAVAILABLE.
 */
import * as config from '../config.mjs';
import * as accounts from '../accounts.mjs';
import * as instances from '../instances.mjs';
import { httpError } from '../error.mjs';
import { VERSION } from '../server.mjs';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { dataDir } from '../config.mjs';

const THEMES = ['dark', 'light', 'system'];

/** Active device-login flows: flow_id -> { device_code, interval } (memory only). */
const msaFlows = new Map();

// M8: transient poll failures (network blips, engine restart) must not kill a
// sign-in the user is actively completing. A flow tolerates this many
// consecutive transient failures before the error is surfaced; each tolerated
// failure waits a little longer (index-aligned with the streak).
const TRANSIENT_BACKOFF_SECONDS = [5, 10, 15];
const MAX_TRANSIENT_STREAK = TRANSIENT_BACKOFF_SECONDS.length;

export async function register(app) {
  app.get('/api/health', async () => ({ ok: true, version: VERSION }));

  // POST /api/shutdown -> { ok: true } — the in-app updater calls this before
  // running the NSIS installer. The engine runs on the bundled node.exe, and
  // an engine that outlives the app keeps node.exe locked, failing the update
  // with "Error opening file for writing". Exits on a short delay so the
  // response flushes first. Running games are independent OS processes and
  // are NOT touched (same as an engine restart).
  app.post('/api/shutdown', async () => {
    setTimeout(() => process.exit(0), 250);
    return { ok: true };
  });

  // Opens a folder in the OS file manager. The path must resolve inside the
  // engine data dir (instances live under <dataDir>/instances) — an arbitrary
  // absolute path would let a compromised UI open any directory.
  app.post('/api/open-folder', async (req, res, params, body) => {
    const raw = body && typeof body.path === 'string' ? body.path.trim() : '';
    if (!raw) throw httpError(400, 'BAD_PATH', 'path is required');
    const root = path.resolve(dataDir());
    const target = path.resolve(path.isAbsolute(raw) ? raw : path.join(root, raw));
    if (target !== root && !target.startsWith(root + path.sep)) {
      throw httpError(400, 'BAD_PATH', 'path must be inside the engine data directory');
    }
    if (!existsSync(target)) throw httpError(404, 'BAD_PATH', `path does not exist: ${raw}`);
    try {
      const open = { win32: 'explorer', darwin: 'open', linux: 'xdg-open' }[process.platform] ?? 'explorer';
      spawn(open, [target], { detached: true, stdio: 'ignore' }).unref();
    } catch (e) {
      throw httpError(500, 'OPEN_FAILED', `could not open folder: ${e.message}`);
    }
    return { ok: true };
  });

  // POST /api/pick-file -> { path } — native Windows file picker (PowerShell
  // STA + WinForms OpenFileDialog). path === null means the user canceled.
  app.post('/api/pick-file', async (req, res, params, body) => {
    const title = body && typeof body.title === 'string' ? body.title : 'Choose file';
    const filter = body && typeof body.filter === 'string' ? body.filter : 'All files (*.*)|*.*';
    const esc = (s) => s.replace(/'/g, "''");
    const script = [
      'Add-Type -AssemblyName System.Windows.Forms',
      '$d = New-Object System.Windows.Forms.OpenFileDialog',
      `$d.Title = '${esc(title)}'`,
      `$d.Filter = '${esc(filter)}'`,
      '$d.CheckFileExists = $true',
      '$r = $d.ShowDialog()',
      'if ($r -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.FileName }',
    ].join('\n');
    const path = await pickFile(script);
    return { path };
  });

  // POST /api/pick-folder -> { path } — native Windows folder picker (modern
  // Vista+ Explorer-style IFileOpenDialog with FOS_PICKFOLDERS (0x20) |
  // FOS_FORCEFILESYSTEM (0x40) via PowerShell STA + inline C# COM interop).
  // The coclass->interface QI cast must happen inside C# (PowerShell cannot
  // QueryInterface-cast a raw __ComObject), so the dialog runs in a small
  // FolderDialog.Show helper; cancel (non-S_OK, no stdout) -> null.
  // Mirrors POST /api/pick-file above.
  app.post('/api/pick-folder', async (req, res, params, body) => {
    const title = body && typeof body.title === 'string' ? body.title : 'Choose folder';
    const esc = (s) => s.replace(/'/g, "''");
    const script = [
      "Add-Type -TypeDefinition @'",
      'using System;',
      'using System.Runtime.InteropServices;',
      'namespace EspectralPicker {',
      '[ComImport]',
      '[Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE")]',
      '[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]',
      'public interface IShellItem {',
      '[PreserveSig] int BindToHandler(IntPtr pbc, ref Guid bhid, ref Guid riid, out IntPtr ppv);',
      '[PreserveSig] int GetParent(out IShellItem ppsi);',
      '[PreserveSig] int GetDisplayName(uint sigdnName, [MarshalAs(UnmanagedType.LPWStr)] out string ppszName);',
      '[PreserveSig] int GetAttributes(uint sfgaoMask, out uint psfgaoAttribs);',
      '[PreserveSig] int Compare(IShellItem psi, uint hint, out int piOrder);',
      '[PreserveSig] int GetPropertyStore(int flags, ref Guid riid, out IntPtr ppv);',
      '}',
      '[ComImport]',
      '[Guid("42F85136-DB7E-439C-85F1-E4075D135FC8")]',
      '[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]',
      'public interface IFileOpenDialog {',
      '[PreserveSig] int Show(IntPtr parent);',
      '[PreserveSig] int SetFileTypes(uint cFileTypes, IntPtr rgFilterSpec);',
      '[PreserveSig] int SetFileTypeIndex(uint iFileType);',
      '[PreserveSig] int GetFileTypeIndex(out uint piFileType);',
      '[PreserveSig] int Advise(IntPtr pfde, out uint pdwCookie);',
      '[PreserveSig] int Unadvise(uint dwCookie);',
      '[PreserveSig] int SetOptions(uint fos);',
      '[PreserveSig] int GetOptions(out uint pfos);',
      '[PreserveSig] int SetDefaultFolder(IShellItem psi);',
      '[PreserveSig] int SetFolder(IShellItem psi);',
      '[PreserveSig] int GetFolder(out IShellItem ppsi);',
      '[PreserveSig] int GetCurrentSelection(out IShellItem ppsi);',
      '[PreserveSig] int SetFileName([MarshalAs(UnmanagedType.LPWStr)] string pszName);',
      '[PreserveSig] int GetFileName([MarshalAs(UnmanagedType.LPWStr)] out string pszName);',
      '[PreserveSig] int SetTitle([MarshalAs(UnmanagedType.LPWStr)] string pszTitle);',
      '[PreserveSig] int SetOkButtonLabel([MarshalAs(UnmanagedType.LPWStr)] string pszText);',
      '[PreserveSig] int SetFileNameLabel([MarshalAs(UnmanagedType.LPWStr)] string pszLabel);',
      '[PreserveSig] int GetResult(out IShellItem ppsi);',
      '[PreserveSig] int AddPlace(IShellItem psi, int alignment);',
      '[PreserveSig] int SetDefaultExtension([MarshalAs(UnmanagedType.LPWStr)] string pszDefaultExtension);',
      '[PreserveSig] int Close(int hr);',
      '[PreserveSig] int SetClientGuid(ref Guid guid);',
      '[PreserveSig] int ClearClientData();',
      '[PreserveSig] int SetFilter(IntPtr pFilter);',
      '}',
      '[ComImport]',
      '[Guid("DC1C5A9C-E88A-4DDE-A5A1-60F82A20AEF7")]',
      'public class FileOpenDialogRCW {',
      '}',
      'public static class FolderDialog {',
      'public static string Show(string title) {',
      'IFileOpenDialog dlg = (IFileOpenDialog)new FileOpenDialogRCW();',
      'try {',
      'uint opts;',
      'dlg.GetOptions(out opts);',
      'dlg.SetOptions(opts | 0x20 | 0x40);',
      'if (!string.IsNullOrEmpty(title)) dlg.SetTitle(title);',
      'if (dlg.Show(IntPtr.Zero) != 0) return null;',
      'IShellItem item;',
      'if (dlg.GetResult(out item) != 0 || item == null) return null;',
      'string path;',
      'if (item.GetDisplayName(0x80058000, out path) != 0) return null;',
      'return path;',
      '} finally {',
      'Marshal.ReleaseComObject(dlg);',
      '}',
      '}',
      '}',
      '}',
      "'@",
      `$r = [EspectralPicker.FolderDialog]::Show('${esc(title)}')`,
      'if ($r -ne $null -and $r -ne \'\') { Write-Output $r }',
    ].join('\n');
    const folderPath = await pickFile(script);
    return { path: folderPath };
  });


  app.get('/api/theme', async () => config.loadConfig().theme ?? 'dark');

  app.put('/api/theme', async (req, res, params, body) => {
    const theme = body && body.theme;
    if (!THEMES.includes(theme)) {
      throw httpError(400, 'BAD_THEME', 'theme must be one of dark|light|system');
    }
    config.saveConfig({ theme });
    return { theme };
  });

  app.get('/api/config', async () => config.appConfig());

  app.patch('/api/config', async (req, res, params, body) => {
    // The client wraps the patch in an envelope (`patchConfig` sends
    // `{ patch: configPatch }`). Accept both the envelope and a bare patch so
    // the Settings page actually saves (this was a silent no-op: sanitizing
    // the envelope object produced an empty patch -> saveConfig({})).
    const raw = body && typeof body === 'object' ? body : {};
    const patchInput = raw && typeof raw.patch === 'object' && raw.patch !== null ? raw.patch : raw;
    const patch = sanitizeConfigPatch(patchInput);
    config.saveConfig(patch);
    return config.appConfig();
  });

  app.get('/api/jvm', async () => {
    try {
      const jvm = await import('../jvm.mjs');
      if (typeof jvm.getJvmInfo === 'function') {
        const info = await jvm.getJvmInfo();
        return { ...info, supported_majors: [25, 21, 17, 16, 8] };
      }
    } catch {
      /* not loaded yet */
    }
    throw httpError(503, 'UNAVAILABLE', 'JVM discovery not loaded (launch slice pending)');
  });

  app.get('/api/accounts', async () =>
    // Sanitize at the API boundary: never hand the UI the durable refresh
    // token or any in-memory access token / XBL-XSTS chain material (H2).
    // The launch path reads the full object via getActiveAccount() internally;
    // the UI only needs username/uuid/token_kind/last_used/created_at (+ the
    // lunar flag for its badge).
    accounts.listAccounts().map(accounts.publicAccount)
  );

  app.post('/api/accounts', async (req, res, params, body) => {
    return accounts.createAccount(body && typeof body.username === 'string' ? body.username : undefined);
  });

  app.post('/api/accounts/active', async (req, res, params, body) => {
    return accounts.setActiveAccount(body && typeof body.username === 'string' ? body.username : undefined);
  });

  // --- Instance icons (<instanceDir>/icon.png) ---

  // POST /api/instances/:name/icon { image_base64 } -> { ok, has_icon }.
  // image_base64 is a data: URL or raw base64 PNG (<= 500KB decoded).
  app.post('/api/instances/:name/icon', async (req, res, params, body) => {
    const buf = decodePngImage(body && body.image_base64);
    return instances.writeInstanceIcon(params.name, buf);
  });

  app.delete('/api/instances/:name/icon', async (req, res, params) => {
    return instances.removeInstanceIcon(params.name);
  });

  // GET serves raw image/png bytes (404 when absent).
  app.get('/api/instances/:name/icon', async (req, res, params) => {
    const buf = await instances.readInstanceIcon(params.name);
    res.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': buf.length });
    res.end(buf);
    return undefined;
  });

  // --- Account avatars (<dataDir>/avatars/<uuid>.png) + avatar color ---

  // POST /api/accounts/:username/avatar { image_base64 } -> { ok, has_avatar }.
  app.post('/api/accounts/:username/avatar', async (req, res, params, body) => {
    const buf = decodePngImage(body && body.image_base64);
    return accounts.writeAccountAvatar(params.username, buf);
  });

  app.delete('/api/accounts/:username/avatar', async (req, res, params) => {
    return accounts.removeAccountAvatar(params.username);
  });

  // GET serves raw image/png bytes (404 when absent).
  app.get('/api/accounts/:username/avatar', async (req, res, params) => {
    const buf = await accounts.readAccountAvatar(params.username);
    res.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': buf.length });
    res.end(buf);
    return undefined;
  });

  // POST /api/accounts/:username/avatar-color { avatar_color } -> publicAccount.
  app.post('/api/accounts/:username/avatar-color', async (req, res, params, body) => {
    return accounts.setAvatarColor(params.username, body ? body.avatar_color : undefined);
  });

  // --- Microsoft (MSA) login ---

  // GET /api/accounts/microsoft/client-id -> { client_id } (masked if set)
  app.get('/api/accounts/microsoft/client-id', async () => {
    const cfg = config.loadConfig();
    const id = typeof cfg.msa_client_id === 'string' && cfg.msa_client_id.length > 0
      ? cfg.msa_client_id
      : (await import('../msauth.mjs')).clientId();
    return { client_id: id };
  });

  // PUT /api/accounts/microsoft/client-id { client_id } -> persists to config.json
  app.put('/api/accounts/microsoft/client-id', async (req, res, params, body) => {
    const id = body && typeof body.client_id === 'string' ? body.client_id.trim() : '';
    if (!/^[0-9a-fA-F-]{10,}$/.test(id)) {
      throw httpError(400, 'BAD_CLIENT_ID', 'client_id must be a valid Azure application ID');
    }
    config.saveConfig({ msa_client_id: id });
    return { client_id: id };
  });
  // POST /api/accounts/microsoft/device-code -> { flow_id, user_code, verification_uri, ... }
  app.post('/api/accounts/microsoft/device-code', async () => {
    const msa = await import('../msauth.mjs');
    const flow = await msa.startDeviceLogin();
    msaFlows.set(flow.flow_id, { device_code: flow.device_code ?? flow.flow_id, interval: flow.interval });
    return flow;
  });

  // POST /api/accounts/microsoft/poll { flow_id } -> single token check. Returns
  // { pending: true } while the user hasn't signed in yet (UI re-polls every few
  // seconds), or the created Account once they have. Back-off-and-continue
  // outcomes keep { pending: true } and may add retry_after (seconds) so the UI
  // re-arms its timer accordingly (M8): slow_down carries Microsoft's suggested
  // wait, and transient engine/network failures retry with capped backoff.
  // Throws on definitive errors only (declined / expired / flow lost).
  app.post('/api/accounts/microsoft/poll', async (req, res, params, body) => {
    const flowId = body && typeof body.flow_id === 'string' ? body.flow_id : '';
    const flow = msaFlows.get(flowId);
    if (!flow) throw httpError(404, 'MSA_FLOW_NOT_FOUND', 'device login not found — start again');
    const msa = await import('../msauth.mjs');
    let tokens;
    try {
      tokens = await msa.checkDeviceToken(flow.device_code);
    } catch (err) {
      if (err?.code === 'MSA_PENDING' || err?.code === 'MSA_SLOW_DOWN') {
        flow.transient_streak = 0;
        return err.code === 'MSA_SLOW_DOWN'
          ? { pending: true, retry_after: err.retry_after ?? 5 }
          : { pending: true, ...(err.retry_after ? { retry_after: err.retry_after } : {}) };
      }
      if (err?.code === 'MSA_TRANSIENT' || err?.code === 'MSA_NETWORK') {
        flow.transient_streak = (flow.transient_streak ?? 0) + 1;
        if (flow.transient_streak <= MAX_TRANSIENT_STREAK) {
          return { pending: true, retry_after: TRANSIENT_BACKOFF_SECONDS[flow.transient_streak - 1] };
        }
        flow.transient_streak = 0; // budget spent — surface the failure
        throw err;
      }
      throw err;
    }
    flow.transient_streak = 0; // a real outcome clears the transient budget
    const mc = await msa.exchangeForMinecraft(tokens.access_token);
    const account = accounts.upsertMsaAccount({
      username: mc.username,
      uuid: mc.uuid,
      refreshToken: tokens.refresh_token,
      accessToken: mc.accessToken,
      expiresIn: mc.expiresIn,
    });
    msaFlows.delete(flowId);
    // Same sanitizer as GET /api/accounts — the poll response keeps identity
    // fields for the UI list/badge but never leaks microsoft.refresh_token.
    return accounts.publicAccount(account);
  });

  // POST /api/accounts/microsoft/logout { username } -> removes the account + refresh token
  app.post('/api/accounts/microsoft/logout', async (req, res, params, body) => {
    const username = body && typeof body.username === 'string' ? body.username : '';
    return accounts.deleteMsaAccount(username);
  });
}

/**
 * Run a PowerShell picker script (OpenFileDialog or IFileOpenDialog) and
 * resolve to the chosen path. Empty stdout (or a canceled dialog) -> null;
 * spawn errors and 60s timeouts surface as PICK_FAILED.
 */
function pickFile(script) {
  return new Promise((resolve, reject) => {
    const child = spawn('powershell.exe', ['-NoProfile', '-STA', '-Command', script], {
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    const timer = setTimeout(() => {
      child.kill();
      reject(httpError(500, 'PICK_FAILED', 'could not open the file picker'));
    }, 60_000);
    child.on('error', () => {
      clearTimeout(timer);
      reject(httpError(500, 'PICK_FAILED', 'could not open the file picker'));
    });
    child.on('close', () => {
      clearTimeout(timer);
      const line = stdout.replace(/\r?\n$/, '');
      resolve(line.length > 0 ? line : null);
    });
  });
}

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const MAX_IMAGE_BYTES = 500 * 1024;

/**
 * Decode an icon/avatar upload: a `data:image/png;base64,…` URL or raw
 * base64 PNG. Validates PNG magic bytes and the 500KB decoded cap.
 * Anything else is a 400 BAD_IMAGE. Exported for tests.
 */
export function decodePngImage(image_base64) {
  if (typeof image_base64 !== 'string' || image_base64.trim().length === 0) {
    throw httpError(400, 'BAD_IMAGE', 'image_base64 is required');
  }
  let b64 = image_base64.trim();
  if (b64.startsWith('data:')) {
    const m = b64.match(/^data:image\/png;base64,(.*)$/s);
    if (!m) throw httpError(400, 'BAD_IMAGE', 'image must be a PNG data URL or raw base64 PNG');
    b64 = m[1];
  }
  let buf;
  try {
    buf = Buffer.from(b64, 'base64');
  } catch {
    throw httpError(400, 'BAD_IMAGE', 'image_base64 is not valid base64');
  }
  if (buf.length > MAX_IMAGE_BYTES) {
    throw httpError(400, 'BAD_IMAGE', `image exceeds ${MAX_IMAGE_BYTES} bytes decoded`);
  }
  if (buf.length < 8 || !buf.subarray(0, 8).equals(PNG_MAGIC)) {
    throw httpError(400, 'BAD_IMAGE', 'image must be a PNG');
  }
  return buf;
}

/** Whitelist + validate the AppConfig patch fields (engine state is off-limits). */
function sanitizeConfigPatch(patch) {
  const out = {};
  if (patch.default_memory_mb !== undefined) {
    const mb = Number(patch.default_memory_mb);
    if (!Number.isInteger(mb) || mb < 512 || mb > 65536) {
      throw httpError(400, 'BAD_MEMORY', 'default_memory_mb must be an integer between 512 and 65536');
    }
    out.default_memory_mb = mb;
  }
  if (patch.download_concurrency !== undefined) {
    const c = Number(patch.download_concurrency);
    if (!Number.isInteger(c) || c < 1 || c > 16) {
      throw httpError(400, 'BAD_CONCURRENCY', 'download_concurrency must be an integer between 1 and 16');
    }
    out.download_concurrency = c;
  }
  if (patch.fast_boot !== undefined) out.fast_boot = !!patch.fast_boot;
  if (patch.aot_auto_train !== undefined) out.aot_auto_train = !!patch.aot_auto_train;
  if (patch.discord_enabled !== undefined) out.discord_enabled = !!patch.discord_enabled;
  if (patch.jdk_path_override !== undefined) {
    if (patch.jdk_path_override !== null && typeof patch.jdk_path_override !== 'string') {
      throw httpError(400, 'BAD_JDK', 'jdk_path_override must be a path string or null');
    }
    out.jdk_path_override = patch.jdk_path_override;
  }
  return out;
}
