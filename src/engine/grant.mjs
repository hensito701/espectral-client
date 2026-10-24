/**
 * Espectral identity grant verifier (compact JWS, Ed25519/EdDSA).
 *
 * A grant is `b64url(header).b64url(payload).b64url(sig)` where the signature
 * covers the ASCII bytes of the first two parts. The production signing key is
 * pinned here (public half only); the private half lives server-side in
 * espectral-id. Tests register their own throwaway keypair via
 * `registerGrantKey` — grants signed by it use a different kid, so the
 * production pin is never overridden.
 *
 * Header:  {"alg":"EdDSA","kid":"eid1","typ":"ESG"}
 * Payload: {"v":1,"iss":"https://id.uhce.top","aud":"espectral-client",
 *           "sub":"<32 lowercase hex uuid>","flags":[...],"iat":s,"exp":s}
 *
 * BOTH issuers verify: the canonical `https://id.uhce.top` and the legacy
 * `https://id.espectral.es` minted before the zone lapsed (2026-10-22). The
 * service's GRANT_ISS flip is an ops step that can follow this build, so a
 * client upgrade must never orphan grants already in the wild.
 *
 * verifyGrant() throws GrantError on ANY mismatch — a missing/invalid grant
 * means "no ★ and no cosmetics", never a soft pass.
 */
import crypto from 'node:crypto';

export const DEFAULT_KID = 'eid1';
/** Canonical issuer — what espectral-id mints after the GRANT_ISS flip. */
export const GRANT_ISS = 'https://id.uhce.top';
/** Issuer minted by espectral-id until the ops GRANT_ISS flip. */
export const LEGACY_GRANT_ISS = 'https://id.espectral.es';
/** Every `iss` value accepted; anything else is GRANT_BAD_ISS. */
export const GRANT_ISS_ACCEPTED = Object.freeze([GRANT_ISS, LEGACY_GRANT_ISS]);
export const GRANT_AUD = 'espectral-client';
/** Leeway on iat so a freshly minted grant on a fast server clock verifies. */
export const MAX_IAT_SKEW_S = 300;

// Pinned production verifying key (SPKI DER, base64) — kid 'eid1'.
const PINNED_SPKI_B64 = 'MCowBQYDK2VwAyEA77kg5+/Nn6NvArCPuDvLYu3E4rvmYJHzaCGxwyvdlCI=';

/** kid -> KeyObject. */
const keys = new Map();

export class GrantError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'GrantError';
    this.code = code;
    this.status = 400;
  }
}

/**
 * Register a verifying key for a kid. Accepts a KeyObject or a raw
 * {der-base64 spki} string. Tests use their own keypair + kid.
 */
export function registerGrantKey(kid, key) {
  if (typeof kid !== 'string' || !kid) throw new Error('kid must be a non-empty string');
  let keyObject = key;
  if (typeof key === 'string') {
    keyObject = crypto.createPublicKey({ key: Buffer.from(key, 'base64'), format: 'der', type: 'spki' });
  } else if (key && key.type === undefined && key.asymmetricKeyType === undefined) {
    // JWK/plain object form
    keyObject = crypto.createPublicKey({ key, format: 'jwk' });
  }
  keys.set(kid, keyObject);
  return keyObject;
}

registerGrantKey(
  DEFAULT_KID,
  crypto.createPublicKey({ key: Buffer.from(PINNED_SPKI_B64, 'base64'), format: 'der', type: 'spki' }),
);

const B64URL_RE = /^[A-Za-z0-9_-]+$/;

function decodePart(part, what) {
  if (typeof part !== 'string' || part.length === 0 || part.length > 8192 || !B64URL_RE.test(part)) {
    throw new GrantError('GRANT_MALFORMED', `grant ${what} part is not base64url`);
  }
  try {
    return Buffer.from(part, 'base64url');
  } catch {
    throw new GrantError('GRANT_MALFORMED', `grant ${what} part is not decodable`);
  }
}

function parseJsonPart(part, what) {
  let obj;
  try {
    obj = JSON.parse(decodePart(part, what).toString('utf8'));
  } catch (e) {
    if (e instanceof GrantError) throw e;
    throw new GrantError('GRANT_MALFORMED', `grant ${what} is not valid JSON`);
  }
  if (obj === null || typeof obj !== 'object' || Array.isArray(obj)) {
    throw new GrantError('GRANT_MALFORMED', `grant ${what} is not a JSON object`);
  }
  return obj;
}

