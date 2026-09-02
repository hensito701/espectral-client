import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { readZipEntry, listZipEntries, zipHasEntry } from '../src/engine/ziputil.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.join(__dirname, 'fixtures');
const INSTALLER = path.join(FIXTURES, 'fake-neoforge-installer.jar');
const MRPACK = path.join(FIXTURES, 'mrpack-mini.mrpack');

test('readZipEntry: extracts the version.json from the fake installer jar', () => {
  const buf = readZipEntry(INSTALLER, 'version.json');
  assert.ok(buf, 'version.json entry must exist');
  assert.ok(buf.length > 1000, 'version.json must be substantial');
  assert.equal(buf[0], 0x7b, 'must start with "{"');
  const parsed = JSON.parse(buf.toString('utf8'));
  assert.equal(parsed.id, 'neoforge-21.1.242');
  assert.equal(parsed.inheritsFrom, '1.21.1');
  assert.ok(Array.isArray(parsed.libraries));
});

test('readZipEntry: returns null for a missing entry', () => {
  assert.equal(readZipEntry(INSTALLER, 'nope.json'), null);
  assert.equal(readZipEntry(INSTALLER, ''), null);
});

test('readZipEntry: reads mrpack index and overrides entries', () => {
  const index = readZipEntry(MRPACK, 'modrinth.index.json');
  assert.ok(index, 'modrinth.index.json must exist');
  const parsed = JSON.parse(index.toString('utf8'));
  assert.equal(parsed.name, 'Pack de Prueba');
  const toml = readZipEntry(MRPACK, 'overrides/config/prueba.toml');
  assert.ok(toml, 'override file must exist');
  assert.match(toml.toString('utf8'), /ok = true/);
});

test('zipHasEntry: true/false for present/absent entries', () => {
  assert.equal(zipHasEntry(MRPACK, 'modrinth.index.json'), true);
  assert.equal(zipHasEntry(MRPACK, 'missing.bin'), false);
  assert.equal(zipHasEntry(MRPACK, 'overrides/config/prueba.toml'), true);
});

test('listZipEntries: returns every entry including dirs', () => {
  const entries = listZipEntries(MRPACK);
  assert.ok(entries.includes('modrinth.index.json'));
  assert.ok(entries.includes('overrides/config/prueba.toml'));
  assert.ok(entries.length >= 4, `expected >= 4 entries, got ${entries.length}`);
});

test('ziputil: corrupt file throws, missing file throws', () => {
  const fake = path.join(FIXTURES, 'not-a-zip.bin');
  fs.writeFileSync(fake, Buffer.from([0x00, 0x01, 0x02, 0x03, 0x04]));
  assert.throws(() => readZipEntry(fake, 'x'), /zip corrupt/);
  assert.throws(() => listZipEntries(path.join(FIXTURES, 'no-such.zip')), /ENOENT/);
});
