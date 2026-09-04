/**
 * Launch engine — argv assembly, spawn + marker watch, dry-run CLI.
 * Argv order is fixed and covered by tests/launch.test.mjs.
 */
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawn } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { getJvmInfo, parseMajor } from './jvm.mjs';
import { javaForVersion, javaForMajorExact } from './runtimes.mjs';
import * as resolver from './resolver.mjs';
import { getInstance } from './instances.mjs';
import { offlineUuid, getActiveAccount } from './accounts.mjs';
import { cacheKey, cacheFilePath, isCacheStale } from './aot.mjs';
import { loadConfig } from './config.mjs';
import { parseOptionsTxt, setOptionKey, writeOptionsTxt } from './import.mjs';
import { seedClientConfig } from './client.mjs';

export const DEFAULT_MEMORY_MB = 3072;
export const MARKER_RE = /Sound engine started/;

/** Fixed JVM arg set — order is significant and asserted by tests. */
export const FIXED_JVM_ARGS = [
  '--add-opens', 'java.base/java.lang=ALL-UNNAMED',
  '--add-opens', 'java.base/java.util=ALL-UNNAMED',
  '--add-opens', 'java.base/java.io=ALL-UNNAMED',
  '--add-opens', 'java.base/java.nio=ALL-UNNAMED',
  '--add-opens', 'java.base/sun.nio.ch=ALL-UNNAMED',
  '--add-exports', 'java.base/sun.nio.ch=ALL-UNNAMED',
  '--enable-native-access=ALL-UNNAMED',
];

/** -Xms/-Xmx in whole gigabytes when possible: 3072 -> 3G, 4096 -> 4G, 2048 -> 2G. */
export function memoryArgs(memoryMb) {
  const mb = Number.isInteger(memoryMb) && memoryMb > 0 ? memoryMb : DEFAULT_MEMORY_MB;
  const g = mb / 1024;
  const label = Number.isInteger(g) && g >= 1 ? `${g}G` : `${mb}M`;
  return [`-Xms${label}`, `-Xmx${label}`];
}

/** Offline account fallback: OS username, MD5 offline UUID, fake token. */
export function defaultAccount() {
  let name = 'Player';
  try {
    name = String(os.userInfo().username || 'Player').replace(/[^A-Za-z0-9_]/g, '_').slice(0, 16) || 'Player';
  } catch {
    /* os.userInfo can throw when HOME is unset */
  }
  return { username: name, uuid: offlineUuid(name), accessToken: '0' };
}

/** Active offline account, else the OS-username fallback. */
export function pickAccount() {
  try {
    const active = getActiveAccount();
    if (active && active.username) {
      return { username: active.username, uuid: active.uuid, accessToken: '0' };
    }
  } catch {
    /* config unavailable — fall through */
  }
  return defaultAccount();
}

/**
 * Resolve a stored account record to a ready-to-launch shape.
 * Takes the stored config account record ({username, uuid, token_kind,
 * microsoft?: {...}}) and returns { username, uuid, accessToken, userType }.
 * MSA: tries ensureMinecraftToken; on failure/no-token warns and falls back
 * to offline mojang with accessToken '0'. Offline/other: immediate mojang.
 */
export async function resolveAccountFor(storedAccount, onWarn = null) {
  const warn = (msg) => {
    if (typeof onWarn === 'function') onWarn(msg);
    console.warn(`[launch] ${msg}`);
  };
  if (!storedAccount || !storedAccount.username) {
    return { ...defaultAccount(), userType: 'mojang' };
  }
  if (storedAccount.token_kind === 'msa') {
    try {
      const msa = await import('./msauth.mjs');
      const token = await msa.ensureMinecraftToken(storedAccount);
      if (token?.accessToken) {
        return {
          username: storedAccount.username,
          uuid: storedAccount.uuid,
          accessToken: token.accessToken,
          userType: 'msa',
        };
      }
      warn(
        `Microsoft session for '${storedAccount.username}' could not be established (no access token returned) — ` +
          'launching OFFLINE; online servers will reject this session.'
      );
    } catch (e) {
      warn(
        `Microsoft session for '${storedAccount.username}' failed (${e?.code ?? 'UNKNOWN'}: ${e?.message ?? e}) — ` +
          'launching OFFLINE; online servers will reject this session.'
      );
    }
    return { username: storedAccount.username, uuid: storedAccount.uuid, accessToken: '0', userType: 'mojang' };
  }
  return { username: storedAccount.username, uuid: storedAccount.uuid, accessToken: '0', userType: 'mojang' };
}

/**
 * Resolve the launch account to a ready-to-use shape:
 *  - offline accounts -> { username, uuid, accessToken: '0', userType: 'mojang' }
 *  - MSA accounts     -> { username, uuid, accessToken: <live MC token>,
 *                          userType: 'msa' } — the token is refreshed here if
 *                          stale/expired.
 *
 * A refresh failure still falls back to offline (so singleplayer/offline
 * servers remain playable), but it is now REPORTED through `onWarn` instead of
 * being swallowed. Silence here was the bug: a rate-limited (429) or expired
 * refresh launched the game with an offline token, which online-mode servers
 * reject with "Invalid session" while the launcher showed no error at all.
 */
