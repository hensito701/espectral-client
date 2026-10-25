import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  presenceForInstance,
  presenceForServer,
  presenceEventFromLog,
  PRESENCE_MENU_MARKER,
} from '../src/engine/routes/launch.mjs';

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
  assert.equal(pServer.state, 'Jugando en main.uhce.top');
  assert.equal(pServer.image, 'espectral_logo');
});

test('presenceForInstance: modpack state capped under Discord 128 chars', () => {
  const long = 'x'.repeat(300);
  const p = presenceForInstance({ name: 'long', version: '1.21.1', modpack: long });
  assert.ok(p.state.length <= 128);
  assert.equal(p.state, ('Jugando ' + long).slice(0, 128));
});

test('presenceForInstance: non-modpack fallthrough unchanged', () => {
  assert.equal(presenceForInstance({ name: 'default', version: '1.21.11', loader: 'fabric' }).state, 'En el menú principal');
  assert.equal(presenceForInstance({ name: 'single', version: '25w11a' }).state, 'Jugando singleplayer');
  assert.equal(presenceForInstance(null).state, 'En el launcher');
  assert.equal(presenceForInstance({}).state, 'En el launcher');
});

test('presenceEventFromLog: joins, singleplayer and leaves from real game log lines', () => {
  assert.deepEqual(
    presenceEventFromLog('[22:12:11] [Render thread/INFO]: Connecting to main.uhce.top, 25565'),
    { server: 'main.uhce.top' },
  );
  assert.deepEqual(
    presenceEventFromLog('[19:03:40] [Server thread/INFO]: Starting integrated minecraft server version 26.2'),
    { singleplayer: true },
  );
  assert.deepEqual(
    presenceEventFromLog(`[19:20:01] [Render thread/INFO]: ${PRESENCE_MENU_MARKER}`),
    { menu: true },
  );
  assert.deepEqual(
    presenceEventFromLog("[19:20:01] [Server Connector #1/ERROR]: Couldn't connect to server"),
    { menu: true },
  );
  // Chat that merely mentions the phrase is not a connection.
  assert.equal(presenceEventFromLog('[CHAT] <Yoshu> Connecting to main ya voy'), null);
  assert.equal(presenceEventFromLog('[Render thread/INFO]: Loaded 1234 recipes'), null);
});

test('presenceForServer: MAIN, Mini and Meetups get their own line; others stay generic', () => {
  const main = presenceForServer('main.uhce.top');
  assert.equal(main.details, 'Espectral UHC · MAIN');
  assert.equal(main.state, 'main.uhce.top');
  // Legacy names, raw node IP, case and the trailing DNS dot route to MAIN.
  for (const h of ['uhc2.espectral.es', '2.27.233.26', 'MAIN.uhce.top', 'main.uhce.top.']) {
    assert.equal(presenceForServer(h).details, 'Espectral UHC · MAIN', h);
  }
  assert.equal(presenceForServer('mini.uhce.top').details, 'Espectral UHC · Mini');
  const meetups = presenceForServer('24h.uhce.top');
  assert.equal(meetups.details, 'Espectral Meetups 24/7');
  assert.equal(meetups.image, '24h');
  assert.equal(presenceForServer('247.espectral.es').details, 'Espectral Meetups 24/7');
  // A private server's address never reaches the public profile.
  const other = presenceForServer('play.hypixel.net');
  assert.equal(other.state, 'Jugando servidor multijugador');
  assert.equal(other.details, undefined);
  assert.ok(!JSON.stringify(other).includes('hypixel'));
});
