import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
// Regression for the first-publish login failure: exchangeForMinecraft read
// camelCase fields (accessToken) from the Minecraft API, which returns
// SNAKE_CASE (access_token) — so EVERY device-flow sign-in died with
// MSA_MC_LOGIN_FAILED before an account could be created. These tests mock
// the whole HTTP chain and pin the field names + the account shape.

const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-msauth-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;
const { exchangeForMinecraft, exchangeLunarForMinecraft, timers, prewarmMsaToken } = await import('../src/engine/msauth.mjs');
const { loadConfig, saveConfig } = await import('../src/engine/config.mjs');
after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

const UHS = '1234567890';
const XSTS_TOKEN = 'xsts-jwt-body';
const MC_TOKEN = 'mc-access-token-value';

function jsonResponse(body, status = 200, headers = {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    // Real responses expose a header map; retryAfterSeconds() reads
    // Retry-After through it (rate-limit tests depend on this).
    headers: { get: (name) => headers[String(name).toLowerCase()] ?? null },
    text: async () => JSON.stringify(body),
  };
}

test('exchangeForMinecraft: full happy path with the real SNAKE_CASE API shape', async (t) => {
  const calls = [];
  t.mock.method(globalThis, 'fetch', async (url, opts) => {
    calls.push({ url: String(url), body: opts?.body ?? '' });
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      // THE REAL SHAPE — snake_case. The old code read data.accessToken and
      // threw MSA_MC_LOGIN_FAILED even on this perfect response.
      return jsonResponse({ access_token: MC_TOKEN, expires_in: 86400 });
    }
    if (url === 'https://api.minecraftservices.com/minecraft/profile') {
      assert.equal(opts.headers.Authorization, 'Bearer ' + MC_TOKEN, 'profile uses the MC token');
      return jsonResponse({ id: 'abc123def4564789', name: 'Tester' });
    }
    return jsonResponse({ error: 'unexpected endpoint' }, 404);
  });

  const mc = await exchangeForMinecraft('msa-access-token');
  assert.equal(mc.accessToken, MC_TOKEN);
  assert.equal(mc.username, 'Tester');
  assert.equal(mc.uuid, 'abc123def4564789');
  assert.equal(mc.expiresIn, 86400);
  // identityToken must carry uhs + XSTS token in the XBL3.0 form
  const loginCall = calls.find((c) => c.url.includes('login_with_xbox'));
  assert.ok(loginCall.body.includes(`XBL3.0 x=${UHS};${XSTS_TOKEN}`));
});

test('exchangeForMinecraft: MC login rejection carries HTTP status + body diagnostic', async (t) => {
  t.mock.method(globalThis, 'fetch', async (url) => {
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      return jsonResponse({ path: '/authentication/login_with_xbox', error: 'Forbidden' }, 403);
    }
    return jsonResponse({}, 404);
  });

  await assert.rejects(
    exchangeForMinecraft('msa-access-token'),
    (err) =>
      err.code === 'MSA_MC_LOGIN_FAILED'
      && err.message.includes('HTTP 403')
      && err.message.includes('Forbidden'),
  );
});

// Regression: Mojang rate-limits bursts of login_with_xbox with HTTP 429.
// Before the retry, that error propagated to resolveAccount(), which swallowed
// it and launched with an OFFLINE token — the game looked fine and every
// online-mode server answered "Invalid session".
test('exchangeForMinecraft: retries a 429 MC login burst and succeeds', async (t) => {
  let loginAttempts = 0;
  const waits = [];
  t.mock.method(timers, 'sleep', async (ms) => {
    waits.push(ms);
  });
  t.mock.method(globalThis, 'fetch', async (url) => {
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      loginAttempts++;
      // First two calls are rate-limited, third succeeds.
      return loginAttempts < 3
        ? jsonResponse({ path: '/authentication/login_with_xbox', error: 'Too Many Requests' }, 429)
        : jsonResponse({ access_token: MC_TOKEN, expires_in: 86400 });
    }
    if (url === 'https://api.minecraftservices.com/minecraft/profile') {
      return jsonResponse({ id: 'abc123def4564789', name: 'Tester' });
    }
    return jsonResponse({}, 404);
  });

  const mc = await exchangeForMinecraft('msa-access-token');
  assert.equal(mc.accessToken, MC_TOKEN, 'a rate-limited burst must still yield a real session');
  assert.equal(loginAttempts, 3);
  // Fallback backoff (no Retry-After header in this mock): 1s then 3s.
  assert.deepEqual(waits, [1000, 3000]);
});