export async function resolveAccount(targetOrWarn = null, onWarn = null) {
  let target = null;
  let warnFn = null;
  if (typeof targetOrWarn === 'function') {
    warnFn = targetOrWarn;
  } else {
    target = targetOrWarn;
    warnFn = onWarn;
  }
  try {
    const acc = target ?? getActiveAccount();
    if (acc && acc.username) {
      return await resolveAccountFor(acc, warnFn);
    }
  } catch {
    /* config unavailable — fall through */
  }
  return { ...defaultAccount(), userType: 'mojang' };
}

/**
 * Expand a Mojang jvm argument template: string entries pass through;
 * {rules, value} entries are kept when the os/arch rules allow, with the
 * value flattened when it is an array. Non-string entries without rules are
 * dropped (not a valid template shape).
 */
export function expandJvmArgs(args, { os = resolver.osName(), arch = process.arch } = {}) {
  const out = [];
  for (const entry of args ?? []) {
    if (typeof entry === 'string') {
      out.push(entry);
    } else if (
      entry &&
      Array.isArray(entry.rules) &&
      resolver.rulesAllow({ rules: entry.rules }, { os, arch })
    ) {
      if (Array.isArray(entry.value)) out.push(...entry.value);
      else if (typeof entry.value === 'string') out.push(entry.value);
    }
  }
  return out;
}

/** Same as expandJvmArgs but evaluates feature-rule entries (game template). */
export function expandGameArgs(args, features = {}) {
  const out = [];
  for (const entry of args ?? []) {
    if (typeof entry === 'string') {
      out.push(entry);
    } else if (
      entry &&
      Array.isArray(entry.rules) &&
      resolver.rulesAllow({ rules: entry.rules }, { features })
    ) {
      if (Array.isArray(entry.value)) out.push(...entry.value);
      else if (typeof entry.value === 'string') out.push(entry.value);
    }
  }
  return out;
}

/** Replace ${key} tokens from `map`; unknown keys expand to ''. */
export function substitutePlaceholders(arg, map) {
  return String(arg).replace(/\$\{([^{}]*)\}/g, (token, key) =>
    Object.prototype.hasOwnProperty.call(map, key) ? String(map[key]) : ''
  );
}

/**
 * Boot-time flags (measured on one Windows machine; see CHANGELOG).
 * -XX:-UsePerfData skips perf-file I/O.
 * -Xverify:none skips re-verification of classes (AOT already verified the
 * archived set at train time). The previous isolated -7% attribution to
 * -Xverify:none was confounded — it was measured jointly with -XX:-UsePerfData
 * in a single n=3 cell, so the isolated effect remains unmeasured.
 * On JDK 27+ -Xverify:none is a hard error (removed upstream), so it is gated
 * on javaMajor < 27.
 *
 * fast_boot (opt-in, Settings → Funciones) adds -XX:TieredStopAtLevel=1:
 * C1-only JIT cuts another ~13% off spawn→menu by not running C2 compile
 * threads during init. Tradeoff: lower peak JIT quality → slightly lower FPS
 * in long sessions. Measured stack: 12.07s -> 8.7–9.8s median with all mods.
 */
export function bootFlags(instance, javaMajor) {
  const flags = ['-XX:-UsePerfData'];
  const major = Number(javaMajor);
  if (!Number.isFinite(major) || major < 27) {
    flags.push('-Xverify:none');
  }
  if (instance?.fast_boot === true || (instance?.fast_boot === undefined && loadConfig().fast_boot === true)) {
    flags.push('-XX:TieredStopAtLevel=1');
  }
  return flags;
}

/**
 * Clamp requested memory to 60% of system RAM. Pure helper for testing;
 * buildArgv calls it with os.totalmem().
 */
export function effectiveMemoryMb(requestedMb, totalMb) {
  const req = Number(requestedMb);
  const total = Number(totalMb);
  if (!Number.isFinite(req) || req <= 0) return requestedMb;
  if (!Number.isFinite(total) || total <= 0) return requestedMb;
  const totalMbNum = total / (1024 * 1024);
  const cap = Math.floor(totalMbNum * 0.6);
  if (req > cap) return cap;
  return req;
}

/**
 * Pure argv assembly — no I/O, fully unit-testable.
 * `resolved` comes from resolveLaunch(); nothing here touches the filesystem.
 */
