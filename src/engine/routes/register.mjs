/**
 * Route registry — dynamically imports every route module so a module owned by
 * a slice that has not landed yet degrades to a warning instead of failing boot.
 */
const ROUTES = ['misc', 'instances', 'servers', 'versions', 'mods', 'import', 'launch', 'mrpack', 'client', 'discord'];

export async function register(app) {
  for (const r of ROUTES) {
    try {
      const m = await import('./' + r + '.mjs');
      if (m.register) await m.register(app);
    } catch (e) {
      console.warn('[routes] ' + r + ' not loaded:', e.message);
    }
  }
}