test('exchangeForMinecraft: honours a sane Retry-After header on 429', async (t) => {
  const waits = [];
  t.mock.method(timers, 'sleep', async (ms) => {
    waits.push(ms);
  });
  t.mock.method(globalThis, 'fetch', async (url) => {
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      return jsonResponse({ error: 'Too Many Requests' }, 429, { 'retry-after': '2' });
    }
    return jsonResponse({}, 404);
  });

  await assert.rejects(
    exchangeForMinecraft('msa-access-token'),
    (err) => err.code === 'MSA_MC_LOGIN_FAILED' && err.message.includes('HTTP 429'),
  );
  // Retry-After wins over the fallback ladder, and is bounded: 3 attempts max.
  assert.deepEqual(waits, [2000, 2000]);
});

test('exchangeForMinecraft: does NOT retry a non-429 4xx (real rejection)', async (t) => {
  let loginAttempts = 0;
  t.mock.method(timers, 'sleep', async () => {
    throw new Error('must not wait on a non-retryable rejection');
  });
  t.mock.method(globalThis, 'fetch', async (url) => {
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      loginAttempts++;
      return jsonResponse({ path: '/authentication/login_with_xbox', error: 'Forbidden' }, 403);
    }
    return jsonResponse({}, 404);
  });

  await assert.rejects(
    exchangeForMinecraft('msa-access-token'),
    (err) => err.code === 'MSA_MC_LOGIN_FAILED' && err.message.includes('HTTP 403'),
  );
  assert.equal(loginAttempts, 1, '403 is a real rejection — retrying only delays an honest error');
});