export function buildArgv(instance, resolved) {
  const javaMajor = resolved.java?.major ?? parseMajor(resolved.java?.build ?? resolved.java?.version ?? '');
  const rawAcc = resolved.account ?? pickAccount();
  const _tok = rawAcc.accessToken;
  const accessToken = _tok != null ? String(_tok) : '0';
  const safeToken = accessToken === 'undefined' || accessToken === '' ? '0' : accessToken;
  const userType = rawAcc.userType ?? (rawAcc.token_kind === 'msa' ? 'msa' : 'mojang');
  const acc = { username: rawAcc.username, uuid: rawAcc.uuid, accessToken: safeToken, userType, token_kind: rawAcc.token_kind };
  const requestedMb = resolved.memoryMb ?? instance.memory_mb ?? DEFAULT_MEMORY_MB;
  const effectiveMb = effectiveMemoryMb(requestedMb, os.totalmem());
  if (effectiveMb !== requestedMb) {
    resolved.warnings.push(`memory_mb ${requestedMb} clamped to ${effectiveMb} (60% of system RAM)`);
  }
  // One substitution map for every ${...} token in the merged loader templates
  // (neoforge jvm/game args) and the fallback program args.
  const subst = {
    auth_player_name: acc.username,
    auth_uuid: acc.uuid,
    auth_access_token: acc.accessToken,
    auth_session: acc.accessToken,
    user_type: acc.userType === 'msa' ? 'msa' : 'mojang',
    version_name: resolved.version.id,
    game_directory: resolved.gameDir,
    assets_root: resolved.assetsDir,
    assets_index_name: resolved.version.assetIndexId,
    natives_directory: resolved.nativesDir,
    library_directory: resolver.librariesDir(),
    classpath: resolved.classpath.join(path.delimiter),
    classpath_separator: path.delimiter,
    launcher_name: 'Espectral Client',
    launcher_version: '0.6.0',
    resolution_width: '854',
    resolution_height: '480',
    clientid: '0',
    auth_xuid: '0',
    version_type: 'release',
  };
  const jvm = [
    ...memoryArgs(effectiveMb),
    // --add-opens/--add-exports/--enable-native-access are Java 9+ (and the
    // latter 17+) flags; legacy tiers (JDK 8/16/17 for 1.15.2–1.20.4) reject
    // them at JVM startup.
    ...(javaMajor >= 21 ? FIXED_JVM_ARGS : []),
    '-Djava.library.path=' + resolved.nativesDir,
    ...(javaMajor >= 21 ? bootFlags(instance, javaMajor) : []),
  ];
  if (Array.isArray(resolved.loader?.jvmArgs) && resolved.loader.jvmArgs.length > 0) {
    // Loader templates are expanded (rules) + placeholder-substituted. The
    // merged jvm args carry -cp ${classpath} and -Djava.library.path=... — the
    // explicit lines above stay (last wins, same values).
    jvm.push(...expandJvmArgs(resolved.loader.jvmArgs).map((a) => substitutePlaceholders(a, subst)));
  }
  if (resolved.loader?.kind === 'neoforge') {
    // The vanilla client jar on the classpath would otherwise become an
    // automatic module (_1._21._1) whose packages split with the srg-jar
    // module ('minecraft'): bootstraplauncher skips module-izing files whose
    // NAME STARTS WITH an ignoreList entry, so the client jar must be exempt
    // too (the srg jar provides the classes).
    const il = jvm.findIndex((a) => a.startsWith('-DignoreList='));
    if (il >= 0 && typeof instance.version === 'string') {
      const clientName = path.basename(resolver.clientJarPath(instance.version));
      if (!jvm[il].includes(clientName)) jvm[il] = `${jvm[il]},${clientName}`;
    }
  }
  jvm.push('-cp', resolved.classpath.join(path.delimiter));

  // AOT (-XX:AOTCache*/JEP 483) exists only on JDK 24+/25 runtimes.
  if (resolved.mode === 'train') {
    if (javaMajor >= 25) {
      // Visible to the in-game mod (System.getProperty) so the branding mod can
      // neutralize MC's 10s ClientShutdownWatchdog during training: the AOT
      // config dump is 100+MB and ALWAYS exceeds the watchdog budget, killing
      // the JVM mid-write and discarding the cache (crash-2026-08-31_00.02.14).
      jvm.push('-Despectral.aot-training=true');
      jvm.push(`-XX:AOTCacheOutput=${resolved.aotCachePath}`);
    } else {
      resolved.warnings.push(
        `AOT training requires a JDK 25-tier runtime (current major ${javaMajor}); skipping -XX:AOTCacheOutput`
      );
    }
  } else if (resolved.mode === 'aot') {
    if (javaMajor >= 25) {
      if (resolved.aotCacheExists && !resolved.aotCacheStale) {
        jvm.push(`-XX:AOTCache=${resolved.aotCachePath}`);
        // RELATIVE proof log — never an absolute Windows path: -Xlog splits
        // options on ':' so file=C:\... would break; cwd = gameDir.
        jvm.push('-Xlog:aot=info:file=aot-%p.log');
      } else if (resolved.aotCacheExists) {
        // Passing a stale cache costs a doomed 0.5s mapping attempt and an
        // error-level JVM log on every boot; skip it and let the launch route
        // queue a retrain instead.
        resolved.warnings.push(
          `AOT cache ${resolved.aotCachePath} no longer matches the classpath on disk; retraining is required`
        );
      } else {
        resolved.warnings.push(`AOT cache ${resolved.aotCachePath} not found; running without -XX:AOTCache`);
      }
    } else {
      resolved.warnings.push(
        `AOT requires a JDK 25-tier runtime (current major ${javaMajor}); running without -XX:AOTCache`
      );
    }
  }

  const mainClass =
    resolved.loader?.mainClass ?? resolved.version.mainClass ?? 'net.minecraft.client.main.Main';
  const progArgs =
    Array.isArray(resolved.loader?.gameArgs) && resolved.loader.gameArgs.length > 0
      ? expandGameArgs(resolved.loader.gameArgs).map((a) => substitutePlaceholders(a, subst))
      : [
          '--username', acc.username,
          '--version', resolved.version.id,
          '--gameDir', resolved.gameDir,
          '--assetsDir', resolved.assetsDir,
          '--assetIndex', resolved.version.assetIndexId,
          '--uuid', acc.uuid,
          '--accessToken', acc.accessToken,
          '--userType', acc.userType === 'msa' ? 'msa' : 'mojang',
          '--versionType', 'release',
        ];
  return [resolved.java.path, ...jvm, mainClass, ...progArgs];
}