const UUID32_RE = /^[0-9a-f]{32}$/;

/**
 * Verify a compact EdDSA grant. Returns the payload object on success;
 * throws GrantError with a specific code otherwise.
 *
 * opts.sub: when given, the grant's `sub` must equal it (uuid it is applied
 * to, 32 lowercase hex) — a grant for account A must never unlock account B.
 * opts.now: unix seconds override for tests.
 */
export function verifyGrant(token, { sub = null, now = Math.floor(Date.now() / 1000) } = {}) {
  if (typeof token !== 'string') {
    throw new GrantError('GRANT_MALFORMED', 'grant must be a compact JWS string');
  }
  const parts = token.split('.');
  if (parts.length !== 3) {
    throw new GrantError('GRANT_MALFORMED', 'grant must have exactly 3 dot-separated parts');
  }
  const [h64, p64, s64] = parts;

  const header = parseJsonPart(h64, 'header');
  if (header.alg !== 'EdDSA') {
    throw new GrantError('GRANT_BAD_ALG', `grant alg must be EdDSA (got ${JSON.stringify(header.alg)})`);
  }
  if (typeof header.kid !== 'string' || !keys.has(header.kid)) {
    throw new GrantError('GRANT_UNKNOWN_KID', `grant signed with unknown kid ${JSON.stringify(header.kid)}`);
  }

  const sig = decodePart(s64, 'signature');
  const signingInput = Buffer.from(`${h64}.${p64}`, 'ascii');
  // Ed25519: algorithm=null, signature verified over the raw message bytes.
  const ok = crypto.verify(null, signingInput, keys.get(header.kid), sig);
  if (!ok) {
    throw new GrantError('GRANT_BAD_SIGNATURE', 'grant signature does not verify');
  }

  const payload = parseJsonPart(p64, 'payload');
  if (payload.v !== 1) {
    throw new GrantError('GRANT_BAD_VERSION', `grant v must be 1 (got ${JSON.stringify(payload.v)})`);
  }
  if (!GRANT_ISS_ACCEPTED.includes(payload.iss)) {
    throw new GrantError('GRANT_BAD_ISS', `grant iss mismatch (got ${JSON.stringify(payload.iss)})`);
  }
  if (payload.aud !== GRANT_AUD) {
    throw new GrantError('GRANT_BAD_AUD', `grant aud mismatch (got ${JSON.stringify(payload.aud)})`);
  }
  if (typeof payload.sub !== 'string' || !UUID32_RE.test(payload.sub)) {
    throw new GrantError('GRANT_BAD_SUB', 'grant sub must be a 32-char lowercase hex uuid');
  }
  if (sub !== null && payload.sub !== sub) {
    throw new GrantError('GRANT_WRONG_SUB', 'grant is bound to a different uuid');
  }
  if (!Number.isFinite(payload.exp) || payload.exp <= now) {
    throw new GrantError('GRANT_EXPIRED', 'grant is expired');
  }
  if (!Number.isFinite(payload.iat) || payload.iat > now + MAX_IAT_SKEW_S) {
    throw new GrantError('GRANT_BAD_IAT', 'grant iat is in the future');
  }
  if (payload.flags !== undefined && !Array.isArray(payload.flags)) {
    throw new GrantError('GRANT_MALFORMED', 'grant flags must be an array');
  }
  return payload;
}

/**
 * Sign a grant — TEST/tooling helper only (production grants come from
 * espectral-id, which owns the private key). Accepts a PKCS8 private key.
 */
export function signGrant(payload, { kid = DEFAULT_KID, privateKey } = {}) {
  const header = { alg: 'EdDSA', kid, typ: 'ESG' };
  const h64 = Buffer.from(JSON.stringify(header), 'utf8').toString('base64url');
  const p64 = Buffer.from(JSON.stringify(payload), 'utf8').toString('base64url');
  const sig = crypto.sign(null, Buffer.from(`${h64}.${p64}`, 'ascii'), privateKey);
  return `${h64}.${p64}.${sig.toString('base64url')}`;
}
