import { test } from 'node:test';
import assert from 'node:assert/strict';
import { presenceForInstance } from '../src/engine/routes/launch.mjs';

test('presenceForInstance: modpack instance shows the pack name', () => {
  const p = presenceForInstance({ name: 'Keke Cliente 1.0', version: '1.21.1', modpack: 'Keke Cliente', loader: 'neoforge' });
  assert.equal(p.state, 'Jugando Keke Cliente');
  assert.equal(p.image, 'espectral_logo');
});

test('presenceForInstance: modpack wins over server-key heuristics', () => {
  const p = presenceForInstance({ name: 'mi 24h modpack', version: '1.21.1', modpack: 'Pack 24h' });
  assert.equal(p.state, 'Jugando Pack 24h');
  // manual server instances (no modpack) still get the server line
  const pServer = presenceForInstance({ name: 'uhc2', version: '1.21.11', loader: 'fabric' });
  assert.equal(pServer.state, 'Jugando en uhc2.espectral.es');
  assert.equal(pServer.image, 'uhc');
});

test('presenceForInstance: modpack state capped under Discord 128 chars', () => {
  const long = 'x'.repeat(300);
  const p = presenceForInstance({ name: 'long', version: '1.21.1', modpack: long });
  assert.ok(p.state.length <= 128);
  assert.equal(p.state, ('Jugando ' + long).slice(0, 128));
});

test('presenceForInstance: non-modpack fallthrough unchanged', () => {
  assert.equal(presenceForInstance({ name: 'default', version: '1.21.11', loader: 'fabric' }).state, 'Jugando servidor multijugador');
  assert.equal(presenceForInstance({ name: 'single', version: '25w11a' }).state, 'Jugando singleplayer');
  assert.equal(presenceForInstance(null).state, 'En el launcher');
  assert.equal(presenceForInstance({}).state, 'En el launcher');
});
