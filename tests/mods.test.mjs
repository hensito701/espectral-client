import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import { readFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  supportsPerformanceBundle,
  pinsForVersion,
  PINS_BY_VERSION,
  PINNED_VERSIONS,
  supportsBranding,
  brandingPinForVersion,
  brandingVersions,
  installPreset,
  listMods,
  isFilenameForSlug,
  slugifyTitle,
  planModInstall,
  installModrinthMod,
  ensureBrandingSeeded,
} from '../src/engine/mods.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..');
const BRANDING_DIR = path.join(REPO_ROOT, 'assets', 'branding');

test('supportsPerformanceBundle: supported release versions', () => {
  for (const version of ['1.21.11', '26.2', '1.22.4', '21.0']) {
    assert.equal(supportsPerformanceBundle(version), true, `${version} should be supported`);
  }
});

test('supportsPerformanceBundle: unsupported versions', () => {
  for (const version of ['1.20.4', '25w11a', '1.15.2', '27.0']) {
    assert.equal(supportsPerformanceBundle(version), false, `${version} should not be supported`);
  }
});

test('supportsPerformanceBundle: boundary and malformed inputs', () => {
  assert.equal(supportsPerformanceBundle('1.21'), true); // classic lower bound
  assert.equal(supportsPerformanceBundle('1.26'), true); // classic upper bound
  assert.equal(supportsPerformanceBundle('21'), true); // modern lower bound
  assert.equal(supportsPerformanceBundle('26'), true); // modern upper bound
  assert.equal(supportsPerformanceBundle('1.27.0'), false); // classic above range
  assert.equal(supportsPerformanceBundle('27'), false); // modern above range
  assert.equal(supportsPerformanceBundle('1.21.11-pre1'), false); // pre-release
  assert.equal(supportsPerformanceBundle(''), false);
  assert.equal(supportsPerformanceBundle(null), false);
  assert.equal(supportsPerformanceBundle(undefined), false);
  assert.equal(supportsPerformanceBundle(21), false); // non-string
  assert.equal(supportsPerformanceBundle('not-a-version'), false);
});

test('pinsForVersion: exact 7-pin sets for pinned versions (no network)', async () => {
  for (const version of PINNED_VERSIONS) {
    const pins = await pinsForVersion(version);
    assert.equal(pins.length, 7, `${version} should resolve to 7 pins`);
    assert.deepEqual(pins, PINS_BY_VERSION[version]);
  }
});

test('pinsForVersion: null for unsupported versions (no network)', async () => {
  for (const version of ['1.20.4', '1.15.2', '25w11a', '27.0']) {
    assert.equal(await pinsForVersion(version), null, `${version} should resolve to null`);
  }
});

// --- branding preset (bundled per-loader brand jars) ---

test('supportsBranding: loader-aware version coverage', () => {
  assert.equal(supportsBranding('1.21.11'), true); // default loader = fabric
  assert.equal(supportsBranding('1.21.11', 'fabric'), true);
  assert.equal(supportsBranding('26.2', 'fabric'), true);
  assert.equal(supportsBranding('1.21.1', 'neoforge'), true);
  assert.equal(supportsBranding('1.21.1', 'fabric'), false); // no fabric jar for 1.21.1
  assert.equal(supportsBranding('1.21.11', 'neoforge'), false); // no neoforge jar for 1.21.11
  assert.equal(supportsBranding('1.21.11', 'vanilla'), false);
  assert.equal(supportsBranding('26.2', 'vanilla'), false);
  assert.equal(supportsBranding('1.22.4'), false);
  assert.equal(supportsBranding('1.20.4'), false);
  assert.equal(supportsBranding(''), false);
  assert.equal(supportsBranding(null), false);
});

test('brandingVersions: all bundled versions covered', () => {
  const versions = brandingVersions();
  for (const v of ['1.21.11', '26.2', '1.21.1']) {
    assert.ok(versions.includes(v), `${v} must be covered`);
  }
});

