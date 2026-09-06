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
