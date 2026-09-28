import { test } from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';

import {
  verifyGrant,
  signGrant,
  registerGrantKey,
  GrantError,
  GRANT_ISS,
  GRANT_AUD,
  DEFAULT_KID,
} from '../src/engine/grant.mjs';

// Throwaway Ed25519 keypair — production grants use the pinned 'eid1' key;
// tests sign with their own kid so the pin is never overridden.
const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
registerGrantKey('test1', publicKey);
const OTHER = crypto.generateKeyPairSync('ed25519');

const SUB = 'aabbccddeeff00112233445566778899';
const NOW = Math.floor(Date.now() / 1000);

function makePayload(over = {}) {
  return {
    v: 1,
    iss: GRANT_ISS,
    aud: GRANT_AUD,
    sub: SUB,
    flags: ['supporter', 'mvp'],
    iat: NOW,
    exp: NOW + 3600,
    ...over,
  };
}
function grant(over = {}, kid = 'test1', key = privateKey) {
  return signGrant(makePayload(over), { kid, privateKey: key });
}

test('valid grant verifies and returns the payload', () => {
  const p = verifyGrant(grant(), { sub: SUB });
  assert.equal(p.sub, SUB);
  assert.deepEqual(p.flags, ['supporter', 'mvp']);
  assert.equal(p.iss, GRANT_ISS);
  assert.equal(p.aud, GRANT_AUD);
});

test('tampered payload fails signature verification', () => {
  const [h, p, s] = grant().split('.');
  // Swap in a different-but-valid JSON payload without re-signing.
  const forged = Buffer.from(JSON.stringify(makePayload({ flags: ['mvp', 'admin'] }))).toString('base64url');
  assert.throws(() => verifyGrant(`${h}.${forged}.${s}`, { sub: SUB }), (e) => e instanceof GrantError && e.code === 'GRANT_BAD_SIGNATURE');
  // And a bit-flipped signature (mid-string: the LAST char of a 64-byte
  // base64url blob carries only pad bits, flipping it decodes identically).
  const idx = Math.floor(s.length / 2);
  const badSig = s.slice(0, idx) + (s[idx] === 'A' ? 'B' : 'A') + s.slice(idx + 1);
  assert.throws(() => verifyGrant(`${h}.${p}.${badSig}`, { sub: SUB }), (e) => e.code === 'GRANT_BAD_SIGNATURE');
});

test('expired grant is rejected', () => {
  assert.throws(
    () => verifyGrant(grant({ iat: NOW - 90000, exp: NOW - 10 }), { sub: SUB }),
    (e) => e.code === 'GRANT_EXPIRED',
  );
});

test('wrong aud / iss / version are rejected', () => {
  assert.throws(() => verifyGrant(grant({ aud: 'minecraft' }), { sub: SUB }), (e) => e.code === 'GRANT_BAD_AUD');
  assert.throws(() => verifyGrant(grant({ iss: 'https://evil.example' }), { sub: SUB }), (e) => e.code === 'GRANT_BAD_ISS');
  assert.throws(() => verifyGrant(grant({ v: 2 }), { sub: SUB }), (e) => e.code === 'GRANT_BAD_VERSION');
});

test('grant bound to another uuid is rejected for this sub', () => {
  const t = grant({ sub: '00000000000000000000000000000001' });
  assert.throws(() => verifyGrant(t, { sub: SUB }), (e) => e.code === 'GRANT_WRONG_SUB');
  // No sub check at all still verifies (caller opted out).
  assert.equal(verifyGrant(t).sub, '00000000000000000000000000000001');
});

test('unknown kid is rejected before signature work', () => {
  const t = signGrant(makePayload(), { kid: 'unknown-kid', privateKey });
  assert.throws(() => verifyGrant(t, { sub: SUB }), (e) => e.code === 'GRANT_UNKNOWN_KID');
});

test('signed by the wrong key under a known kid fails', () => {
  const t = signGrant(makePayload(), { kid: 'test1', privateKey: OTHER.privateKey });
  assert.throws(() => verifyGrant(t, { sub: SUB }), (e) => e.code === 'GRANT_BAD_SIGNATURE');
});

test('the pinned production key is registered and rejects foreign signatures', () => {
  // A grant claiming kid eid1 but signed by the test key must NOT verify —
  // proves the pinned key is active, not a placeholder.
  const t = signGrant(makePayload(), { kid: DEFAULT_KID, privateKey });
  assert.throws(() => verifyGrant(t, { sub: SUB }), (e) => e.code === 'GRANT_BAD_SIGNATURE');
});

test('non-EdDSA alg is rejected', () => {
  const h = Buffer.from(JSON.stringify({ alg: 'HS256', kid: 'test1', typ: 'ESG' })).toString('base64url');
  const p = Buffer.from(JSON.stringify(makePayload())).toString('base64url');
  assert.throws(() => verifyGrant(`${h}.${p}.AAAA`), (e) => e.code === 'GRANT_BAD_ALG');
});

test('future iat beyond skew is rejected', () => {
  assert.throws(
    () => verifyGrant(grant({ iat: NOW + 3600, exp: NOW + 7200 }), { sub: SUB }),
    (e) => e.code === 'GRANT_BAD_IAT',
  );
  // Within the 300 s skew it verifies.
  assert.equal(verifyGrant(grant({ iat: NOW + 120 }), { sub: SUB }).sub, SUB);
});

test('malformed inputs are rejected', () => {
  for (const bad of [null, undefined, 42, '', 'abc', 'a.b', 'a.b.c.d', 'a.b.!']) {
    assert.throws(() => verifyGrant(bad), (e) => e instanceof GrantError, String(bad));
  }
});