/**
 * Resolve everything a launch needs: JVM, version JSON (merged inheritance
 * chain for neoforge), libraries (installed unless dryRun), Fabric profile,
 * NeoForge loader profile, assets, classpath, AOT key.
 * `dryRun: true` never downloads or writes — it only resolves paths.
 */
/**
 * Integrated Client QoL, seeded into options.txt before the game starts — the
 * zero-mod counterpart of the gamma-utils fullbright mod, so it works for
 * vanilla instances that can't load Fabric mods:
 *   - fullbright_on_launch -> `gamma:1.0` (the maximum vanilla allows)
 *
 * Vanilla clamps gamma to [0,1] on load (MC-51418, fixed in 22w12a) — values
 * above 1.0 in options.txt are no longer honored, so a TRUE fullbright needs
 * the gamma-utils mod (QoL bundle); `gamma:1.0` is the honest maximum this
 * seeding can deliver. There is NO vanilla options.txt key for fog (a
 * `fogToggle` line is silently ignored), so no-fog is only reachable through
 * the clear-fog mod — the previous `fogToggle:0` seeding was a silent no-op
 * and has been removed.
 *
 * Fabric gate: fabric instances get their fullbright from the gamma-utils
 * mod (managed feature, Contract A), so seeding `gamma:1.0` there is
 * skipped — the vanilla fallback is vanilla-only. `gameDir` optionally
 * redirects the options.txt target (per-account profile dirs, Contract C);
 * it defaults to the instance dir.
 *
 * When the preference is off this returns before touching the file. The game
 * merges defaults for any key we don't set, so a freshly-created file or an
 * existing one both behave. Launch must never fail over a preference — errors
 * are logged and ignored.
 *
 * Exported (previously private) so tests/preferences.test.mjs can unit-test it
 * in isolation against a sandboxed ESPECTRAL_DATA_DIR; behavior unchanged.
 */
export function applyLaunchPreferences(instance, gameDir = null) {
  try {
    if (instance.loader === 'fabric') return; // gamma-utils owns gamma there
    const cfg = loadConfig();
    if (cfg.fullbright_on_launch !== true) return;
    const optionsPath = path.join(gameDir ?? resolver.instanceDir(instance.name), 'options.txt');
    let parsed;
    try {
      parsed = parseOptionsTxt(fs.readFileSync(optionsPath, 'utf8'));
    } catch {
      parsed = parseOptionsTxt(''); // no file yet -> seed the keys alone
    }
    setOptionKey(parsed, 'gamma', '1.0');
    fs.writeFileSync(optionsPath, writeOptionsTxt(parsed), 'utf8');
  } catch (e) {
    console.warn(`[launch] could not apply launch preferences: ${e.message}`);
  }
}

/**
 * Run-dir decision for a launch.
 * - instance.game_dir set (non-empty string): wins as gameDir (mkdir -p
 *   unless dryRun); the per-account profiles split is skipped and natives
 *   stay at the instance default.
 * - Otherwise (Contract C): a non-active account gets its own
 *   <instanceDir>/profiles/<uuid>/ for BOTH gameDir and nativesDir so
 *   concurrent launches under different accounts don't collide on
 *   options.txt/logs. mods/, libraries, assets, version json stay shared
 *   (read-only at boot). Active-account launches keep the instance dir.
 */
export function selectGameDir(instance, launchAccount, { dryRun = false, warnings = null } = {}) {
  const base = resolver.instanceDir(instance.name);
  if (typeof instance.game_dir === 'string' && instance.game_dir.length > 0) {
    if (!dryRun) {
      try {
        fs.mkdirSync(instance.game_dir, { recursive: true });
      } catch (e) {
        warnings?.push(`could not create game_dir: ${e.message}`);
      }
    }
    return { gameDir: instance.game_dir, nativesDir: resolver.instanceNativesDir(instance.name) };
  }
  if (!launchAccount) {
    return { gameDir: base, nativesDir: resolver.instanceNativesDir(instance.name) };
  }
  const activeAccount = getActiveAccount();
  if (!activeAccount || launchAccount.uuid !== activeAccount.uuid) {
    const profileDir = path.join(base, 'profiles', launchAccount.uuid);
    if (!dryRun) {
      try {
        fs.mkdirSync(path.join(profileDir, 'logs'), { recursive: true });
        fs.mkdirSync(path.join(profileDir, 'config'), { recursive: true });
      } catch (e) {
        warnings?.push(`could not create profile dir: ${e.message}`);
      }
    }
    return { gameDir: profileDir, nativesDir: path.join(profileDir, 'natives') };
  }
  return { gameDir: base, nativesDir: resolver.instanceNativesDir(instance.name) };
}

