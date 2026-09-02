// @ts-nocheck — svelte 5 compiles .svelte.ts modules with a JS-only parser
// (analyze_module hardcodes typescript=false), so this file cannot contain TS
// syntax/annotations; tsc would flag implicit anys, hence the file-level opt-out.
import { API_BASE } from './sse';
import { isTauri } from './tauri';

/* ==========================================================================
   Espectral Horizon Glass — Discord Session & Identity Store
   Manages client identity authentication state, persistence, and OAuth gate.
   ========================================================================== */

const STORAGE_KEY = 'horizon:discord-session';
const MOCK_USER = {
  id: '718293847561829304',
  name: 'TestPlayer',
  avatar: 'https://cdn.discordapp.com/embed/avatars/0.png',
};

/**
 * Load initial session cache from localStorage synchronously.
 */
function loadCachedSession() {
  if (typeof window === 'undefined' || !window.localStorage) {
    return { status: 'loading', user: null, anonChosen: false };
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { status: 'loading', user: null, anonChosen: false };
    const parsed = JSON.parse(raw);
    return {
      status: parsed.status === 'authed' && parsed.user ? 'authed' : 'anon',
      user: parsed.user || null,
      anonChosen: !!parsed.anonChosen,
    };
  } catch {
    return { status: 'loading', user: null, anonChosen: false };
  }
}

/**
 * Save session state to localStorage.
 */
function saveSessionToStorage(state) {
  if (typeof window === 'undefined' || !window.localStorage) return;
  try {
    const payload = {
      status: state.status,
      user: state.user,
      anonChosen: state.anonChosen,
    };
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    /* storage quota / private mode fallback */
  }
}

const initial = loadCachedSession();

let status = $state(initial.status);
let user = $state(initial.user);
let anonChosen = $state(initial.anonChosen);
let isAuthenticating = $state(false);
let authError = $state(null);
let initialized = false;

/**
 * Derived reactive state for whether the full-screen LoginGate should be displayed.
 * Gate is shown whenever status is 'loading' or 'anon', and user has not chosen anonymous,
 * and user is not authenticated.
 */
const shouldShowGate = $derived(
  status !== 'authed' && !anonChosen
);

/**
 * Check Discord session with the engine (/api/discord/me).
 * Supports ?mock=1 fallback in dev environments.
 */
async function checkEngineSession(forceMock = false) {
  try {
    const urlParams = typeof window !== 'undefined' ? new URLSearchParams(window.location.search) : null;
    const isMockParam = urlParams?.get('mock') === '1' || forceMock;
    const url = `${API_BASE}/api/discord/me${isMockParam ? '?mock=1' : ''}`;

    const res = await fetch(url, {
      method: 'GET',
      credentials: 'include',
      headers: {
        'x-espectral-client': '1',
      },
    });

    if (!res.ok) {
      // Backend error -> fallback to anonymous
      if (status !== 'authed') {
        status = 'anon';
      }
      return null;
    }

    const data = await res.json();
    if (data && data.user) {
      status = 'authed';
      user = data.user;
      anonChosen = false;
      saveSessionToStorage({ status, user, anonChosen });
      return data.user;
    } else {
      // No active session in engine
      if (status === 'authed') {
        status = 'anon';
        user = null;
      } else if (status === 'loading') {
        status = 'anon';
      }
      saveSessionToStorage({ status, user, anonChosen });
      return null;
    }
  } catch {
    // Engine offline or network unreachable
    if (status === 'loading') {
      status = 'anon';
    }
    return null;
  }
}

/**
 * Initialize Discord session verification on boot.
 */
export function initDiscordSession() {
  if (initialized) return;
  initialized = true;

  // Revalidate session against engine
  void checkEngineSession();

  // Listen for OAuth callback completion messages from popup windows
  if (typeof window !== 'undefined') {
    window.addEventListener('message', (event) => {
      if (event.data && event.data.type === 'discord-auth-success') {
        if (event.data.user) {
          status = 'authed';
          user = event.data.user;
          anonChosen = false;
          isAuthenticating = false;
          authError = null;
          saveSessionToStorage({ status, user, anonChosen });
        } else {
          void checkEngineSession();
        }
      }
    });
  }
}

/**
 * Start the Discord OAuth2 login flow.
 * Opens the authorize URL in the SYSTEM browser via the Tauri shell plugin
 * (the WebView blocks window.open popups — the old popup flow left the gate
 * spinning forever), then polls the one-time state claim endpoint until the
 * browser completes the callback. The popup path remains as a browser-dev
 * fallback.
 */
