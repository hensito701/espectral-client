import { test } from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { offlineUuid, isValidUsername } from '../src/engine/accounts.mjs';

/**
 * Independent reimplementation of the Java offline-UUID algorithm
 * (MD5 of 'OfflinePlayer:'+name with version-3 / RFC 4122 bit-masking),
 * used to cross-check the module's own output.
 */
function localOfflineUuid(username) {
  const bytes = crypto.createHash('md5').update('OfflinePlayer:' + username, 'utf8').digest();
  bytes[6] = (bytes[6] & 0x0f) | 0x30; // version 3 (name-based)
  bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
  const hex = bytes.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

// Third group starts with '3' (version 3); fourth group starts with 8/9/a/b
// (RFC 4122 variant bits 10xx).
const OFFLINE_UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-3[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

test('offlineUuid matches a locally computed MD5 offline UUID', () => {
  for (const name of ['Notch', 'Espectral', 'Steve', 'Player_01', 'a'.repeat(16), 'x']) {
    assert.equal(offlineUuid(name), localOfflineUuid(name), `offlineUuid('${name}')`);
  }
});

test('offlineUuid has the Java offline-UUID format (version 3, RFC 4122 variant)', () => {
  for (const name of ['Notch', 'Espectral', 'Steve']) {
    assert.match(offlineUuid(name), OFFLINE_UUID_RE, `offlineUuid('${name}') format`);
  }
});

test('offlineUuid hardcoded vectors', () => {
  assert.equal(offlineUuid('Notch'), 'b50ad385-829d-3141-a216-7e7d7539ba7f');
  assert.equal(offlineUuid('Steve'), '5627dd98-e6be-3c21-b8a8-e92344183641');
});

test('isValidUsername accepts 1-16 chars of [A-Za-z0-9_]', () => {
  assert.equal(isValidUsername('Notch'), true);
  assert.equal(isValidUsername('Player_01'), true);
  assert.equal(isValidUsername(''), false);
  assert.equal(isValidUsername('has space'), false);
  assert.equal(isValidUsername('x'.repeat(17)), false);
});
