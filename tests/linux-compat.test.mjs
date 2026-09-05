import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';

import { fastClientJava } from '../src/engine/jvm.mjs';

// Spoof process.platform (read-only accessor) for the duration of fn.
function withPlatform(value, fn) {
  const original = Object.getOwnPropertyDescriptor(process, 'platform');
  Object.defineProperty(process, 'platform', { value });
  try {
    return fn();
  } finally {
    Object.defineProperty(process, 'platform', original);
  }
}

test('fastClientJava: null off-Windows (no %APPDATA% FastClient path on Linux)', () => {
  withPlatform('linux', () => {
    assert.equal(fastClientJava(), null);
  });
  withPlatform('darwin', () => {
    assert.equal(fastClientJava(), null);
  });
});

test('fastClientJava: %APPDATA%-based java.exe path on Windows (unchanged)', () => {
  withPlatform('win32', () => {
    const got = fastClientJava();
    const base = process.env.APPDATA || path.join(process.env.USERPROFILE || '', 'AppData', 'Roaming');
    assert.equal(got, path.join(base, 'FastClient', 'runtimes', 'java-25', 'jdk-25.0.4+7', 'bin', 'java.exe'));
  });
});

test('discord: setPresence/clearPresence degrade to quiet no-op with no Discord running', async () => {
  const { setPresence, clearPresence, shutdown } = await import('../src/engine/discord.mjs');
  try {
    // No Discord client in CI: must resolve (not hang past connect timeouts,
    // not throw) on every platform, including the Linux abstract-socket path.
    const t0 = Date.now();
    await setPresence({ details: 'test', state: 'test' });
    await clearPresence();
    assert.ok(Date.now() - t0 < 30_000, 'presence calls must settle promptly without Discord');
  } finally {
    shutdown();
  }
});
