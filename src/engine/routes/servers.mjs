/**
 * Server status route: GET /api/servers -> { servers: ServerStatus[] }
 * (MAIN + 24/7 Meetups, 150s server-side cache in engine/servers.mjs).
 */
import * as servers from '../servers.mjs';

export async function register(app) {
  app.get('/api/servers', async () => ({ servers: await servers.getAllServerStatuses() }));
}