test('brandingPinForVersion: pin matches the bundled jar on disk (sha1)', () => {
  const expected = {
    '1.21.11': { loader: 'fabric', prefix: 'espectral-menu', slug: 'espectral-menu' },
    '26.2': { loader: 'fabric', prefix: 'espectral-menu', slug: 'espectral-menu' },
    '1.21.1': { loader: 'neoforge', prefix: 'espectral-brand', slug: 'espectral-brand' },
  };
  for (const [version, { loader, prefix, slug }] of Object.entries(expected)) {
    const pin = brandingPinForVersion(version, loader);
    assert.ok(pin, `pin for ${version}/${loader}`);
    assert.equal(pin.slug, slug);
    assert.equal(pin.filename, `${prefix}-${version}-${pin.version_number}.jar`);
    const data = readFileSync(path.join(BRANDING_DIR, pin.filename));
    const sha1 = createHash('sha1').update(data).digest('hex');
    assert.equal(pin.sha1, sha1, `${pin.filename} sha1 must match the bundled file`);
    assert.equal(pin.size, data.length, `${pin.filename} size must match`);
  }
  assert.equal(brandingPinForVersion('1.20.4'), null);
  assert.equal(brandingPinForVersion('1.21.11', 'neoforge'), null);
});

test('listMods: tags the Fabric branding jar on a real instance dir', async () => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-listmods-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const instDir = path.join(tmp, 'instances', 'brand-fabric-test');
    const modsDir = path.join(instDir, 'mods');
    fs.mkdirSync(modsDir, { recursive: true });
    fs.writeFileSync(path.join(instDir, 'instance.json'),
      JSON.stringify({ name: 'brand-fabric-test', version: '1.21.11', loader: 'fabric' }));
    const pin = brandingPinForVersion('1.21.11', 'fabric');
    fs.copyFileSync(path.join(BRANDING_DIR, pin.filename), path.join(modsDir, pin.filename));
    const rows = await listMods('brand-fabric-test');
    const row = rows.find((r) => r.filename === pin.filename);
    assert.ok(row, 'branding jar listed');
    assert.equal(row.project_slug, 'espectral-menu');
    assert.equal(row.version_number, pin.version_number);
    assert.equal(row.installed, true);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

// A launcher update re-seeds a rebuilt brand jar from the instance page while
// a game started by the previous engine still has the jar open. Overwriting
// in place swapped the bytes under the JVM ("Failed to load class file for
// 'es.spectral.menu.…'" mid-match); the open handle must keep the old bytes.
test('ensureBrandingSeeded never rewrites the jar under an open handle', async () => {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-reseed-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  const pin = brandingPinForVersion('26.2', 'fabric');
  const modsDir = path.join(tmp, 'instances', 'reseed-test', 'mods');
  const jar = path.join(modsDir, pin.filename);
  fs.mkdirSync(modsDir, { recursive: true });
  const stale = Buffer.from('stale brand jar bytes from the previous launcher');
  fs.writeFileSync(jar, stale);
  let fd = fs.openSync(jar, 'r');
  try {
    await ensureBrandingSeeded({ name: 'reseed-test', version: '26.2', loader: 'fabric' });
    const seen = Buffer.alloc(stale.length);
    fs.readSync(fd, seen, 0, seen.length, 0);
    assert.deepEqual(seen, stale, 'the open handle still reads the bytes it opened');
    assert.deepEqual(fs.readdirSync(modsDir), [pin.filename], 'no temp file left behind');
    // Windows defers the swap while the jar is open; once the game is gone
    // the next seed (launch / instance page) installs the pinned jar.
    fs.closeSync(fd);
    fd = null;
    await ensureBrandingSeeded({ name: 'reseed-test', version: '26.2', loader: 'fabric' });
    const onDisk = createHash('sha1').update(fs.readFileSync(jar)).digest('hex');
    assert.equal(onDisk, pin.sha1, 'the path now holds the pinned jar');
  } finally {
    if (fd !== null) fs.closeSync(fd);
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('listMods: NeoForge branding tagged even when performance pins are unresolvable', async (t) => {
  t.mock.method(globalThis, 'fetch', async () => { throw new TypeError('fetch failed'); });
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-listmods-nf-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const instDir = path.join(tmp, 'instances', 'brand-neoforge-test');
    const modsDir = path.join(instDir, 'mods');
    fs.mkdirSync(modsDir, { recursive: true });
    fs.writeFileSync(path.join(instDir, 'instance.json'),
      JSON.stringify({ name: 'brand-neoforge-test', version: '1.21.1', loader: 'neoforge', loader_version: '21.1.242' }));
    const pin = brandingPinForVersion('1.21.1', 'neoforge');
    fs.copyFileSync(path.join(BRANDING_DIR, pin.filename), path.join(modsDir, pin.filename));
    const rows = await listMods('brand-neoforge-test');
    const row = rows.find((r) => r.filename === pin.filename);
    assert.ok(row, 'neoforge branding jar listed despite null performance pins');
    assert.equal(row.project_slug, 'espectral-brand');
    assert.equal(row.installed, true);
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test('installPreset: branding preset rejects unknown presets and missing instances', async () => {
  await assert.rejects(() => installPreset('Invalid!', 'branding'));
  await assert.rejects(() => installPreset('no-such-instance', 'branding'));
  await assert.rejects(() => installPreset('x', 'not-a-preset'), /unknown preset/);
});

// --- installed-mod detection (pure helpers) ---

test('isFilenameForSlug: slug token and prefix matches', () => {
  assert.equal(isFilenameForSlug('sodium-fabric-0.8.7.jar', 'sodium'), true);
  assert.equal(isFilenameForSlug('ferrite-core-8.2.0.jar', 'ferrite-core'), true);
  assert.equal(isFilenameForSlug('iris-mc1.21.1.jar', 'iris'), true);
  assert.equal(isFilenameForSlug('modmenu_1.0.jar', 'modmenu'), true); // underscore token
  assert.equal(isFilenameForSlug('lithium.jar', 'lithium'), true);
  assert.equal(isFilenameForSlug('a.jar', 'a'), true);
});

test('isFilenameForSlug: no partial/substring matches', () => {
  assert.equal(isFilenameForSlug('lithium.jar', 'sodium'), false);
  assert.equal(isFilenameForSlug('ab.jar', 'a'), false);
});

test('isFilenameForSlug: case-insensitive and .jar.disabled stripped', () => {
  assert.equal(isFilenameForSlug('SODIUM-FABRIC-0.8.7.JAR', 'sodium'), true);
  assert.equal(isFilenameForSlug('sodium-fabric-0.8.7.jar.disabled', 'sodium'), true);
  assert.equal(isFilenameForSlug('ferrite-core-8.2.0.jar', 'Ferrite-Core'), true);
});

test('slugifyTitle: Modrinth titles collapse to slug-style ids', () => {
  assert.equal(slugifyTitle('Ferrite Core'), 'ferrite-core');
  assert.equal(slugifyTitle('Sodium Extra'), 'sodium-extra');
  assert.equal(slugifyTitle('  X Y Z  '), 'x-y-z');
  assert.equal(slugifyTitle(null), '');
});

// --- single-mod install with dependencies (Modrinth mocked) ---

function modrinthVersion(projectId, versionId, deps = []) {
  return {
    id: versionId,
    project_id: projectId,
    version_number: '1.0.0',
    status: 'listed',
    dependencies: deps,
    files: [{ primary: true, filename: `${projectId}-1.0.0.jar`, url: `https://cdn.test/${projectId}.jar`, hashes: {}, size: null }],
  };
}

/** Fake Modrinth: versions per project, pinned versions, installed-by-hash, titles. */
function fakeModrinth({ versions, pinned = {}, installedByHash = {} }) {
  return async (url, opts) => {
    const u = String(url);
    if (u.startsWith('https://cdn.test/')) return new Response(Buffer.from(`jar:${u}`));
    if (u.endsWith('/version_files') && opts?.method === 'POST') {
      const { hashes } = JSON.parse(opts.body);
      const out = {};
      for (const h of hashes) if (installedByHash[h]) out[h] = { project_id: installedByHash[h] };
      return new Response(JSON.stringify(out));
    }
    const proj = u.match(/\/project\/([^/]+)\/version\?/);
    if (proj) return new Response(JSON.stringify(versions[decodeURIComponent(proj[1])] ?? []));
    const ver = u.match(/\/version\/([^/?]+)$/);
    if (ver && pinned[ver[1]]) return new Response(JSON.stringify(pinned[ver[1]]));
    if (u.includes('/projects?ids=')) {
      const ids = JSON.parse(decodeURIComponent(u.split('ids=')[1]));
      return new Response(JSON.stringify(ids.map((id) => ({ id, title: `Title ${id}`, slug: id.toLowerCase() }))));
    }
    return new Response('not found', { status: 404 });
  };
}

async function withInstance(name, fn) {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-moddeps-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const instDir = path.join(tmp, 'instances', name);
    const modsDir = path.join(instDir, 'mods');
    fs.mkdirSync(modsDir, { recursive: true });
    const instance = { name, version: '26.2', loader: 'fabric' };
    fs.writeFileSync(path.join(instDir, 'instance.json'), JSON.stringify(instance));
    await fn({ instance, modsDir });
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

const sha1Of = (s) => createHash('sha1').update(s).digest('hex');

test('planModInstall: required deps resolve transitively; installed, embedded, missing, optional and conflicts are classified', async (t) => {
  await withInstance('deps-plan', async ({ instance, modsDir }) => {
    // Two jars already in the mods dir, identified by sha1 (filenames are arbitrary).
    fs.writeFileSync(path.join(modsDir, 'whatever-api.jar'), 'installed-api');
    fs.writeFileSync(path.join(modsDir, 'old.jar'), 'installed-enemy');
    t.mock.method(globalThis, 'fetch', fakeModrinth({
      versions: {
        PROOT1: [modrinthVersion('PROOT1', 'root-v', [
          { project_id: 'PDEPB1', dependency_type: 'required' },
          { project_id: 'PAPI01', dependency_type: 'required' },
          { project_id: 'PEMB01', dependency_type: 'embedded' },
          { project_id: 'PMISS1', dependency_type: 'required' },
          { project_id: 'POPT01', dependency_type: 'optional' },
          { project_id: 'PENEMY', dependency_type: 'incompatible' },
        ])],
        PDEPB1: [modrinthVersion('PDEPB1', 'b-v', [{ project_id: 'PDEPC1', version_id: 'c-pinned', dependency_type: 'required' }])],
        PMISS1: [],
      },
      pinned: { 'c-pinned': modrinthVersion('PDEPC1', 'c-pinned') },
      installedByHash: { [sha1Of('installed-api')]: 'PAPI01', [sha1Of('installed-enemy')]: 'PENEMY' },
    }));

    const plan = await planModInstall(instance, 'PROOT1', 'fabric');
    assert.deepEqual(plan.installs.map((i) => i.project_id), ['PROOT1', 'PDEPB1', 'PDEPC1']);
    assert.deepEqual(plan.dependencies.map((d) => d.title), ['Title PDEPB1', 'Title PDEPC1']);
    assert.deepEqual(plan.missing.map((d) => d.project_id), ['PMISS1']);
    assert.deepEqual(plan.optional.map((d) => d.project_id), ['POPT01']);
    assert.deepEqual(plan.conflicts.map((d) => d.project_id), ['PENEMY']);
  });
});

test('installModrinthMod: downloads the mod and its missing dependencies into the mods dir', async (t) => {
  await withInstance('deps-install', async ({ modsDir }) => {
    t.mock.method(globalThis, 'fetch', fakeModrinth({
      versions: {
        PROOT2: [modrinthVersion('PROOT2', 'root2-v', [{ project_id: 'PDEPB2', dependency_type: 'required' }])],
        PDEPB2: [modrinthVersion('PDEPB2', 'b2-v')],
      },
    }));
    const result = await installModrinthMod('deps-install', 'PROOT2', 'fabric');
    assert.deepEqual(result.installed.sort(), ['PDEPB2-1.0.0.jar', 'PROOT2-1.0.0.jar']);
    assert.deepEqual(fs.readdirSync(modsDir).sort(), ['PDEPB2-1.0.0.jar', 'PROOT2-1.0.0.jar']);
  });
});

// --- capture tools (Replay Mod pin + shader packs; Modrinth mocked) ---

const mods = await import('../src/engine/mods.mjs');
const sha1OfBytes = (s) => createHash('sha1').update(s).digest('hex');

const REPLAY_BYTES = 'the pinned replay jar';

/**
 * A replay pin with its own version id. versionMeta() caches by version id for
 * the process, so each test gets a fresh id instead of reusing one that a
 * previous test already cached.
 */
function replayPin(id, overrides = {}) {
  return Object.freeze({
    version_id: id,
    sha1: overrides.sha1 ?? sha1OfBytes(REPLAY_BYTES),
    filename: 'replaymod-26.3-2.6.28.jar',
    version_number: '2.6.28',
  });
}

/** A shader pack pin that verifies against `bytes`, shaped like the real one. */
function testPack(name, bytes) {
  return { filename: name, url: `https://cdn.test/${name}`, sha1: sha1OfBytes(bytes), size: bytes.length };
}

/** Body the fake CDN serves for a shader pack filename. */
const packBody = (name) => `body of ${name}`;
const packPin = (name) => testPack(name, packBody(name));

/**
 * Fake Modrinth + CDN covering a replay pin and the 26.3 performance pins, so
 * installCaptureTools runs end to end with no network.
 * `replaySha1` is what the version metadata CLAIMS for the replay build.
 */
function fakeCaptureModrinth({ pin, replayBytes = REPLAY_BYTES, replaySha1 = null } = {}) {
  const perfPins = PINS_BY_VERSION['26.3'];
  return async (url) => {
    const u = String(url);
    if (u.startsWith('https://cdn.test/')) {
      if (u.endsWith(pin.filename)) return new Response(Buffer.from(replayBytes));
      const name = u.split('/').pop();
      if (name.startsWith('PrettyPack') || name.startsWith('FastPack')) return new Response(Buffer.from(packBody(name)));
      return new Response(Buffer.from(`body of ${name}`));
    }
    const ver = u.match(/\/version\/([^/?]+)$/);
    if (!ver) return new Response(JSON.stringify([]));
    const id = ver[1];
    if (id === pin.version_id) {
      return new Response(
        JSON.stringify({
          id,
          project_id: mods.REPLAY_MOD_PROJECT,
          version_number: pin.version_number,
          dependencies: [],
          files: [
            {
              primary: true,
              filename: pin.filename,
              url: `https://cdn.test/${pin.filename}`,
              hashes: { sha1: replaySha1 ?? pin.sha1 },
              size: null,
            },
          ],
        })
      );
    }
    const perf = perfPins.find((p) => p.version_id === id);
    if (perf) {
      const filename = `${perf.slug}-${perf.version_number}.jar`;
      const body = `body of ${filename}`;
      return new Response(
        JSON.stringify({
          id,
          project_id: perf.project_id,
          version_number: perf.version_number,
          dependencies: [],
          files: [
            {
              primary: true,
              filename,
              url: `https://cdn.test/${filename}`,
              hashes: { sha1: sha1OfBytes(body) },
              size: null,
            },
          ],
        })
      );
    }
    return new Response('{}', { status: 404 });
  };
}

/** Instance dir + sandboxed data dir for one capture-tools test. */
async function withCaptureInstance(fn) {
  const old = process.env.ESPECTRAL_DATA_DIR;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-capture-'));
  process.env.ESPECTRAL_DATA_DIR = tmp;
  try {
    const name = 'capture';
    const instDir = path.join(tmp, 'instances', name);
    const modsDir = path.join(instDir, 'mods');
    fs.mkdirSync(modsDir, { recursive: true });
    const instance = { name, version: '26.3', loader: 'fabric' };
    fs.writeFileSync(path.join(instDir, 'instance.json'), JSON.stringify(instance));
    await fn({ instance, modsDir, instDir });
  } finally {
    if (old === undefined) delete process.env.ESPECTRAL_DATA_DIR;
    else process.env.ESPECTRAL_DATA_DIR = old;
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

test('installCaptureTools: installs the pinned Replay Mod + performance pins + packs, seeds Iris', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir, instDir }) => {
    const pin = replayPin('cap-install-v1');
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin }));
    const pretty = packPin('PrettyPack-1.0.zip');
    const fast = packPin('FastPack-1.0.zip');

    const result = await mods.installCaptureTools(instance, { replayPin: pin, packs: [pretty, fast] });

    assert.equal(result.replay_mod, pin.filename);
    assert.equal(fs.readFileSync(path.join(modsDir, pin.filename), 'utf8'), REPLAY_BYTES);
    assert.equal(fs.readFileSync(path.join(instDir, 'shaderpacks', pretty.filename), 'utf8'), packBody(pretty.filename));
    assert.equal(fs.readFileSync(path.join(instDir, 'shaderpacks', fast.filename), 'utf8'), packBody(fast.filename));
    assert.deepEqual(result.shaderpacks, [pretty.filename, fast.filename]);
    // Iris/Sodium arrive with the performance preset.
    const jars = fs.readdirSync(modsDir);
    assert.ok(jars.some((f) => f.startsWith('iris-')), `iris installed, got ${jars.join(', ')}`);
    assert.ok(jars.some((f) => f.startsWith('sodium-')), `sodium installed, got ${jars.join(', ')}`);
    // The prettiest (first) pack is what Iris is pointed at.
    const props = fs.readFileSync(path.join(instDir, 'config', 'iris.properties'), 'utf8');
    assert.match(props, new RegExp(`shaderPack=${pretty.filename.replace(/\./g, '\\.')}`));
    assert.match(props, /enableShaders=true/);
    assert.equal(result.iris_pack, pretty.filename);
  });
});

test('installCaptureTools: a downloaded jar with the wrong sha1 is discarded, never installed', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir }) => {
    const pin = replayPin('cap-tampered-v1');
    // Metadata claims the pin; the CDN serves different bytes.
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin, replayBytes: 'tampered bytes' }));

    await assert.rejects(() => mods.installCaptureTools(instance, { replayPin: pin, packs: [] }), /sha1 mismatch/);
    assert.deepEqual(fs.readdirSync(modsDir), [], 'neither the jar nor a .part survives');
  });
});