export async function resolveLaunch(
  instance,
  { mode = 'normal', account = null, dryRun = false, forceVerify = false, onProgress = null, onPhase = null } = {}
) {
  const warnings = [];
  const phase = (name) => {
    try { onPhase?.(name); } catch {}
  };
  onProgress?.(`[espectral] preparing ${instance.name}…`);
  phase('version');
  let versionJson = await resolver.getVersionJson(instance.version);
  onProgress?.('[espectral] Minecraft version resolved');
  phase('version');
  let neoforge = null;
  let nf = null; // NeoForge loader module (lazy; owned by the neoforge slice)
  if (instance.loader === 'neoforge') {
    if (!instance.loader_version) {
      throw Object.assign(new Error('loader_version es obligatorio para NeoForge'), {
        status: 400,
        code: 'LOADER_VERSION_REQUIRED',
      });
    }
    nf = await import('./neoforge.mjs');
    const nfId = nf.neoforgeVersionId(instance.loader_version);
    let nfJson = await resolver.readLocalVersionJson(nfId);
    if (!nfJson) {
      await nf.ensureNeoForgeLoader(instance.loader_version, { instance });
      nfJson = await resolver.readLocalVersionJson(nfId);
      if (!nfJson) {
        throw Object.assign(new Error('NeoForge profile missing after loader install'), {
          status: 502,
          code: 'NEOFORGE_PROFILE_MISSING',
        });
      }
    } else if (!nf.launchArtifactsReady(instance.loader_version, nfJson)) {
      // Profile cached but the generated launch artifacts (srg/extra/universal/
      // client jars) are missing — re-run the ensure, which invokes the
      // official installer once to produce them.
      onProgress?.('[espectral] generando artefactos NeoForge (primera vez)…');
      await nf.ensureNeoForgeLoader(instance.loader_version, { instance });
    }
    // The merged json (child neoforge profile over the vanilla parent) is what
    // everything below consumes — versionMain/assetIndexId are read AFTER the
    // merge so they come from the merged profile.
    versionJson = await resolver.resolveVersionChain(nfJson);
    neoforge = versionJson;
    onProgress?.('[espectral] perfil NeoForge cargado');
  }
  const assetIndexId = versionJson.assetIndex?.id ?? null;
  const versionMain = versionJson.mainClass ?? 'net.minecraft.client.main.Main';

  let java;
  const versionJavaMajor = Number(versionJson.javaVersion?.majorVersion ?? 8);
  if (instance.jdk_path_override) {
    java = await getJvmInfo({ override: instance.jdk_path_override });
  } else if (instance.loader === 'neoforge') {
    // NeoForge 1.21.x must boot on an EXACT Java 21 runtime (a newer major
    // breaks bootstraplauncher module resolution); no tier upgrade allowed.
    java = await javaForMajorExact(21);
  } else if (versionJavaMajor <= 21) {
    // Old-tier versions (1.15.2 – 1.20.4): provision a Temurin runtime for the
    // version's required major. AOT stays a JDK 25+ feature.
    java = await javaForVersion(versionJavaMajor);
  } else {
    java = await getJvmInfo();
  }
  onProgress?.(`[espectral] JDK ${java.major} listo`);
  phase('jdk');

  let fabric = null;
  if (instance.loader === 'fabric') {
    fabric = await resolver.resolveFabric(instance.version);
    phase('fabric');
  } else {
    phase('fabric');
  }

  // Surface MSA refresh failures in the launch log: a silent offline downgrade
  // is indistinguishable from a good launch until the server rejects it.
  phase('account');
  let launchAccount;
  if (account && typeof account.accessToken === 'string') {
    // Already resolved (route's live path passes result of resolveAccountFor)
    launchAccount = {
      username: account.username,
      uuid: account.uuid,
      accessToken: account.accessToken ?? '0',
      userType: account.userType ?? (account.token_kind === 'msa' ? 'msa' : 'mojang'),
    };
  } else {
    launchAccount = await resolveAccount(account, (msg) => {
      warnings.push(msg);
      onProgress?.(`[espectral] ${msg}`);
    });
  }
  phase('account');

  // Run-dir decision (selectGameDir below): a custom instance.game_dir wins
  // outright (mkdir -p, no per-account split); otherwise the per-account
  // profiles split (Contract C) applies.
  const { gameDir, nativesDir } = selectGameDir(instance, launchAccount, { dryRun, warnings });

  if (!dryRun) {
    onProgress?.('[espectral] verificando dependencias…');
    phase('fingerprint');
    let fingerprintHit = false;
    let fingerprintReason = 'unknown';
    if (!forceVerify) {
      try {
        const fp = await import('./fingerprint.mjs');
        const check = await fp.checkFingerprint(instance, versionJson, fabric);
        fingerprintHit = check.hit;
        fingerprintReason = check.reason ?? 'ok';
      } catch {
        fingerprintHit = false;
        fingerprintReason = 'fingerprint unavailable';
      }
    }

    if (fingerprintHit) {
      warnings.push(`fingerprint hit — skipping verification`);
    } else {
      if (!forceVerify) warnings.push(`fingerprint miss (${fingerprintReason}) — verifying`);
      if (fabric) {
        await resolver.installFabricLibraries(instance, fabric, { seedFromFastClient: true });
      }
      await resolver.installLibraries(instance, versionJson, { seedFromFastClient: true, forceExtract: forceVerify });
      onProgress?.('[espectral] libraries ready');
      phase('libraries');
      await resolver.installAssets(instance, versionJson);
      onProgress?.('[espectral] game assets ready');
      phase('assets');
      try {
        const fp = await import('./fingerprint.mjs');
        await fp.writeFingerprintFile(instance, versionJson, fabric);
      } catch (e) {
        warnings.push(`fingerprint write failed: ${e.message}`);
      }
    }

    try {
      const { ensureBrandingSeeded } = await import('./mods.mjs');
      await ensureBrandingSeeded(instance).catch(() => {});
    } catch {}
    phase('branding');
    applyLaunchPreferences(instance, gameDir);
    phase('prefs');
    seedClientConfig(instance.name);
    if (gameDir !== resolver.instanceDir(instance.name)) {
      try {
        const srcCfg = path.join(resolver.instanceDir(instance.name), 'config', 'espectral-client.json');
        const dstCfg = path.join(gameDir, 'config', 'espectral-client.json');
        if (fs.existsSync(srcCfg) && !fs.existsSync(dstCfg)) {
          fs.copyFileSync(srcCfg, dstCfg);
        }
      } catch {
        /* ignore profile config copy error */
      }
    }
    // Fire-and-forget AOT proof-log pruning (lazy import to avoid circular dep with aot.mjs)
    try {
      const aot = await import('./aot.mjs');
      if (typeof aot.pruneAotProofLogs === 'function') {
        void aot.pruneAotProofLogs(instance).catch(() => {});
      }
    } catch {}
  }

  const classpath = resolver.resolveClasspath(instance, versionJson, fabric);
  phase('classpath');
  const assetsDir = resolver.assetsDir();

  onProgress?.('[espectral] launching game…');
  phase('ready');
  const aotKey = cacheKey(instance.version, java.build, process.arch);
  const aotCachePath = cacheFilePath(aotKey);
  let aotCacheExists = false;
  try {
    aotCacheExists = fs.existsSync(aotCachePath) && fs.statSync(aotCachePath).size > 0;
  } catch {
    /* treat as absent */
  }
  // A cache whose recorded classpath identity drifted (any entry's size/mtime)
  // will be REFUSED by the JVM at map time; detect it here so the launch skips
  // -XX:AOTCache and the route can queue a retrain. Meaningless without a
  // cache, so it stays false in that case.
  const aotCacheStale = aotCacheExists ? isCacheStale(aotKey, classpath) : false;

  return {
    java,
    version: {
      id: neoforge ? nf.neoforgeVersionId(instance.loader_version) : instance.version,
      mainClass: versionMain,
      assetIndexId,
      json: versionJson,
    },
    loader: fabric
      ? {
          mainClass: fabric.main_class,
          jvmArgs: gameDir !== resolver.instanceDir(instance.name)
            ? [...(fabric.jvm_args ?? []), `-Dfabric.modsDir=${resolver.instanceModsDir(instance.name)}`]
            : fabric.jvm_args,
          gameArgs: [],
        }
      : neoforge
        ? {
            mainClass: versionJson.mainClass,
            jvmArgs: versionJson.arguments?.jvm ?? [],
            gameArgs: versionJson.arguments?.game ?? [],
            kind: 'neoforge',
          }
        : null,
    fabricProfile: fabric,
    classpath,
    nativesDir,
    gameDir,
    assetsDir,
    memoryMb: instance.memory_mb ?? loadConfig().default_memory_mb ?? DEFAULT_MEMORY_MB,
    account: launchAccount,
    mode,
    aotKey,
    aotCachePath,
    aotCacheExists,
    aotCacheStale,
    aotAvailable: (java?.major ?? 0) >= 25,
    warnings,
  };
}

