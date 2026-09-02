/**
 * Discord OAuth2 & identity routes.
 *
 * Endpoints:
 * - GET  /api/discord/auth/start  -> { url: string }
 * - GET  /api/discord/callback    -> exchange code, upsert user, set session cookie, return completion page/redirect
 * - GET  /api/discord/me          -> { user: { id, name, avatar } | null }
 * - POST /api/discord/logout      -> clear session & cookie, { ok: true }
 */
import * as discordAuth from '../discordAuth.mjs';
import { httpError } from '../error.mjs';
/**
 * HTML completion template shown in external browser popups after OAuth callback.
 */
function renderAuthSuccessHtml(user) {
  const safeName = (user?.name || 'Usuario')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');

  return `<!DOCTYPE html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Espectral — Discord Login</title>
  <style>
    :root {
      --bg: #060a14;
      --card-bg: rgba(15, 23, 42, 0.88);
      --blurple: #5865f2;
      --blurple-glow: rgba(88, 101, 242, 0.35);
      --text: #e8ecf4;
      --text-muted: #94a3b8;
      --border: rgba(88, 101, 242, 0.3);
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      padding: 24px;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      background: var(--bg);
      color: var(--text);
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      text-align: center;
    }
    .card {
      max-width: 420px;
      width: 100%;
      background: var(--card-bg);
      border: 1px solid var(--border);
      border-radius: 20px;
      padding: 40px 32px;
      box-shadow: 0 20px 50px rgba(0, 0, 0, 0.6), 0 0 30px var(--blurple-glow);
      backdrop-filter: blur(16px);
    }
    .avatar {
      width: 72px;
      height: 72px;
      border-radius: 50%;
      border: 2px solid var(--blurple);
      box-shadow: 0 0 16px var(--blurple-glow);
      margin: 0 auto 16px auto;
      display: block;
      object-fit: cover;
      background: #0f172a;
    }
    .badge {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 4px 12px;
      border-radius: 9999px;
      background: rgba(88, 101, 242, 0.15);
      border: 1px solid rgba(88, 101, 242, 0.4);
      color: #a5b4fc;
      font-size: 12px;
      font-weight: 600;
      margin-bottom: 12px;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }
    h1 {
      margin: 0 0 8px 0;
      font-size: 22px;
      font-weight: 700;
      color: #fff;
    }
    p {
      margin: 0 0 20px 0;
      font-size: 14px;
      color: var(--text-muted);
      line-height: 1.5;
    }
    .hint {
      font-size: 12px;
      color: #64748b;
    }
  </style>
  <script>
    try {
      if (window.opener) {
        window.opener.postMessage({
          type: 'discord-auth-success',
          user: ${JSON.stringify(user)}
        }, '*');
      }
    } catch (e) {}

    // Auto-close popup after a short delay
    setTimeout(function() {
      try { window.close(); } catch (e) {}
    }, 1400);
  </script>
</head>
<body>
  <div class="card">
    ${user?.avatar ? `<img src="${user.avatar}" alt="Avatar" class="avatar" />` : ''}
    <div class="badge">Identidad Espectral</div>
    <h1>¡Hola, ${safeName}!</h1>
    <p>Tu cuenta de Discord se ha vinculado correctamente con Espectral Client.</p>
    <div class="hint">Ya puedes volver a Espectral Client — la sesión se completará sola.</div>
  </div>
</body>
</html>`;
}

export async function register(app) {
  /**
   * GET /api/discord/auth/start
   * Generate Discord OAuth2 authorization URL.
   */
  app.get('/api/discord/auth/start', async () => {
    return discordAuth.getAuthStartUrl();
  });

  /**
   * GET /api/discord/claim?state=<state>
   * One-time claim of the session created by the callback for an OAuth flow
   * this app started (state from /auth/start). Poll while the user completes
   * login in the system browser: pending -> ok(user) | invalid.
   */
  app.get('/api/discord/claim', async (req) => {
    const urlObj = new URL(req.url, 'http://localhost');
    const state = urlObj.searchParams.get('state') ?? '';
    return discordAuth.claimSessionUser(state);
  });

  /**
   * GET /api/discord/callback
   * OAuth2 authorization code exchange callback.
   */
  app.get('/api/discord/callback', async (req, res) => {
    const urlObj = new URL(req.url, 'http://localhost');
    const query = Object.fromEntries(urlObj.searchParams.entries());

    let result;
    try {
      result = await discordAuth.handleCallback(query);
    } catch (err) {
      // If authorization failed or error returned, render error HTML or throw
      if (err.status && err.code) {
        res.writeHead(err.status, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end(`<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Error de autenticación</title>
<style>body{background:#060a14;color:#ef4444;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;text-align:center;}h2{margin-bottom:8px;}p{color:#94a3b8;}</style>
</head>
<body><div><h2>Error de vinculación con Discord</h2><p>${err.message}</p></div></body>
</html>`);
        return;
      }
      throw err;
    }

    const { sessionId, user } = result;
    const cookieHeader = discordAuth.createSessionCookie(sessionId);

    // If client requested Tauri protocol redirect or custom deep link:
    if (query.redirect === 'tauri' || query.redirect === 'deep') {
      res.writeHead(302, {
        Location: 'tauri://localhost/#/',
        'Set-Cookie': cookieHeader,
      });
      res.end();
      return;
    }

    // Default: Return completion page with postMessage and cookie
    const html = renderAuthSuccessHtml(user);
    res.writeHead(200, {
      'Content-Type': 'text/html; charset=utf-8',
      'Content-Length': Buffer.byteLength(html),
      'Set-Cookie': cookieHeader,
    });
    res.end(html);
  });

  /**
   * GET /api/discord/me
   * Returns current authenticated user or { user: null }.
   * Supports ?mock=1 when ESPECTRAL_DISCORD_MOCK=1.
   */
  app.get('/api/discord/me', async (req) => {
    const urlObj = new URL(req.url, 'http://localhost');
    const mockRequested = urlObj.searchParams.get('mock') === '1';

    const cookieHeader = req.headers.cookie;
    const authHeader = req.headers.authorization;
    const customHeader = req.headers['x-discord-session'];

    const sessionId = discordAuth.extractSessionId(cookieHeader, authHeader, customHeader);
    const user = discordAuth.getSessionUser(sessionId, mockRequested);

    return { user };
  });

  /**
   * POST /api/discord/logout
   * Destroys current session and clears the session cookie.
   */
  app.post('/api/discord/logout', async (req, res) => {
    const cookieHeader = req.headers.cookie;
    const authHeader = req.headers.authorization;
    const customHeader = req.headers['x-discord-session'];

    const sessionId = discordAuth.extractSessionId(cookieHeader, authHeader, customHeader);
    if (sessionId) {
      discordAuth.destroySession(sessionId);
    }

    res.setHeader('Set-Cookie', discordAuth.clearSessionCookie());
    return { ok: true };
  });
}
