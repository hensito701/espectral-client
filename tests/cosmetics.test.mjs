import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import crypto from 'node:crypto';

// Sandbox the data dir BEFORE importing engine modules (config caches it).
const originalDataDir = process.env.ESPECTRAL_DATA_DIR;
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-cosmetics-test-'));
process.env.ESPECTRAL_DATA_DIR = dataDir;

const cosmetics = await import('../src/engine/cosmetics.mjs');
const identity = await import('../src/engine/identity.mjs');
const grant = await import('../src/engine/grant.mjs');

const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
grant.registerGrantKey('test-cos', publicKey);

const UUID = 'aabbccddeeff00112233445566778899';
const NOW = Math.floor(Date.now() / 1000);

function signedGrant(over = {}) {
  return grant.signGrant(
    {
      v: 1,
      iss: grant.GRANT_ISS,
      aud: grant.GRANT_AUD,
      sub: UUID,
      flags: ['supporter', 'mvp'],
      iat: NOW,
      exp: NOW + 3600,
      ...over,
    },
    { kid: 'test-cos', privateKey },
  );
}

after(() => {
  if (originalDataDir === undefined) delete process.env.ESPECTRAL_DATA_DIR;
  else process.env.ESPECTRAL_DATA_DIR = originalDataDir;
  fs.rmSync(dataDir, { recursive: true, force: true });
});

// --- loadout validation -----------------------------------------------------

const MVP = ['supporter', 'mvp'];

test('validateLoadout: a legal loadout round-trips with all slots present', () => {
  const out = cosmetics.validateLoadout(
    { espalda: 'alas_eco', cuello: 'bufanda_llama_alma', cabeza: 'orejas_vulpinas' },
    MVP,
  );
  assert.equal(out.espalda, 'alas_eco');
  assert.equal(out.cuello, 'bufanda_llama_alma');
  assert.equal(out.cabeza, 'orejas_vulpinas');
  assert.equal(out.elitros, null);
  // The withdrawn `brazos` slot is gone from the catalog entirely.
  assert.equal('brazos' in out, false);
});

test('validateLoadout: unknown slot/item/wrong slot are rejected', () => {
  assert.throws(() => cosmetics.validateLoadout({ piernas: 'alas_eco' }, MVP), (e) => e.code === 'BAD_LOADOUT');
  assert.throws(() => cosmetics.validateLoadout({ espalda: 'nope' }, MVP), (e) => e.code === 'UNKNOWN_COSMETIC');
  assert.throws(() => cosmetics.validateLoadout({ cuello: 'alas_eco' }, MVP), (e) => e.code === 'WRONG_SLOT');
  assert.throws(() => cosmetics.validateLoadout('manto_espectral', MVP), (e) => e.code === 'BAD_LOADOUT');
});

test('validateLoadout: mvp items require the mvp flag', () => {
  assert.throws(
    () => cosmetics.validateLoadout({ espalda: 'manto_espectral' }, []),
    (e) => e.code === 'FLAG_REQUIRED',
  );
  assert.throws(
    () => cosmetics.validateLoadout({ espalda: 'manto_espectral' }, ['supporter']),
    (e) => e.code === 'FLAG_REQUIRED',
  );
});

test('validateLoadout: scarf + cloak exclusivity is enforced, wings are fine', () => {
  // Exclusive group: a cuello item forbids the three cloaks.
  assert.throws(
    () => cosmetics.validateLoadout({ cuello: 'bufanda_llama_alma', espalda: 'manto_espectral' }, MVP),
    (e) => e.code === 'EXCLUSIVE_CONFLICT',
  );
  assert.throws(
    () => cosmetics.validateLoadout({ cuello: 'bufanda_llama_alma', espalda: 'jirones_ceniza' }, MVP),
    (e) => e.code === 'EXCLUSIVE_CONFLICT',
  );
  // Wings are not in the exclusive list.
  const ok = cosmetics.validateLoadout({ cuello: 'bufanda_llama_alma', espalda: 'alas_eco' }, MVP);
  assert.equal(ok.espalda, 'alas_eco');
});

test('sanitizeLoadout drops disallowed items and resolves exclusivity quietly', () => {
  const state = { espalda: 'manto_espectral', cuello: 'bufanda_llama_alma', cabeza: 'nope' };
  const clean = cosmetics.sanitizeLoadout(state, MVP);
  // Exclusivity: an equipped cuello (scarf) forbids the listed cloaks — the
  // cloak drops, the scarf survives.
  assert.equal(clean.espalda, null);
  assert.equal(clean.cuello, 'bufanda_llama_alma');
  assert.equal(clean.cabeza, null); // unknown id
  assert.equal('brazos' in clean, false); // slot withdrawn 2026-10-25
  // No flags at all -> everything gated drops.
  const empty = cosmetics.sanitizeLoadout(state, []);
  assert.ok(Object.values(empty).every((v) => v === null));
});