/**
 * DryRunResult (contract shape): argv + classpath + missing + natives +
 * assets check + java path + warnings. Never spawns, never writes.
 */
export function dryRunResult(instance, resolved) {
  const classpathMissing = resolved.classpath.filter((p) => !fs.existsSync(p));
  const assetIndexFile = path.join(
    resolved.assetsDir,
    'indexes',
    `${resolved.version.assetIndexId ?? ''}.json`
  );
  const argv = buildArgv(instance, resolved);
  // Redact the live --accessToken at the source: the API dry-run response and
  // the CLI preview must never carry the real token (the preview normalizes
  // the token slot, so this is diff-safe).
  return {
    argv: redactPreview(argv, resolved.account?.accessToken),
    classpath: [...resolved.classpath],
    classpath_missing: classpathMissing,
    natives_dir: resolved.nativesDir,
    assets_ok: fs.existsSync(assetIndexFile),
    java_path: resolved.java.path,
    warnings: [...resolved.warnings],
  };
}

/** Harness-style stats for the dry-run report (mirrors resolveLibraries()). */
export function libraryStats(libraries) {
  let jars = 0;
  let skippedNatives = 0;
  let skippedRules = 0;
  for (const lib of libraries ?? []) {
    const parts = String(lib.name ?? '').split(':');
    const classifier = parts.length > 3 ? parts[3] : null;
    if (classifier && classifier.startsWith('natives')) {
      skippedNatives++;
      continue;
    }
    if (!resolver.rulesAllow(lib)) {
      skippedRules++;
      continue;
    }
    jars++;
  }
  return { jars, skippedNatives, skippedRules };
}