test('exchangeLunarForMinecraft: retries a 429 MC login burst and succeeds', async (t) => {
  let loginAttempts = 0;
  t.mock.method(timers, 'sleep', async () => {});
  t.mock.method(globalThis, 'fetch', async (url) => {
    if (url === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (url === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      loginAttempts++;
      return loginAttempts < 3
        ? jsonResponse({ error: 'Too Many Requests' }, 429)
        : jsonResponse({ access_token: MC_TOKEN, expires_in: 86400 });
    }
    if (url === 'https://api.minecraftservices.com/minecraft/profile') {
      return jsonResponse({ id: 'abc123def4564789', name: 'Tester' });
    }
    return jsonResponse({}, 404);
  });

  const mc = await exchangeLunarForMinecraft('msa-access-token');
  assert.equal(mc.accessToken, MC_TOKEN, 'Lunar path must survive the same rate limit');
  assert.equal(mc.username, 'Tester');
});

// ---------------------------------------------------------------------------
// prewarmMsaToken (C11)
// ---------------------------------------------------------------------------

function resetConfig(accounts = [], activeUsername = null) {
  const cfg = loadConfig();
  cfg.accounts = accounts;
  cfg.active_username = activeUsername;
  // Persist without access_token (replacer strips it) — live object keeps it if we set it.
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  // Clean disk token cache between tests.
  try { fs.unlinkSync(path.join(dataDir, 'mc-token-cache.json')); } catch {}
}

test('prewarmMsaToken: no active account -> resolves, no network', async (t) => {
  resetConfig([], null);
  let fetched = false;
  t.mock.method(globalThis, 'fetch', async () => {
    fetched = true;
    return jsonResponse({}, 200);
  });
  await assert.doesNotReject(() => prewarmMsaToken());
  assert.equal(fetched, false, 'no account must not trigger any fetch');
});

test('prewarmMsaToken: offline account -> resolves, no network', async (t) => {
  resetConfig(
    [{ username: 'OfflineJoe', uuid: 'offline-uuid', token_kind: 'offline' }],
    'OfflineJoe',
  );
  let fetched = false;
  t.mock.method(globalThis, 'fetch', async () => {
    fetched = true;
    return jsonResponse({}, 200);
  });
  await assert.doesNotReject(() => prewarmMsaToken());
  assert.equal(fetched, false, 'offline account must not trigger network');
});

test('prewarmMsaToken: msa with fresh cached token -> no-op, no network', async (t) => {
  // Fresh token: expires in 1 hour (>10 min window)
  const freshExp = Date.now() + 60 * 60 * 1000;
  const cfg = loadConfig();
  cfg.accounts = [{
    username: 'MsaFresh',
    uuid: 'fresh-uuid-1234',
    token_kind: 'msa',
    microsoft: { refresh_token: 'refresh-fresh', access_token: 'cached-mc-token', expires_at: freshExp },
  }];
  cfg.active_username = 'MsaFresh';
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  // Keep access_token in memory after saveConfig stripped it from disk
  loadConfig().accounts[0].microsoft.access_token = 'cached-mc-token';
  loadConfig().accounts[0].microsoft.expires_at = freshExp;
  try { fs.unlinkSync(path.join(dataDir, 'mc-token-cache.json')); } catch {}

  let fetched = false;
  t.mock.method(globalThis, 'fetch', async () => {
    fetched = true;
    return jsonResponse({}, 200);
  });
  await assert.doesNotReject(() => prewarmMsaToken());
  assert.equal(fetched, false, 'fresh token must not trigger refresh');
});

test('prewarmMsaToken: msa near-expiry -> refresh attempted, dedupe concurrent calls', async (t) => {
  const nearExp = Date.now() + 2 * 60 * 1000; // 2 min left -> within 10 min window
  const cfg = loadConfig();
  cfg.accounts = [{
    username: 'MsaNear',
    uuid: 'near-uuid-5678',
    token_kind: 'msa',
    microsoft: { refresh_token: 'refresh-near', access_token: 'old-mc-token', expires_at: nearExp },
  }];
  cfg.active_username = 'MsaNear';
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  loadConfig().accounts[0].microsoft.access_token = 'old-mc-token';
  loadConfig().accounts[0].microsoft.expires_at = nearExp;
  try { fs.unlinkSync(path.join(dataDir, 'mc-token-cache.json')); } catch {}

  let fetchCalls = 0;
  const urls = [];
  t.mock.method(timers, 'sleep', async () => {});
  t.mock.method(globalThis, 'fetch', async (url, opts) => {
    fetchCalls++;
    urls.push(String(url));
    if (String(url).includes('/oauth2/v2.0/token')) {
      return jsonResponse({ access_token: 'new-msa-access', refresh_token: 'new-refresh', expires_in: 3600 });
    }
    if (String(url) === 'https://user.auth.xboxlive.com/user/authenticate') {
      return jsonResponse({ Token: 'xbl-token', DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (String(url) === 'https://xsts.auth.xboxlive.com/xsts/authorize') {
      return jsonResponse({ Token: XSTS_TOKEN, DisplayClaims: { xui: [{ uhs: UHS }] } });
    }
    if (String(url) === 'https://api.minecraftservices.com/authentication/login_with_xbox') {
      return jsonResponse({ access_token: MC_TOKEN, expires_in: 86400 });
    }
    if (String(url) === 'https://api.minecraftservices.com/minecraft/profile') {
      return jsonResponse({ id: 'near-uuid-5678', name: 'MsaNear' });
    }
    return jsonResponse({}, 404);
  });
  // Silence expected warn on success path? ensureMinecraftToken logs success; prewarm only warns on failure.
  const warnSpy = [];
  const origWarn = console.warn;
  console.warn = (...args) => warnSpy.push(args.join(' '));
  try {
    await Promise.all([prewarmMsaToken(), prewarmMsaToken()]);
  } finally {
    console.warn = origWarn;
  }
  // One full refresh chain = 5 fetches (token + xbl + xsts + mc login + profile)
  assert.equal(fetchCalls, 5, `two concurrent prewarms must dedupe to one chain, got ${fetchCalls} fetches: ${urls.join(', ')}`);
  assert.equal(warnSpy.length, 0, 'successful refresh must not warn');

  // Second sequential call after refresh should see fresh token and do no network
  let secondFetches = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    secondFetches++;
    return jsonResponse({}, 200);
  });
  await prewarmMsaToken();
  assert.equal(secondFetches, 0, 'after refresh token is fresh, subsequent prewarm must be no-op');
});

test('prewarmMsaToken: swallows refresh errors', async (t) => {
  const nearExp = Date.now() + 1 * 60 * 1000;
  const cfg = loadConfig();
  cfg.accounts = [{
    username: 'MsaFail',
    uuid: 'fail-uuid-9999',
    token_kind: 'msa',
    microsoft: { refresh_token: 'bad-refresh', access_token: 'old-token', expires_at: nearExp },
  }];
  cfg.active_username = 'MsaFail';
  saveConfig({ accounts: cfg.accounts, active_username: cfg.active_username });
  loadConfig().accounts[0].microsoft.access_token = 'old-token';
  loadConfig().accounts[0].microsoft.expires_at = nearExp;
  try { fs.unlinkSync(path.join(dataDir, 'mc-token-cache.json')); } catch {}

  t.mock.method(globalThis, 'fetch', async () => jsonResponse({ error: 'invalid_grant' }, 400));
  const warns = [];
  const origWarn = console.warn;
  console.warn = (...a) => warns.push(a.join(' '));
  try {
    await assert.doesNotReject(() => prewarmMsaToken());
  } finally {
    console.warn = origWarn;
  }
  // Must have warned at most, not thrown.
  assert.ok(warns.length <= 1, 'should warn at most once on failure');
});
