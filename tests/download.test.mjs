import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { downloadFile, fetchJson, resetMirrorPreference } from '../src/engine/download.mjs';

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

/** Undici's shape for every network failure: a bare message, reason in `cause`. */
function networkError(code) {
  return Object.assign(new TypeError('fetch failed'), { cause: Object.assign(new Error(code), { code }) });
}

test('fetchJson survives a transient connection reset', async (t) => {
  let calls = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    calls++;
    if (calls === 1) throw networkError('ECONNRESET');
    return new Response('{"ok":true}');
  });
  assert.deepEqual(await fetchJson('https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'), { ok: true });
  assert.equal(calls, 2);
});

test('fetchJson names the host and the network reason when every attempt fails', async (t) => {
  let calls = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    calls++;
    throw networkError('UND_ERR_CONNECT_TIMEOUT');
  });
  await assert.rejects(
    fetchJson('https://piston-meta.mojang.com/v1/packages/x/26.2.json'),
    (err) => {
      assert.match(err.message, /piston-meta\.mojang\.com/);
      assert.match(err.message, /UND_ERR_CONNECT_TIMEOUT/);
      assert.notEqual(err.message, 'fetch failed');
      return true;
    },
  );
  assert.equal(calls, 3);
});

test('fetchJson does not retry a 404', async (t) => {
  let calls = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    calls++;
    return new Response('nope', { status: 404 });
  });
  await assert.rejects(fetchJson('https://piston-meta.mojang.com/v1/packages/x/9.9.json'), (err) => err.status === 404);
  assert.equal(calls, 1);
});

// Spanish ISPs block Cloudflare IPs during LaLiga matches; meta/maven.fabricmc.net
// are behind Cloudflare, Fabric's meta2./maven2. hosts are not.
test('Fabric meta falls back to meta2 when the primary is unreachable, and sticks to it', async (t) => {
  resetMirrorPreference();
  t.after(resetMirrorPreference);
  const hosts = [];
  t.mock.method(globalThis, 'fetch', async (url) => {
    const host = new URL(url).host;
    hosts.push(host);
    if (host === 'meta.fabricmc.net') throw networkError('UND_ERR_CONNECT_TIMEOUT');
    return new Response(JSON.stringify({ from: host, path: new URL(url).pathname }));
  });
  assert.deepEqual(await fetchJson('https://meta.fabricmc.net/v2/versions/loader/26.2'),
    { from: 'meta2.fabricmc.net', path: '/v2/versions/loader/26.2' });
  // One fast primary try (the mirror is the retry), then the mirror.
  assert.deepEqual(hosts, ['meta.fabricmc.net', 'meta2.fabricmc.net']);
  // The next request goes straight to the mirror — no second connect timeout.
  hosts.length = 0;
  await fetchJson('https://meta.fabricmc.net/v2/versions/loader');
  assert.deepEqual(hosts, ['meta2.fabricmc.net']);
});

test('a 4xx from Fabric meta is a real answer, not an outage: no mirror', async (t) => {
  resetMirrorPreference();
  t.after(resetMirrorPreference);
  const hosts = [];
  t.mock.method(globalThis, 'fetch', async (url) => {
    hosts.push(new URL(url).host);
    return new Response('no loader', { status: 400 });
  });
  await assert.rejects(fetchJson('https://meta.fabricmc.net/v2/versions/loader/1.0'), (err) => err.status === 400);
  assert.deepEqual(hosts, ['meta.fabricmc.net']);
});

test('both Fabric hosts down: the error names both', async (t) => {
  resetMirrorPreference();
  t.after(resetMirrorPreference);
  t.mock.method(globalThis, 'fetch', async () => { throw networkError('UND_ERR_CONNECT_TIMEOUT'); });
  await assert.rejects(fetchJson('https://meta.fabricmc.net/v2/versions/loader'), (err) => {
    assert.match(err.message, /meta\.fabricmc\.net nor its mirror meta2\.fabricmc\.net/);
    return true;
  });
});

test('Fabric maven jars download from maven2 when maven is unreachable', async (t) => {
  resetMirrorPreference();
  t.after(resetMirrorPreference);
  await withTempDir(async (tmp) => {
    const dest = path.join(tmp, 'fabric-loader-0.20.1.jar');
    t.mock.method(globalThis, 'fetch', async (url) => {
      if (new URL(url).host === 'maven.fabricmc.net') throw networkError('UND_ERR_CONNECT_TIMEOUT');
      return new Response(Buffer.from('loader-bytes'));
    });
    await downloadFile('https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar', dest);
    assert.equal(fs.readFileSync(dest, 'utf8'), 'loader-bytes');
  });
});
