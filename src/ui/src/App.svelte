<script lang="ts">
  /* ==========================================================================
     Espectral Client — Horizon Glass Main Application Shell
     Root layout managing fixed viewport shell, z-order hierarchy,
     global ambient backdrop, TopChrome dock, hash-based router with fluid
     motion transitions, Command Palette, PiP Flight Deck, and Toasts.
     ========================================================================== */

  import AmbientBackdrop from './components/AmbientBackdrop.svelte';
  import TopChrome from './components/TopChrome.svelte';
  import CommandPalette from './components/CommandPalette.svelte';
  import PipLogDrawer from './components/PipLogDrawer.svelte';
  import Toast from './components/Toast.svelte';
  import LoginGate from './components/LoginGate.svelte';

  // Horizon Glass screens (Wave B/B2)
  import Home from './pages/Home.svelte';
  import InstanceHub from './pages/InstanceHub.svelte';
  import ServerRadar from './pages/ServerRadar.svelte';
  import VersionArmory from './pages/VersionArmory.svelte';
  import ImportCenter from './pages/ImportCenter.svelte';
  import ModCatalog from './pages/ModCatalog.svelte';
  import ClientSuite from './pages/ClientSuite.svelte';
  import AccountVault from './pages/AccountVault.svelte';
  import SettingsDashboard from './pages/SettingsDashboard.svelte';
  import DonationsPavilion from './pages/DonationsPavilion.svelte';

  import { initThemeEffect } from './lib/theme.svelte';
  import { initLangEffect } from './lib/i18n.svelte';
  import { startEngine } from './lib/tauri';
  import { flyY, fade } from './lib/motion';
  import { discordSession, initDiscordSession } from './lib/discord.svelte';
  import { startUpdatePolling } from './lib/updater.svelte';

  // Theme and language must be initialized from a component scope (module-scope
  initThemeEffect();
  initLangEffect();
  initDiscordSession();

  // Desktop shell: ensure the engine child process is running before the UI
  // issues its initial fetch requests.
  startEngine();

  // Desktop shell: poll for signed updates (no-op in the browser flow).
  // Non-blocking; the top-bar CTA renders only when an update exists.
  // C13: the first check is deferred 30 s (15 s HTTPS timeout) so it does
  // not contend with the first launch's boot; re-checks run every 6 h.
  startUpdatePolling();

  let route = $state(typeof window !== 'undefined' ? window.location.hash || '#/' : '#/');

  $effect(() => {
    const onHash = () => {
      route = window.location.hash || '#/';
    };
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  });

  // Extract clean path segments (stripping '#' and query strings '?tab=...')
  const cleanPath = $derived(route.replace(/^#/, '').split('?')[0] || '/');
  const parts = $derived(cleanPath.split('/').filter(Boolean));

  // Determine transition grouping key to trigger smooth view swap
  const routeGroup = $derived(parts.slice(0, 2).join('/') || 'home');

  function decodeName(raw: string): string {
    try {
      return decodeURIComponent(raw);
    } catch {
      return raw;
    }
  }
</script>

<div class="app-shell">
  <!-- Layer 0: Dynamic Ambient Mesh & Grid Backdrop -->
  <AmbientBackdrop />

  <!-- Layer 90: Discord OAuth Supporter Identity Gate -->
  {#if discordSession.shouldShowGate}
    <LoginGate />
  {/if}

  <!-- Layer 30: Fixed Floating Top Command Bar -->
  <TopChrome {route} />

  <!-- Layer 10: Scrollable Main Page Viewport -->
  <main class="page-viewport">
    {#key routeGroup}
      <div
        class="route-transition-wrap"
        in:flyY={{ y: 10, duration: 180 }}
        out:fade={{ duration: 90 }}
      >
        {#if parts.length === 0 || (parts[0] === 'instances' && !parts[1])}
          <!-- #/ or #/instances -> Home Screen -->
          <Home />
        {:else if parts[0] === 'instances' && parts[1]}
          <!-- #/instances/:name -> Instance Hub Screen -->
          <InstanceHub name={decodeName(parts[1])} />
        {:else if parts[0] === 'servers'}
          <!-- #/servers -> Multiplayer Server Radar -->
          <ServerRadar />
        {:else if (parts[0] === 'library' && parts[1] === 'versions') || parts[0] === 'versions'}
          <!-- #/library/versions or legacy #/versions -> Version Armory -->
          <VersionArmory />
        {:else if (parts[0] === 'library' && parts[1] === 'import') || parts[0] === 'import'}
          <!-- #/library/import or legacy #/import -> Import Center -->
          <ImportCenter />
        {:else if parts[0] === 'mods'}
          <!-- #/mods -> Mod Catalog & Presets -->
          <ModCatalog />
        {:else if parts[0] === 'client'}
          <!-- #/client -> Espectral In-Game Suite & Macros -->
          <ClientSuite />
        {:else if parts[0] === 'account'}
          <!-- #/account -> Account Vault -->
          <AccountVault />
        {:else if parts[0] === 'settings'}
          <!-- #/settings -> Settings Dashboard -->
          <SettingsDashboard />
        {:else if parts[0] === 'donaciones'}
          <!-- #/donaciones -> Donations Pavilion -->
          <DonationsPavilion />
        {:else}
          <!-- Fallback: Unknown route redirects to Home -->
          <Home />
        {/if}
      </div>
    {/key}
  </main>

  <!-- Layer 20: Picture-in-Picture Launch Log Flight Deck -->
  <PipLogDrawer />

  <!-- Layer 50: Universal Command Matrix Overlay -->
  <CommandPalette />

  <!-- Layer 60: Advancement-style Toast Host -->
  <Toast />
</div>

<style>
  :global(html, body) {
    margin: 0;
    padding: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background-color: var(--bg, #060a14);
    color: var(--text, #e8ecf4);
    font-family: var(--font-body, 'Space Grotesk', system-ui, -apple-system, sans-serif);
  }

  .app-shell {
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background-color: var(--bg, #060a14);
    color: var(--text, #e8ecf4);
  }

  .page-viewport {
    position: relative;
    flex: 1;
    width: 100%;
    height: 100%;
    padding-top: var(--topbar-height, 64px);
    overflow-y: auto;
    overflow-x: hidden;
    z-index: var(--z-page, 10);
    box-sizing: border-box;
  }

  .route-transition-wrap {
    width: 100%;
    min-height: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
  }
</style>
