import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const read = (rel) => fs.readFileSync(path.join(root, rel), 'utf8');

const viewer = read('src/ui/src/components/SkinViewer.svelte');
const vault = read('src/ui/src/pages/AccountVault.svelte');
const i18n = read('src/ui/src/lib/i18n/account.ts');

test('SkinViewer exposes autoplay/lazy props with safe defaults', () => {
  assert.match(viewer, /autoplay\?: boolean/, 'autoplay prop declared');
  assert.match(viewer, /lazy\?: boolean/, 'lazy prop declared');
  assert.match(viewer, /autoplay = true, lazy = false/, 'defaults: autoplay on, lazy off');
  assert.match(viewer, /let rotating = \$state\(autoplay\)/, 'initial spin follows autoplay');
});

test('SkinViewer lazy cards never boot WebGL while hidden', () => {
  assert.match(viewer, /let inView = \$state\(!lazy\)/, 'non-lazy viewers stay eager');
  assert.match(viewer, /new IntersectionObserver/, 'lazy boot gated on visibility');
  assert.match(viewer, /if \(!visible \|\| !url \|\| viewerFailed\) return;/, 'effect skips boot until visible');
  assert.match(viewer, /observer\?\.disconnect\(\)/, 'observer released after first reveal');
});

test('frozen cards stay user-controllable once booted', () => {
  assert.match(viewer, /v\.autoRotate = rotating/, 'autoplay=false boots frozen via rotating=false');
  assert.match(viewer, /function toggleSpin/, 'spin toggle still available on frozen cards');
});

test('gallery cards render frozen lazy 3D viewers instead of PNG images', () => {
  assert.match(
    vault,
    /<SkinViewer\s+skinUrl=\{`\$\{librarySkinPngUrl\(entry\.id\)\}/,
    'card art uses library PNG url as SkinViewer texture',
  );
  const card = vault.slice(vault.indexOf('skin-card__art'));
  assert.match(card, /autoplay=\{false\}/, 'cards boot frozen');
  assert.match(card, /height=\{120\}/, 'cards use compact height');
  assert.ok(/<SkinViewer[^>]*\blazy\b/.test(card), 'cards boot lazily (WebGL context cap)');
  assert.ok(!/<button[^>]*class="skin-card__art"/.test(vault), 'art is not a <button> (spin toggle cannot nest)');
  assert.ok(!/<img[^>]*librarySkinPngUrl/.test(vault), 'PNG <img> fully replaced by 3D viewer');
});

test('gallery has an explicit Probar action that stages + reveals the preview', () => {
  assert.match(vault, /\{t\('vault\.skin\.galleryTry'\)\}/, 'Probar button uses the i18n key');
  assert.match(vault, /function previewSkin\(id: string\): void/, 'preview helper exists');
  assert.match(vault, /previewEntryId = id;/, 'Probar stages the entry (rotation happens on stage)');
  assert.match(
    vault,
    /skinStageEl\?\.scrollIntoView\(\{ behavior: 'smooth', block: 'nearest' \}\)/,
    'stage scrolls into view after staging',
  );
  assert.match(vault, /bind:this=\{skinStageEl\}/, 'stage element is bound for scrolling');
});

test('galleryTry labels exist in both locales', () => {
  assert.match(i18n, /'vault\.skin\.galleryTry': 'Probar'/, 'ES label');
  assert.match(i18n, /'vault\.skin\.galleryTry': 'Try'/, 'EN label');
});
