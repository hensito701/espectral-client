#!/usr/bin/env node
/**
 * sync-suite-registry.mjs — copy the canonical Suite registry into the mod jar
 * sources and generate the mod's language files from it.
 *
 * Single source of truth: `src/engine/suite-registry.json` (frozen, owned by
 * the launcher). This script derives everything the mod ships:
 *   - `branding-mod/.../assets/espectral-menu/suite-registry.json` (verbatim copy)
 *   - `branding-mod/.../assets/espectral-menu/lang/en_us.json` (English strings)
 *   - `branding-mod/.../assets/espectral-menu/lang/es_es.json` (Spanish strings)
 *
 * The lang files hold exactly the registry-derived keys
 * (`espectral.category.<id>`, `espectral.feature.<id>.name|description`) plus
 * the fixed Suite UI strings below — no extras. Output is deterministic
 * (sorted keys, 2-space indent, trailing newline); the freshness test in
 * `tests/suite-registry.test.mjs` regenerates the same bytes.
 *
 * Run with the bundled node:
 *   "/usr/lib/Espectral Client/node-bin/node" scripts/sync-suite-registry.mjs
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const REGISTRY_SOURCE = path.join(REPO_ROOT, 'src/engine/suite-registry.json');
const MENU_ASSETS = path.join(REPO_ROOT, 'branding-mod/src/main/resources/assets/espectral-menu');
export const DEST_REGISTRY = path.join(MENU_ASSETS, 'suite-registry.json');
export const DEST_EN = path.join(MENU_ASSETS, 'lang/en_us.json');
export const DEST_ES = path.join(MENU_ASSETS, 'lang/es_es.json');

/**
 * Fixed Suite UI strings (ES/EN). Why here and not in the registry JSON: the
 * registry owns feature data (names, defaults, categories); these chrome
 * strings belong to the Suite screens and change with the UI, not the data.
 */
export const FIXED_STRINGS = {
  es: {
    'espectral.suite.title': 'ESPECTRAL SUITE',
    'espectral.suite.subtitle': 'Configura la suite sin salir del juego.',
    'espectral.suite.search.hint': 'Buscar función…',
    'espectral.suite.category.all': 'Todas',
    'espectral.suite.master.on': 'Suite: Activada',
    'espectral.suite.master.off': 'Suite: DESACTIVADA',
    'espectral.suite.state.on': 'Activada',
    'espectral.suite.state.off': 'Desactivada',
    'espectral.suite.state.suppressed': '· en pausa',
    'espectral.suite.toggle.narration': '%s, %s',
    'espectral.suite.button.reset': 'Reiniciar',
    'espectral.suite.button.support': 'Apoyar ♥',
    'espectral.suite.button.done': 'Listo',
    'espectral.suite.button.prev': '◀',
    'espectral.suite.button.next': '▶',
    'espectral.suite.page': 'Página %s/%s',
    'espectral.suite.empty': 'Nada coincide con la búsqueda.',
    'espectral.suite.reset.title': 'Reiniciar la Suite',
    'espectral.suite.reset.message': 'Se restauran los valores por defecto de todas las funciones. El interruptor de la Suite no cambia.',
    'espectral.suite.reset.confirm': 'Reiniciar',
    'espectral.suite.support.title': 'Apoyar a Espectral',
    'espectral.suite.support.message': 'Se abrirá %s en tu navegador.',
    'espectral.suite.support.confirm': 'Abrir',
    'espectral.common.cancel': 'Cancelar',
    'espectral.common.close': 'Cerrar',
    'espectral.narration.suite': 'Suite de Espectral: %s funciones, %s',
    'espectral.title.client': '★ Espectral Client…',
    'espectral.title.support': '♥ Apoyar',
    'espectral.pause.client': '★ Espectral Client…',
    'espectral.pause.support': '♥ Apoyar',
  },
  en: {
    'espectral.suite.title': 'ESPECTRAL SUITE',
    'espectral.suite.subtitle': 'Tune the suite without leaving the game.',
    'espectral.suite.search.hint': 'Search features…',
    'espectral.suite.category.all': 'All',
    'espectral.suite.master.on': 'Suite: On',
    'espectral.suite.master.off': 'Suite: OFF',
    'espectral.suite.state.on': 'Enabled',
    'espectral.suite.state.off': 'Disabled',
    'espectral.suite.state.suppressed': '· paused',
    'espectral.suite.toggle.narration': '%s, %s',
    'espectral.suite.button.reset': 'Reset',
    'espectral.suite.button.support': 'Support ♥',
    'espectral.suite.button.done': 'Done',
    'espectral.suite.button.prev': '◀',
    'espectral.suite.button.next': '▶',
    'espectral.suite.page': 'Page %s/%s',
    'espectral.suite.empty': 'Nothing matches the search.',
    'espectral.suite.reset.title': 'Reset the Suite',
    'espectral.suite.reset.message': 'Every feature goes back to its default. The Suite switch itself does not change.',
    'espectral.suite.reset.confirm': 'Reset',
    'espectral.suite.support.title': 'Support Espectral',
    'espectral.suite.support.message': '%s will open in your browser.',
    'espectral.suite.support.confirm': 'Open',
    'espectral.common.cancel': 'Cancel',
    'espectral.common.close': 'Close',
    'espectral.narration.suite': 'Espectral suite: %s features, %s',
    'espectral.title.client': '★ Espectral Client…',
    'espectral.title.support': '♥ Support',
    'espectral.pause.client': '★ Espectral Client…',
    'espectral.pause.support': '♥ Support',
  },
};

/** Build the per-language key maps from the parsed registry document. */
export function buildLangMaps(doc) {
  const es = {};
  const en = {};
  for (const c of doc.categories ?? []) {
    es[`espectral.category.${c.id}`] = c.name_es;
    en[`espectral.category.${c.id}`] = c.name_en;
  }
  for (const f of doc.features ?? []) {
    es[`espectral.feature.${f.id}.name`] = f.name_es;
    es[`espectral.feature.${f.id}.description`] = f.description_es;
    en[`espectral.feature.${f.id}.name`] = f.name_en;
    en[`espectral.feature.${f.id}.description`] = f.description_en;
  }
  Object.assign(es, FIXED_STRINGS.es);
  Object.assign(en, FIXED_STRINGS.en);
  return { es, en };
}

/** Deterministic serialization: keys sorted, 2-space indent, trailing newline. */
export function stableStringify(obj) {
  const sorted = Object.fromEntries(Object.keys(obj).sort().map((k) => [k, obj[k]]));
  return JSON.stringify(sorted, null, 2) + '\n';
}

/** Regenerate the three derived files; returns their paths. */
export function syncSuiteRegistry() {
  const sourceText = fs.readFileSync(REGISTRY_SOURCE, 'utf8');
  const doc = JSON.parse(sourceText);
  const { es, en } = buildLangMaps(doc);
  fs.mkdirSync(path.dirname(DEST_EN), { recursive: true });
  fs.writeFileSync(DEST_REGISTRY, sourceText.endsWith('\n') ? sourceText : sourceText + '\n', 'utf8');
  fs.writeFileSync(DEST_EN, stableStringify(en), 'utf8');
  fs.writeFileSync(DEST_ES, stableStringify(es), 'utf8');
  return [DEST_REGISTRY, DEST_EN, DEST_ES];
}

const invokedAsMain = process.argv[1] !== undefined
  && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsMain) {
  for (const p of syncSuiteRegistry()) console.log(`wrote ${path.relative(REPO_ROOT, p)}`);
}
