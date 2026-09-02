/**
 * routes/mrpack.mjs — mrpack import endpoint:
 *   POST /api/instances/import-mrpack { path, memory_mb? } -> { summary }
 * The heavy lifting (zip parse, instance creation, background pack-file
 * install + overrides extraction) lives in ../mrpack.mjs (Slice C); this
 * module validates the request and bubbles its httpError codes through
 * server.mjs. Import progress/done stream as SSE import-progress /
 * import-done events (events.mjs registry).
 */
import { httpError } from '../error.mjs';

export async function register(app) {
  // POST /api/instances/import-mrpack { path, memory_mb? } -> { summary }
  app.post('/api/instances/import-mrpack', async (req, res, params, body) => {
    const payload = body && typeof body === 'object' ? body : {};
    const filePath = payload.path;
    if (typeof filePath !== 'string' || filePath.length === 0) {
      throw httpError(400, 'BAD_PATH', 'path is required and must be a non-empty string');
    }
    const memoryMb = payload.memory_mb;
    if (memoryMb !== undefined && memoryMb !== null && (!Number.isInteger(memoryMb) || memoryMb < 512)) {
      throw httpError(400, 'BAD_MEMORY', 'memory_mb must be an integer >= 512');
    }
    const mrpack = await import('../mrpack.mjs');
    return await mrpack.importMrpack({
      file: filePath,
      memory_mb: typeof memoryMb === 'number' ? memoryMb : null,
    });
  });
}
