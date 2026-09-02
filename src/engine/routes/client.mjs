/**
 * routes/client.mjs — Espectral Client config endpoints (v1.0.0, Contract A):
 *   GET   /api/instances/:name/client -> { config, registry, supported }
 *   PATCH /api/instances/:name/client { features?, macros? } -> { config, errors }
 * Structural patch problems throw httpError 400 (serialized by the server);
 * per-feature reconciliation problems come back in the `errors` array.
 */
import { getClientInfo, patchClientConfig } from '../client.mjs';
import { httpError } from '../error.mjs';

export async function register(app) {
  // GET /api/instances/:name/client -> { config, registry, supported }
  app.get('/api/instances/:name/client', async (req, res, params) => {
    return await getClientInfo(params.name);
  });

  // PATCH /api/instances/:name/client { features?, macros? } -> { config, errors }
  app.patch('/api/instances/:name/client', async (req, res, params, body) => {
    if (body === null || body === undefined) {
      throw httpError(400, 'BAD_PATCH', 'request body is required');
    }
    return await patchClientConfig(params.name, body);
  });
}
