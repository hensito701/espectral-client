/* ==========================================================================
   Formatting helpers. Words are language-aware via the i18n store (t() reads
   the reactive lang, so calls from templates / $derived re-render on toggle).
   ========================================================================== */

import { t, lang } from './i18n.svelte';

function trimZero(s: string): string {
  return s.replace(/\.0$/, '');
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '—';
  if (n < 1024) return `${Math.round(n)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = n;
  let unit = -1;
  do {
    value /= 1024;
    unit += 1;
  } while (value >= 1024 && unit < units.length - 1);
  const num = value >= 100 ? String(Math.round(value)) : trimZero(value.toFixed(1));
  return `${num} ${units[unit]}`;
}

export function formatPercent(part: number, total: number): string {
  if (!Number.isFinite(total) || total <= 0) return '0 %';
  const pct = (part / total) * 100;
  if (pct >= 99.95) return '100 %';
  return `${pct >= 9.95 ? Math.round(pct) : pct.toFixed(1)} %`;
}

export function timeAgo(input: number | string | Date): string {
  const ts = input instanceof Date ? input.getTime() : typeof input === 'number' ? input : Date.parse(input);
  if (!Number.isFinite(ts)) return '—';
  const diff = Date.now() - ts;
  if (Math.abs(diff) < 5000) return t('time.now');
  const key = diff < 0 ? 'time.in' : 'time.ago';
  const s = Math.round(Math.abs(diff) / 1000);
  if (s < 60) return t(key, { n: s, unit: 's' });
  const m = Math.round(s / 60);
  if (m < 60) return t(key, { n: m, unit: 'min' });
  const h = Math.round(m / 60);
  if (h < 24) return t(key, { n: h, unit: 'h' });
  const d = Math.round(h / 24);
  if (d < 30) return t(key, { n: d, unit: 'd' });
  const mo = Math.round(d / 30);
  if (mo < 12) {
    return t(key, { n: mo, unit: mo === 1 ? t('time.unit.month.one') : t('time.unit.month.other') });
  }
  const y = Math.round(mo / 12);
  return t(key, { n: y, unit: y === 1 ? t('time.unit.year.one') : t('time.unit.year.other') });
}

export function memLabel(mb: number): string {
  if (!Number.isFinite(mb) || mb <= 0) return '—';
  if (mb >= 1024 && mb % 1024 === 0) return `${mb / 1024} GB`;
  if (mb >= 1024) return `${trimZero((mb / 1024).toFixed(1))} GB`;
  return `${Math.round(mb)} MB`;
}

export function formatClock(input: number | string | Date): string {
  const d = input instanceof Date ? input : new Date(input);
  return Number.isNaN(d.getTime())
    ? '—'
    : d.toLocaleTimeString(lang.value === 'en' ? 'en-US' : 'es-ES', { hour12: false });
}
