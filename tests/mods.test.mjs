import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
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

// --- branding preset (bundled Espectral Menu) ---

test('supportsBranding: exactly the bundled versions', () => {
  assert.equal(supportsBranding('1.21.11'), true);
  assert.equal(supportsBranding('26.2'), true);
  assert.equal(supportsBranding('1.22.4'), false);
  assert.equal(supportsBranding('1.20.4'), false);
  assert.equal(supportsBranding(''), false);
  assert.equal(supportsBranding(null), false);
});

test('brandingVersions: both pinned versions covered', () => {
  const versions = brandingVersions();
  assert.ok(versions.includes('1.21.11'), '1.21.11 must be covered');
  assert.ok(versions.includes('26.2'), '26.2 must be covered');
});

test('brandingPinForVersion: pin matches the bundled jar on disk (sha1)', () => {
  for (const version of brandingVersions()) {
    const pin = brandingPinForVersion(version);
    assert.ok(pin, `pin for ${version}`);
    assert.equal(pin.slug, 'espectral-menu');
    assert.equal(pin.filename, `espectral-menu-${version}-${pin.version_number}.jar`);
    const data = readFileSync(path.join(BRANDING_DIR, pin.filename));
    const sha1 = createHash('sha1').update(data).digest('hex');
    assert.equal(pin.sha1, sha1, `${pin.filename} sha1 must match the bundled file`);
    assert.equal(pin.size, data.length, `${pin.filename} size must match`);
  }
  assert.equal(brandingPinForVersion('1.20.4'), null);
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
