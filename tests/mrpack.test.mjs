import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  parseMrpackIndex,
  sanitizePackName,
  uniqueInstanceName,
  safeJoin,
  resolveMrpackInput,
  findExistingPackInstance,
} from '../src/engine/mrpack.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const MRPACK = path.join(__dirname, 'fixtures', 'mrpack-mini.mrpack');

const tmpDirs = [];
async function makeTmpDir() {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'mrpack-test-'));
  tmpDirs.push(dir);
  return dir;
}
after(() => {
  for (const dir of tmpDirs) fs.rmSync(dir, { recursive: true, force: true });
});

test('parseMrpackIndex: reads name/deps/files from the fixture', async () => {
  const index = await parseMrpackIndex(MRPACK);
  assert.equal(index.name, 'Pack de Prueba');
  assert.equal(index.version_id, '1.0');
  assert.equal(index.dependencies.minecraft, '1.21.1');
  assert.equal(index.dependencies.neoforge, '21.1.242');
  assert.equal(index.files.length, 2);
  assert.equal(index.files[0].path, 'mods/prueba-1.0.jar');
  assert.ok(index.files[0].hashes.sha1, 'sha1 present');
});

test('parseMrpackIndex: rejects non-mrpacks and corrupt indexes', async () => {
  await assert.rejects(
    () => parseMrpackIndex(path.join(__dirname, '..', 'package.json')),
    (err) => err.code === 'NOT_MRPACK'
  );
});

test('sanitizePackName: trims, collapses, strips illegal chars, caps at 40', () => {
  assert.equal(sanitizePackName('Keke Cliente 1.0'), 'Keke Cliente 1.0');
  assert.equal(sanitizePackName('  Keke   Cliente 1.0!! '), 'Keke Cliente 1.0');
  assert.equal(sanitizePackName(''), 'Modpack');
  assert.equal(sanitizePackName(null), 'Modpack');
  assert.equal(sanitizePackName('x'.repeat(60)), 'x'.repeat(40));
  assert.equal(sanitizePackName('a/b\\c:d*e?f"g<h>i|j'), 'abcdefghij'); // only [A-Za-z0-9 ._-] survive
  assert.equal(sanitizePackName('..'), 'Modpack'); // never '.' / '..' / '...' instance names
  assert.equal(sanitizePackName('...'), 'Modpack');
  assert.equal(sanitizePackName('.Keke'), 'Keke'); // leading dots stripped
  assert.equal(sanitizePackName('Keke.'), 'Keke'); // trailing dots stripped
});

test('uniqueInstanceName: dedupes case-insensitively with -2/-3 suffixes', () => {
  assert.equal(uniqueInstanceName('Keke', []), 'Keke');
  assert.equal(uniqueInstanceName('Keke', ['keke']), 'Keke-2');
  assert.equal(uniqueInstanceName('Keke', ['Keke', 'Keke-2']), 'Keke-3');
});

test('safeJoin: allows safe relative paths, rejects escapes', () => {
  const base = 'C:/instances/Keke';
  assert.equal(safeJoin(base, 'mods/a.jar'), path.join(base, 'mods/a.jar'));
  assert.equal(safeJoin(base, 'config/foo/bar.toml'), path.join(base, 'config/foo/bar.toml'));
  assert.equal(safeJoin(base, 'resourcepacks/pack.zip'), path.join(base, 'resourcepacks/pack.zip'));
  for (const bad of ['../evil', 'a/../../b', '/abs', 'C:/abs', 'C:\\abs', 'C:rel', 'c:foo', '', '..']) {
    assert.throws(() => safeJoin(base, bad), (err) => err.code === 'BAD_PATH', `should reject '${bad}'`);
  }
});

test('resolveMrpackInput: folder with a single .mrpack -> zip', async () => {
  const dir = await makeTmpDir();
  const pack = path.join(dir, 'pack.mrpack');
  await fs.promises.writeFile(pack, '');
  const resolved = await resolveMrpackInput(dir);
  assert.deepEqual(resolved, { kind: 'zip', file: pack });
});

test('resolveMrpackInput: folder with modrinth.index.json -> dir', async () => {
  const dir = await makeTmpDir();
  await fs.promises.writeFile(path.join(dir, 'modrinth.index.json'), JSON.stringify({ formatVersion: 1, name: 'P' }));
  const resolved = await resolveMrpackInput(dir);
  assert.deepEqual(resolved, { kind: 'dir', dir });
});

test('resolveMrpackInput: folder with neither -> rejects NOT_MRPACK', async () => {
  const dir = await makeTmpDir();
  await fs.promises.writeFile(path.join(dir, 'readme.txt'), 'hi');
  await assert.rejects(
    () => resolveMrpackInput(dir),
    (err) => err.code === 'NOT_MRPACK'
  );
});

test('resolveMrpackInput: folder with two .mrpacks -> rejects NOT_MRPACK', async () => {
  const dir = await makeTmpDir();
  await fs.promises.writeFile(path.join(dir, 'a.mrpack'), '');
  await fs.promises.writeFile(path.join(dir, 'b.mrpack'), '');
  await assert.rejects(
    () => resolveMrpackInput(dir),
    (err) => err.code === 'NOT_MRPACK'
  );
});

test('resolveMrpackInput: missing path -> rejects FILE_NOT_FOUND', async () => {
  const dir = await makeTmpDir();
  await assert.rejects(
    () => resolveMrpackInput(path.join(dir, 'no-such-file.mrpack')),
    (err) => err.code === 'FILE_NOT_FOUND'
  );
});

test('resolveMrpackInput: a real file -> zip', async () => {
  const dir = await makeTmpDir();
  const file = path.join(dir, 'pack.mrpack');
  await fs.promises.writeFile(file, '');
  const resolved = await resolveMrpackInput(file);
  assert.deepEqual(resolved, { kind: 'zip', file });
});

test('resolveMrpackInput: rejects non-string and empty input', async () => {
  for (const bad of [null, undefined, 42, '', '   ']) {
    await assert.rejects(() => resolveMrpackInput(bad), (err) => err.code === 'BAD_PATH', `should reject ${JSON.stringify(bad)}`);
  }
});

test('findExistingPackInstance: matches exact and case-insensitive modpack names', () => {
  const summaries = [
    { name: 'Alpha', modpack: 'Keke Cliente 1.0' },
    { name: 'Beta', modpack: 'Otro Pack' },
    { name: 'Gamma', modpack: null },
  ];
  assert.equal(findExistingPackInstance(summaries, 'Keke Cliente 1.0'), summaries[0]);
  assert.equal(findExistingPackInstance(summaries, 'keke cliente 1.0'), summaries[0]); // case-insensitive
  assert.equal(findExistingPackInstance(summaries, '  Keke   Cliente 1.0!! '), summaries[0]); // sanitized comparison
});

test('findExistingPackInstance: returns null when nothing matches', () => {
  const summaries = [{ name: 'Alpha', modpack: 'Keke Cliente 1.0' }];
  assert.equal(findExistingPackInstance(summaries, 'Nada Que Ver'), null);
  assert.equal(findExistingPackInstance([], 'Cualquiera'), null);
});

test('findExistingPackInstance: never dedupes on the generic Modpack fallback', () => {
  const summaries = [{ name: 'Alpha', modpack: 'Keke Cliente 1.0' }];
  for (const bad of ['', '!!', '...', 'Modpack', null, undefined]) {
    assert.equal(findExistingPackInstance(summaries, bad), null, `should not match '${bad}'`);
  }
});
