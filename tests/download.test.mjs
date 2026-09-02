import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { downloadFile } from '../src/engine/download.mjs';

/**
 * download.mjs identity preservation.
 *
 * A download with neither sha1 nor size cannot be short-circuited, so it
 * re-fetches on every verification pass. Renaming byte-identical content over
 * the destination gives the file a NEW mtime, and JEP 483 stamps
 * path+size+timestamp for every AOT classpath entry — a single touch of
 * Fabric's hashless intermediary jar permanently invalidated the AOT cache
 * ("timestamp has changed" -> "Unable to map shared spaces") and every boot
 * silently lost the AOT win. finalizePart must therefore keep the original
 * file when the fresh bytes match, and still replace it when they differ.
 */
function withTempDir(fn) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-download-test-'));
  return fn(tmp).finally(() => {
    try {
      fs.rmSync(tmp, { recursive: true, force: true });
    } catch {
      /* windows may hold a handle briefly */
    }
  });
}

/** Serve `body` for any GET; ignores Range (the files here are tiny). */
function serve(body) {
  return async () => new Response(Buffer.from(body));
}

test('downloadFile without sha1/size keeps the existing file (and its mtime) when bytes match', async (t) => {
  await withTempDir(async (tmp) => {
    const dest = path.join(tmp, 'intermediary-0.0.0.jar');
    fs.writeFileSync(dest, 'identical-bytes');
    const stamp = new Date(Date.now() - 5 * 60_000);
    fs.utimesSync(dest, stamp, stamp);
    const before = fs.statSync(dest);

    t.mock.method(globalThis, 'fetch', serve('identical-bytes'));
    await downloadFile('https://maven.example/intermediary-0.0.0.jar', dest);

    const after = fs.statSync(dest);
    assert.equal(after.mtimeMs, before.mtimeMs, 'mtime must survive a no-op re-download');
    assert.equal(fs.readFileSync(dest, 'utf8'), 'identical-bytes');
    assert.equal(fs.existsSync(`${dest}.part`), false, '.part must be cleaned up');
  });
});

test('downloadFile without sha1/size still replaces the file when bytes differ', async (t) => {
  await withTempDir(async (tmp) => {
    const dest = path.join(tmp, 'fabric-loader.jar');
    fs.writeFileSync(dest, 'old-bytes');
    const stamp = new Date(Date.now() - 5 * 60_000);
    fs.utimesSync(dest, stamp, stamp);
    const before = fs.statSync(dest);

    t.mock.method(globalThis, 'fetch', serve('new-bytes-longer'));
    await downloadFile('https://maven.example/fabric-loader.jar', dest);

    assert.equal(fs.readFileSync(dest, 'utf8'), 'new-bytes-longer');
    assert.ok(fs.statSync(dest).mtimeMs > before.mtimeMs, 'real content change must land');
    assert.equal(fs.existsSync(`${dest}.part`), false);
  });
});

test('downloadFile with a matching sha1 never issues a request at all', async (t) => {
  await withTempDir(async (tmp) => {
    const dest = path.join(tmp, 'asm.jar');
    fs.writeFileSync(dest, 'hashed');
    // sha1('hashed')
    const sha1 = '6318553899daae2941718c02508aeee938af1a1c';
    let calls = 0;
    t.mock.method(globalThis, 'fetch', async () => {
      calls++;
      throw new Error('a sha1-verified file must not be re-fetched');
    });
    await downloadFile('https://maven.example/asm.jar', dest, { sha1 });
    assert.equal(calls, 0);
  });
});