test('installCaptureTools: refuses metadata whose sha1 disagrees with the pin', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir }) => {
    const pin = replayPin('cap-rehashed-v1');
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin, replaySha1: 'f'.repeat(40) }));
    await assert.rejects(
      () => mods.installCaptureTools(instance, { replayPin: pin, packs: [] }),
      /but the pin says/
    );
    assert.deepEqual(fs.readdirSync(modsDir), []);
  });
});

test('installCaptureTools: a second enable neither duplicates the jar nor overrides a chosen pack', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir, instDir }) => {
    const pin = replayPin('cap-idempotent-v1');
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin }));
    const packs = [packPin('PrettyPack-1.0.zip'), packPin('FastPack-1.0.zip')];

    await mods.installCaptureTools(instance, { replayPin: pin, packs });
    // The player picks their own pack, then capture tools are enabled again.
    const cfg = path.join(instDir, 'config', 'iris.properties');
    fs.writeFileSync(cfg, 'shaderPack=BSL_v10.1.8.zip\n');

    const again = await mods.installCaptureTools(instance, { replayPin: pin, packs });

    assert.deepEqual(
      fs.readdirSync(modsDir).filter((f) => f.startsWith('replaymod')),
      [pin.filename],
      'exactly one Replay Mod jar'
    );
    assert.match(fs.readFileSync(cfg, 'utf8'), /shaderPack=BSL_v10\.1\.8\.zip/, 'user selection is never overwritten');
    assert.equal(again.iris_pack, 'BSL_v10.1.8.zip');
  });
});