/**
 * Spawn indirection (exported for tests): spawnJava calls childProcess.spawn
 * at call time so tests/launch.test.mjs can intercept it with
 * t.mock.method(childProcess, 'spawn', ...) — the direct `spawn` import binding
 * is snapshotted at module link time and cannot be mocked. Production behavior
 * is identical to calling the imported spawn directly.
 */
export const childProcess = { spawn };

/**
 * Spawn java with the full argv (argv[0] = java.exe included, as buildArgv
 * returns). child_process.spawn prepends the executable as argv[0] on EVERY
 * platform (POSIX execvp and the Windows CreateProcess command line alike), so
 * argv[0] must be dropped here — otherwise the JVM would see the exe path twice
 * and treat it as the main class (`spawn(JDK, argv)` with argv
 * starting at the JVM args). The previous win32-only slice left POSIX launches
 * broken (H5); the drop below is unconditional.
 */
export function spawnJava(javaPath, argv, opts) {
  // buildArgv returns [javaPath, ...jvm, main, ...prog]. child_process.spawn
  // prepends the executable as argv[0] on EVERY platform (POSIX execvp and the
  // Windows CreateProcess command line alike), so argv[0] (the java path) must
  // be dropped here — otherwise the java launcher would treat it as the main
  // class. The previous win32-only slice left POSIX launches broken.
  const spawnArgs = argv.slice(1);
  return childProcess.spawn(javaPath, spawnArgs, opts);
}

/**
 * Spawn the game. cwd = gameDir, output piped + relayed to stdout,
 * size-tracked latest.log marker watch. Returns { pid, child, argv }.
 */