test('sanitizeLoadout tolerates a stored loadout that still names the withdrawn brazos slot', () => {
  // Installs that equipped Vendas Rúnicas before 2026-10-25 keep the id in
  // their persisted loadout. The slot no longer exists, so sanitizing drops it
  // instead of throwing, and the surviving slots are untouched.
  const stored = {
    espalda: 'alas_eco',
    cuello: null,
    cabeza: 'orejas_felinas',
    brazos: 'vendas_runicas',
    elitros: null,
  };
  const clean = cosmetics.sanitizeLoadout(stored, MVP);
  assert.deepEqual(clean, {
    espalda: 'alas_eco',
    cuello: null,
    cabeza: 'orejas_felinas',
    elitros: null,
  });
  // validateLoadout, by contrast, rejects it: a client sending the slot today
  // is out of date, and the engine says so explicitly.
  assert.throws(
    () => cosmetics.validateLoadout(stored, MVP),
    (e) => e.code === 'BAD_LOADOUT' && /brazos/.test(e.message),
  );
});

// --- cosmetics file ---------------------------------------------------------

test('writeCosmeticsFile writes the contract document (grant carried, loadout sanitized)', async () => {
  const gameDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-gamedir-'));
  identity.updateEntry(UUID, { grant: signedGrant(), flags: MVP });
  await cosmetics.patchCosmetics(
    UUID,
    { share: true, loadout: { espalda: 'alas_eco', cuello: 'bufanda_llama_alma' }, hidden_players: ['Griefer_1'] },
    { flags: MVP },
  );
  cosmetics.writeCosmeticsFile(UUID, gameDir);
  const doc = JSON.parse(
    fs.readFileSync(path.join(gameDir, 'config', 'espectral-cosmetics.json'), 'utf8'),
  );
  assert.equal(doc.schema, 1);
  assert.equal(doc.uuid, UUID);
  assert.equal(doc.share, true);
  assert.equal(doc.emotes, true);
  assert.ok(typeof doc.grant === 'string' && doc.grant.split('.').length === 3);
  assert.deepEqual(doc.loadout, {
    espalda: 'alas_eco',
    cuello: 'bufanda_llama_alma',
    cabeza: null,
    elitros: null,
  });
  assert.deepEqual(doc.hidden_players, ['Griefer_1']);
});

test('cosmetics file: invalid/expired grant writes grant:null + empty loadout', () => {
  const gameDir = fs.mkdtempSync(path.join(os.tmpdir(), 'espectral-gamedir-'));
  // Tampered grant: payload says mvp but the signature no longer matches.
  const good = signedGrant();
  const [h, , s] = good.split('.');
  const tampered = `${h}.${Buffer.from(JSON.stringify({ v: 1, iss: grant.GRANT_ISS, aud: grant.GRANT_AUD, sub: UUID, flags: ['mvp'], iat: NOW, exp: NOW + 3600 })).toString('base64url')}.${s}`;
  identity.updateEntry(UUID, { grant: tampered, flags: MVP });
  cosmetics.writeCosmeticsFile(UUID, gameDir);
  const doc = JSON.parse(
    fs.readFileSync(path.join(gameDir, 'config', 'espectral-cosmetics.json'), 'utf8'),
  );
  assert.equal(doc.grant, null);
  assert.ok(Object.values(doc.loadout).every((v) => v === null), 'no grant -> empty loadout');
  // share/hidden survive (they are client prefs, not grant-gated).
  assert.equal(doc.share, true);
  assert.deepEqual(doc.hidden_players, ['Griefer_1']);
});

test('patchCosmetics validates unknown fields, hidden names, emotes/share types', async () => {
  const UUID2 = 'ffffffffffffffffffffffffffffffff';
  await assert.rejects(cosmetics.patchCosmetics(UUID2, { bogus: 1 }, { flags: [] }), (e) => e.code === 'BAD_PATCH');
  await assert.rejects(cosmetics.patchCosmetics(UUID2, { share: 'yes' }, { flags: [] }), (e) => e.code === 'BAD_SHARE');
  await assert.rejects(
    cosmetics.patchCosmetics(UUID2, { hidden_players: ['not a name!'] }, { flags: [] }),
    (e) => e.code === 'BAD_HIDDEN',
  );
  const out = await cosmetics.patchCosmetics(UUID2, { share: true, emotes: false }, { flags: [] });
  assert.equal(out.share, true);
  assert.equal(out.emotes, false);
});