export async function loginWithDiscord() {
  isAuthenticating = true;
  authError = null;

  try {
    const res = await fetch(`${API_BASE}/api/discord/auth/start`, {
      method: 'GET',
      credentials: 'include',
      headers: {
        'x-espectral-client': '1',
      },
    });

    if (!res.ok) {
      const errData = await res.json().catch(() => ({}));
      const msg = errData?.error?.message || 'Error starting Discord authentication';
      authError = msg;
      isAuthenticating = false;
      throw new Error(msg);
    }

    const data = await res.json();
    const authUrl = data.url;
    const authState = data.state;

    if (!authUrl) {
      throw new Error('No authorization URL received from engine');
    }

    // Open externally: system browser from the Tauri shell, popup in the
    // plain-browser build. A failure here is LOUD — a silent fallback to the
    // blocked popup just spins the gate forever.
    let opened = false;
    if (isTauri()) {
      try {
        await window.__TAURI_INTERNALS__.invoke('plugin:shell|open', { path: authUrl });
        opened = true;
      } catch {
        /* plugin/permission unavailable — try the popup below */
      }
    }
    if (!opened && typeof window !== 'undefined') {
      const width = 540;
      const height = 800;
      const left = window.screenX + (window.outerWidth - width) / 2;
      const top = window.screenY + (window.outerHeight - height) / 2;
      const popup = window.open(authUrl, 'discord-oauth', `width=${width},height=${height},left=${left},top=${top},scrollbars=yes,status=1`);
      opened = !!popup;
    }
    if (!opened) {
      const msg = `No se pudo abrir el navegador. Abre este enlace manualmente: ${authUrl}`;
      authError = msg;
      isAuthenticating = false;
      throw new Error(msg);
    }

    // Poll the one-time claim (state-keyed, cookie-independent) while the
    // user finishes login in the browser. 3 min budget.
    let attempts = 0;
    const maxAttempts = 120;
    const interval = setInterval(async () => {
      attempts++;
      if (attempts > maxAttempts || status === 'authed') {
        clearInterval(interval);
        isAuthenticating = false;
        return;
      }

      if (authState) {
        try {
          const claimRes = await fetch(`${API_BASE}/api/discord/claim?state=${encodeURIComponent(authState)}`, {
            headers: { 'x-espectral-client': '1' },
          });
          if (claimRes.ok) {
            const claim = await claimRes.json();
            if (claim?.status === 'ok' && claim.user) {
              clearInterval(interval);
              status = 'authed';
              user = claim.user;
              anonChosen = false;
              isAuthenticating = false;
              authError = null;
              saveSessionToStorage({ status, user, anonChosen });
              return;
            }
          }
        } catch {
          /* transient — keep polling */
        }
      }

      // Legacy fallback for the popup path (cookie-visible in same jar).
      const activeUser = await checkEngineSession();
      if (activeUser) {
        clearInterval(interval);
        isAuthenticating = false;
      }
    }, 1500);

    return { ok: true, url: authUrl };
  } catch (err) {
    isAuthenticating = false;
    authError = err instanceof Error ? err.message : String(err);
    throw err;
  }
}

/**
 * Continue without Discord account (anonymous mode).
 * Dismisses the login gate and allows full offline usage.
 */
export function continueAnonymous() {
  status = 'anon';
  anonChosen = true;
  isAuthenticating = false;
  authError = null;
  saveSessionToStorage({ status, user, anonChosen });
}

/**
 * Log out of Discord account.
 */
export async function logoutDiscord() {
  try {
    await fetch(`${API_BASE}/api/discord/logout`, {
      method: 'POST',
      credentials: 'include',
      headers: {
        'x-espectral-client': '1',
      },
    });
  } catch {
    /* ignore network errors on logout */
  }

  status = 'anon';
  user = null;
  anonChosen = false; // Allow gate or re-login if desired
  isAuthenticating = false;
  authError = null;
  saveSessionToStorage({ status, user, anonChosen });
}

/**
 * Testing helper: simulate authenticated state with mock user.
 */
export function setMockSession(mockUser = MOCK_USER) {
  status = 'authed';
  user = mockUser;
  anonChosen = false;
  isAuthenticating = false;
  authError = null;
  saveSessionToStorage({ status, user, anonChosen });
}

/**
 * Exported reactive session object.
 */
export const discordSession = {
  get status() {
    return status;
  },
  get user() {
    return user;
  },
  get anonChosen() {
    return anonChosen;
  },
  get isAuthenticating() {
    return isAuthenticating;
  },
  get authError() {
    return authError;
  },
  get shouldShowGate() {
    return shouldShowGate;
  },
};