export function launchInstance(instance, resolved, { onLog, onMarker, onExit, onSpawned, onPhase } = {}) {
  const argv = buildArgv(instance, resolved);
  const gameDir = resolved.gameDir;
  fs.mkdirSync(path.join(gameDir, 'logs'), { recursive: true });
  const logFile = path.join(gameDir, 'logs', 'latest.log');

  const child = spawnJava(resolved.java.path, argv, {
    cwd: gameDir,
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  if (typeof onSpawned === 'function') {
    try { onSpawned(Date.now()); } catch {}
  }
  const _phaseSeen = new Set();
  const _phaseMap = [
    ['loading_mc', /Loading Minecraft/],
    ['mixin', /MIXIN Subsystem/],
    ['gl_probe', /Searching for graphics cards/],
    ['datafixer', /Datafixer/],
    ['mc_ctor', /Completely ignored arguments/],
    ['mc_user', /Setting user:/],
    ['resources', /Reloading ResourceManager/],
    ['openal', /OpenAL initialized/],
  ];
  const _checkPhases = (s) => {
    if (typeof onPhase !== 'function') return;
    for (const [name, re] of _phaseMap) {
      if (_phaseSeen.has(name)) continue;
      if (re.test(s)) {
        _phaseSeen.add(name);
        try { onPhase(name); } catch {}
      }
    }
  };
  let marker = false;
  let seenSize = fs.existsSync(logFile) ? fs.statSync(logFile).size : -1;
  const iv = setInterval(() => {
    try {
      if (!fs.existsSync(logFile)) return;
      const size = fs.statSync(logFile).size;
      if (seenSize === -1 || size < seenSize) seenSize = 0; // appeared/truncated -> fresh
      if (size > seenSize) {
        const fd = fs.openSync(logFile, 'r');
        const buf = Buffer.alloc(size - seenSize);
        fs.readSync(fd, buf, 0, buf.length, seenSize);
        fs.closeSync(fd);
        seenSize = size;
        if (MARKER_RE.test(buf.toString('utf8'))) {
          marker = true;
          clearInterval(iv);
          if (onMarker) onMarker(Date.now());
        }
      }
    } catch {
      /* log file may be locked mid-write; retry */
    }
  }, 100);

  const relay = (chunk) => {
    try {
      process.stdout.write(chunk);
    } catch {
      /* stdout closed */
    }
  };
  if (child.stdout) {
    child.stdout.on('data', (d) => {
      const s = d.toString('utf8');
      _checkPhases(s);
      // The latest.log watcher below can miss the marker on Windows (the
      // game holds the file with sharing modes that make reads fail); the
      // game's own output stream always carries it.
      if (!marker && onMarker && MARKER_RE.test(s)) {
        marker = true;
        clearInterval(iv);
        onMarker(Date.now());
      }
      if (onLog) onLog(s);
      relay(d);
    });
  }
  if (child.stderr) {
    child.stderr.on('data', (d) => {
      const s = d.toString('utf8');
      _checkPhases(s);
      if (!marker && onMarker && MARKER_RE.test(s)) {
        marker = true;
        clearInterval(iv);
        onMarker(Date.now());
      }
      if (onLog) onLog(s);
      relay(d);
    });
  }
  child.on('exit', (code, signal) => {
    clearInterval(iv);
    if (onExit) onExit({ code, signal, marker });
  });
  child.on('error', (err) => {
    clearInterval(iv);
    if (onExit) onExit({ code: null, signal: null, marker, error: err.message });
  });

  return { pid: child.pid, child, argv };
}

// ---------------------------------------------------------------------------
// CLI: node src/engine/launch.mjs --dry-run --instance <name> [--mode aot|train]
// ---------------------------------------------------------------------------

function redactPreview(argv, accessToken) {
  return argv.map((a) => (a === accessToken ? '<REDACTED>' : a));
}

function quoteArg(arg) {
  return /\s/.test(arg) ? `"${arg}"` : arg;
}

export function formatCommandPreview(argv, accessToken) {
  return redactPreview(argv, accessToken).map(quoteArg).join(' ');
}

function printDryRun(dr, resolved, versionStats, loaderStats) {
  const missing = dr.classpath_missing;
  let totalMb = 0;
  for (const p of dr.classpath) {
    try {
      if (fs.existsSync(p)) totalMb += fs.statSync(p).size;
    } catch {
      /* stat race — ignore */
    }
  }
  const assetIndexFile = path.join(
    resolved.assetsDir,
    'indexes',
    `${resolved.version.assetIndexId ?? ''}.json`
  );
  console.log(
    `loader libs: ${loaderStats.jars} (skipped natives ${loaderStats.skippedNatives}, skipped rules ${loaderStats.skippedRules})`
  );
  console.log(
    `vanilla libs: ${versionStats.jars} (skipped natives ${versionStats.skippedNatives}, skipped rules ${versionStats.skippedRules})`
  );
  console.log(`classpath entries: ${dr.classpath.length}`);
  console.log(`classpath missing (${missing.length}):`);
  for (const p of missing) console.log(`  MISSING ${p}`);
  console.log(`classpath MB total: ${(totalMb / 1048576).toFixed(1)}`);
  console.log(
    `asset index ${resolved.version.assetIndexId}: ${dr.assets_ok ? 'present' : 'MISSING'} (${assetIndexFile})`
  );
  console.log(`java: ${dr.java_path} ${fs.existsSync(dr.java_path) ? 'present' : 'MISSING'}`);
  for (const w of dr.warnings) console.warn(`warning: ${w}`);
  console.log(`COMMAND ${formatCommandPreview(dr.argv, resolved.account?.accessToken)}`);
}

function cliUsage() {
  console.log('usage: node src/engine/launch.mjs --dry-run --instance <name> [--mode normal|aot|train]');
}

const IS_CLI =
  process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href;

if (IS_CLI) {
  const flags = { dryRun: false, instance: null, mode: 'normal' };
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === '--dry-run') flags.dryRun = true;
    else if (a === '--instance') flags.instance = args[++i];
    else if (a === '--mode') flags.mode = args[++i];
    else if (a === '--help' || a === '-h') {
      cliUsage();
      process.exit(0);
    } else {
      console.error(`unknown flag: ${a}`);
      cliUsage();
      process.exit(2);
    }
  }
  if (!flags.dryRun) {
    console.error('real launches go through the server API (POST /api/instances/:name/launch)');
    process.exit(2);
  }
  if (!flags.instance) {
    console.error('--instance <name> is required');
    cliUsage();
    process.exit(2);
  }
  if (!['normal', 'aot', 'train'].includes(flags.mode)) {
    console.error(`--mode must be normal|aot|train (got ${flags.mode})`);
    process.exit(2);
  }

  try {
    const instance = await getInstance(flags.instance);
    const resolved = await resolveLaunch(instance, { mode: flags.mode, dryRun: true });
    const dr = dryRunResult(instance, resolved);
    const versionStats = libraryStats(resolved.version.json.libraries ?? []);
    const loaderStats = {
      jars: resolved.fabricProfile?.libraries?.length ?? 0,
      skippedNatives: 0,
      skippedRules: 0,
    };
    printDryRun(dr, resolved, versionStats, loaderStats);
    if (dr.classpath_missing.length > 0) {
      console.error(`dry-run FAILED: ${dr.classpath_missing.length} classpath file(s) missing`);
      process.exit(1);
    }
    if (!dr.assets_ok) {
      console.error('dry-run FAILED: asset index file missing');
      process.exit(1);
    }
    console.log('dry-run OK');
    process.exit(0);
  } catch (e) {
    console.error(`dry-run FAILED: ${e?.message ?? e}`);
    process.exit(1);
  }
}