test('installCaptureTools: a version with no Replay Mod pin is a clear 400, not a crash', async () => {
  await withCaptureInstance(async ({ instDir }) => {
    const instance = { name: 'capture', version: '1.20.1', loader: 'fabric' };
    fs.writeFileSync(path.join(instDir, 'instance.json'), JSON.stringify(instance));
    await assert.rejects(
      () => mods.installCaptureTools(instance),
      (e) => e.status === 400 && e.code === 'CAPTURE_TOOLS_UNSUPPORTED'
    );
  });
});

test('removeCaptureTools: removes only Replay Mod jars (all builds), keeps shaders and Iris', async () => {
  await withCaptureInstance(async ({ modsDir, instDir }) => {
    for (const f of [
      'replaymod-26.3-2.6.28.jar',
      'replaymod-26.2-2.6.27.jar.disabled',
      'replaymod.jar',
      'iris-mc26.3-1.11.7.jar',
      'sodium-mc26.3-0.9.2.jar',
      'notreplaymod-1.0.jar',
      'replaymodplus-1.0.jar',
    ]) {
      fs.writeFileSync(path.join(modsDir, f), 'x');
    }
    fs.mkdirSync(path.join(instDir, 'shaderpacks'), { recursive: true });
    fs.writeFileSync(path.join(instDir, 'shaderpacks', 'BSL_v10.1.8.zip'), 'pack');

    const result = await mods.removeCaptureTools({ name: 'capture', version: '26.3', loader: 'fabric' });

    assert.deepEqual(result.removed.sort(), [
      'replaymod-26.2-2.6.27.jar.disabled',
      'replaymod-26.3-2.6.28.jar',
      'replaymod.jar',
    ]);
    assert.deepEqual(fs.readdirSync(modsDir).sort(), [
      'iris-mc26.3-1.11.7.jar',
      'notreplaymod-1.0.jar',
      'replaymodplus-1.0.jar',
      'sodium-mc26.3-0.9.2.jar',
    ]);
    assert.ok(fs.existsSync(path.join(instDir, 'shaderpacks', 'BSL_v10.1.8.zip')), 'shader packs stay');
  });
});

