import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  buildLangMaps,
  stableStringify,
  REGISTRY_SOURCE,
  DEST_REGISTRY,
  DEST_EN,
  DEST_ES,
} from '../scripts/sync-suite-registry.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const doc = JSON.parse(fs.readFileSync(REGISTRY_SOURCE, 'utf8'));
const en = JSON.parse(fs.readFileSync(DEST_EN, 'utf8'));
const es = JSON.parse(fs.readFileSync(DEST_ES, 'utf8'));
const clientI18n = fs.readFileSync(
  path.join(repoRoot, 'src/ui/src/lib/i18n/client.ts'),
  'utf8',
);

test('registry shape: 13 features, 4 categories, schema 2, suite on by default', () => {
  assert.equal(doc.schema, 2);
  assert.equal(doc.suite?.default_enabled, true);
  assert.equal(doc.features.length, 13);
  assert.equal(doc.categories.length, 4);
  assert.equal(new Set(doc.features.map((f) => f.id)).size, 13);
  for (const f of doc.features) {
    assert.ok(typeof f.name_en === 'string' && typeof f.name_es === 'string', `${f.id} both names`);
    assert.ok(
      typeof f.description_en === 'string' && typeof f.description_es === 'string',
      `${f.id} both descriptions`,
    );
  }
});

test('registry defaults: nofog off, fullbright gamma 1.0', () => {
  const nofog = doc.features.find((f) => f.id === 'nofog');
  assert.equal(nofog.default_enabled, false);
  const fullbright = doc.features.find((f) => f.id === 'fullbright');
  assert.equal(fullbright.state?.gamma, 1.0);
});

test('parity: every feature and category has a key in both lang files', () => {
  for (const f of doc.features) {
    for (const key of [`espectral.feature.${f.id}.name`, `espectral.feature.${f.id}.description`]) {
      assert.ok(key in en, `en missing ${key}`);
      assert.ok(key in es, `es missing ${key}`);
    }
  }
  for (const c of doc.categories) {
    assert.ok(`espectral.category.${c.id}` in en, `en missing category ${c.id}`);
    assert.ok(`espectral.category.${c.id}` in es, `es missing category ${c.id}`);
  }
});

test('parity: launcher i18n carries every feature name/desc in both languages', () => {
  const esSection = clientI18n.slice(0, clientI18n.indexOf('export const en'));
  const enSection = clientI18n.slice(clientI18n.indexOf('export const en'));
  for (const f of doc.features) {
    for (const key of [`client.feat.${f.id}.name`, `client.feat.${f.id}.desc`]) {
      assert.ok(esSection.includes(`'${key}'`), `es missing ${key}`);
      assert.ok(enSection.includes(`'${key}'`), `en missing ${key}`);
    }
  }
});

test('freshness: regenerating yields byte-identical files', () => {
  const { es: freshEs, en: freshEn } = buildLangMaps(doc);
  assert.equal(fs.readFileSync(DEST_EN, 'utf8'), stableStringify(freshEn));
  assert.equal(fs.readFileSync(DEST_ES, 'utf8'), stableStringify(freshEs));
  assert.equal(fs.readFileSync(DEST_REGISTRY, 'utf8'), fs.readFileSync(REGISTRY_SOURCE, 'utf8'));
});
