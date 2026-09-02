/**
 * API error types — own module so leaf modules (accounts, instances, routes)
 * never import the HTTP server (breaks the ESM cycle that made the old
 * server.mjs bootstrap deadlock: server -> register -> misc -> accounts ->
 * server while server.mjs was still mid-evaluation).
 *
 * Canonical error envelope (every failure the UI sees):
 *   { error: { code: 'UPPER_SNAKE', message: string } }
 * `code` is UPPER_SNAKE; `message` is human-readable (mapped through i18n on
 * the UI where possible, otherwise shown verbatim). HTTP status is carried on
 * the response status line, not in the body.
 */

const UPPER_SNAKE_RE = /^[A-Z][A-Z0-9_]*$/;

/**
 * Normalize a thrown value to `{ status, code, message }` for the response.
 * - 5xx errors without a code collapse to `INTERNAL_ERROR` (no stack/code
 *   leak for server faults); 4xx keep the caller-provided code or fall back
 *   to `ERROR`.
 * - `message` is always present (fallback: "Internal server error").
 */
export function normalizeError(err) {
  const status = Number.isInteger(err && err.status) ? err.status : 500;
  const rawCode = typeof (err && err.code) === 'string' ? err.code : null;
  const code = rawCode && UPPER_SNAKE_RE.test(rawCode)
    ? rawCode
    : status >= 500
      ? 'INTERNAL_ERROR'
      : rawCode ?? 'ERROR';
  // For 5xx: only surface INTERNAL_ERROR with a generic or the original
  // non-leaky message; if the thrown code was not UPPER_SNAKE, it has been
  // mapped above. For non-ApiError throws (>=500 default), same path.
  const message = (err && err.message) || 'Internal server error';
  return { status, code, message };
}

export class ApiError extends Error {
  constructor(status, code, message) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

/** Factory for throwing handler errors: `throw httpError(404, 'NOT_FOUND', '...')`. */
export function httpError(status, code, message) {
  return new ApiError(status, code, message);
}