test('removeCaptureTools: missing mods dir and unsupported versions are clean no-ops', async () => {
  await withCaptureInstance(async ({ modsDir }) => {
    fs.rmSync(modsDir, { recursive: true, force: true });
    assert.deepEqual(await mods.removeCaptureTools({ name: 'capture', version: '26.3', loader: 'fabric' }), {
      removed: [],
    });
    fs.mkdirSync(modsDir, { recursive: true });
    assert.deepEqual(await mods.removeCaptureTools({ name: 'capture', version: '1.20.1', loader: 'fabric' }), {
      removed: [],
    });
  });
});

test('selectDefaultShaderPack: keeps comments/other keys and resolves duplicate keys Java-style', async () => {
  await withCaptureInstance(async ({ instDir }) => {
    const cfg = path.join(instDir, 'config', 'iris.properties');
    fs.mkdirSync(path.dirname(cfg), { recursive: true });
    fs.writeFileSync(cfg, '# Iris config, hand-edited\nshaderPack=OFF\ncolorSpace=SRGB\nshaderPack=\n');

    const res = mods.selectDefaultShaderPack({ name: 'capture', version: '26.3', loader: 'fabric' });

    assert.equal(res.applied, true);
    const out = fs.readFileSync(cfg, 'utf8');
    assert.match(out, /^# Iris config, hand-edited$/m, 'comments survive');
    assert.match(out, /^colorSpace=SRGB$/m, 'unrelated keys survive');
    assert.equal(
      (out.match(/^shaderPack=/gm) ?? []).length,
      1,
      'exactly one shaderPack line, so a stale duplicate cannot win'
    );
    assert.match(out, new RegExp(`^shaderPack=${mods.DEFAULT_SHADER_PACK.filename.replace(/\./g, '\\.')}$`, 'm'));
    assert.match(out, /^enableShaders=true$/m);
    assert.deepEqual(
      fs.readdirSync(path.dirname(cfg)).filter((f) => f.endsWith('.tmp')),
      [],
      'no temp file left behind'
    );
  });
});

test('selectDefaultShaderPack: treats "off"/"OFF" as unset and a real pack as chosen', async () => {
  await withCaptureInstance(async ({ instDir }) => {
    const cfg = path.join(instDir, 'config', 'iris.properties');
    fs.mkdirSync(path.dirname(cfg), { recursive: true });
    const inst = { name: 'capture', version: '26.3', loader: 'fabric' };
    fs.writeFileSync(cfg, 'shaderPack=OFF\n');
    assert.equal(mods.selectDefaultShaderPack(inst).applied, true);
    fs.writeFileSync(cfg, 'shaderPack=   \n');
    assert.equal(mods.selectDefaultShaderPack(inst).applied, true, 'blank is unset');
    fs.writeFileSync(cfg, 'shaderPack=Photon_v1.4.zip\n');
    const kept = mods.selectDefaultShaderPack(inst);
    assert.equal(kept.applied, false);
    assert.equal(kept.pack, 'Photon_v1.4.zip');
  });
});

test('installCaptureTools: a failure after the replay download removes the jar it just installed', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir }) => {
    const pin = replayPin('cap-rollback-v1');
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin }));
    // The performance preset resolves nothing here, so it cannot fail; make the
    // shader-pack download fail instead (the pack pin needs different bytes).
    const brokenPack = { filename: 'BrokenPack.zip', url: 'https://cdn.test/BrokenPack.zip', sha1: 'e'.repeat(40), size: null };

    await assert.rejects(
      () => mods.installCaptureTools(instance, { replayPin: pin, packs: [brokenPack] }),
      /downloadAll/
    );
    assert.equal(fs.existsSync(path.join(modsDir, pin.filename)), false, 'no Replay Mod left with capture_tools false');
    assert.deepEqual(
      fs.readdirSync(modsDir).filter((f) => f.startsWith('replaymod')),
      []
    );
  });
});

test('installCaptureTools: a pre-existing jar with the wrong bytes is replaced, not trusted', async (t) => {
  await withCaptureInstance(async ({ instance, modsDir }) => {
    const pin = replayPin('cap-stale-v1');
    t.mock.method(globalThis, 'fetch', fakeCaptureModrinth({ pin }));
    fs.writeFileSync(path.join(modsDir, pin.filename), 'stale bytes from an older build');

    await mods.installCaptureTools(instance, { replayPin: pin, packs: [] });

    assert.equal(fs.readFileSync(path.join(modsDir, pin.filename), 'utf8'), REPLAY_BYTES);
  });
});
