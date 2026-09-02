/**
 * routes/versions.mjs — GET /api/versions (cached manifest) and
 * GET /api/versions/:id/resolve (VersionInfo with the JDK-tier gate).
 * The resolver CLI lives in src/engine/resolver.mjs:
 *   node src/engine/resolver.mjs install --version 1.21.11 --loader fabric
 *        [--name <name>] [--memory <mb>]
 */
import { getVersionManifest, resolveVersion } from '../resolver.mjs';
import { httpError } from '../error.mjs';

export async function register(app) {
  // GET /api/versions -> VersionManifest (6h cache in data/versions/manifest.json)
  app.get('/api/versions', async () => {
    try {
      const m = await getVersionManifest();
      return {
        latest_release: m.latest_release,
        latest_snapshot: m.latest_snapshot,
        versions: m.versions,
      };
    } catch (err) {
      throw httpError(502, 'MANIFEST_FETCH_FAILED', `version manifest fetch failed: ${err?.message ?? err}`);
    }
  });

  // GET /api/versions/:id/resolve -> VersionInfo { id, java_major, supported, reason }
  app.get('/api/versions/:id/resolve', async (req, res, params) => {
    // params.id is already decoded once by the router — do not decode again.
    try {
      return await resolveVersion(params.id);
    } catch (err) {
      if (err?.code === 'version_not_found') {
        throw httpError(404, 'VERSION_NOT_FOUND', err.message);
      }
      throw httpError(502, 'VERSION_RESOLVE_FAILED', `version resolve failed: ${err?.message ?? err}`);
    }
  });
}
