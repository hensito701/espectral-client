<script lang="ts">
  /* ==========================================================================
     TopChrome — Floating Horizon Glass Top Command Bar
     Layer 30 (z-index: 30 / --z-popover): Slim glass dock containing the
     brand wordmark, Command Palette trigger, Language/Theme toggles, and
     the PSN-style active account avatar chip with AccountPopover.
     ========================================================================== */

  import { onMount } from 'svelte';
  import GradientText from './GradientText.svelte';
  import ThemeToggle from './ThemeToggle.svelte';
  import LanguageToggle from './LanguageToggle.svelte';
  import AccountPopover from './AccountPopover.svelte';
  import MonogramTile from './MonogramTile.svelte';
  import { theme, resolveTheme } from '../lib/theme.svelte';
  import { t } from '../lib/i18n.svelte';
  import { updateState, installUpdate } from '../lib/updater.svelte';
  import { getAccounts, avatarUrl } from '../lib/api';
  import type { Account } from '../lib/types';

  interface Props {
    route?: string;
  }

  let { route = '#/' }: Props = $props();

  let activeAccount = $state<Account | null>(null);
  let popoverOpen = $state(false);

  // Signed-update CTA — this top-bar button is the single update surface
  // (the old banner component was removed). Nothing renders while idle, so an
  // up-to-date client sees the unchanged chrome.
  const showUpdate = $derived(
    updateState.status === 'available' ||
      updateState.status === 'downloading' ||
      updateState.status === 'installing' ||
      updateState.status === 'ready',
  );
  const updatePct = $derived(Math.round((updateState.progress ?? 0) * 100));
  // Label for the busy states: the download reports a percentage only when the
  // server sent a content-length; the install/relaunch phase has no progress.
  const updateBusyLabel = $derived(
    updateState.status === 'downloading'
      ? updatePct > 0
        ? t('update.progress', { pct: updatePct })
        : t('update.working')
      : t('update.installing'),
  );

  function toggleTheme(): void {
    const resolved = resolveTheme(theme.value);
    theme.set(resolved === 'dark' ? 'light' : 'dark');
  }

  function openCommandPalette(): void {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('horizon:open-command-palette'));
    }
  }

  function toggleAccountPopover(): void {
    popoverOpen = !popoverOpen;
  }

  async function loadActiveAccount(): Promise<void> {
    try {
      const list = await getAccounts();
      if (list && list.length > 0) {
        const sorted = [...list].sort(
          (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
        );
        activeAccount = sorted[0] ?? null;
      } else {
        activeAccount = null;
      }
    } catch {
      activeAccount = null;
    }
  }

  onMount(() => {
    void loadActiveAccount();

    const onAccountChanged = () => {
      void loadActiveAccount();
    };

    window.addEventListener('horizon:account-changed', onAccountChanged);
    return () => {
      window.removeEventListener('horizon:account-changed', onAccountChanged);
    };
  });

  // Re-fetch when navigating to/from #/account
  $effect(() => {
    if (route.startsWith('#/account')) {
      void loadActiveAccount();
    }
  });
</script>

<header class="top-chrome" role="banner">
  <!-- Left: Espectral Brand Mark -->
  <div class="top-chrome__brand">
    <a href="#/" class="brand-link" title={t('nav.home') || 'Inicio'}>
      <div class="brand-mark">
        <span class="brand-mark__glyph">✦</span>
      </div>
      <div class="brand-text">
        <GradientText as="span">
          <strong class="brand-wordmark">ESPECTRAL</strong>
        </GradientText>
        <span class="brand-sub">CLIENT</span>
      </div>
    </a>
  </div>

  <!-- Center: Hub Spacer -->
  <div class="top-chrome__center"></div>

  <!-- Right: Command trigger + Toggles + Account Chip -->
  <div class="top-chrome__actions">
    <!-- Update CTA (renders only when a signed update exists; idle shows nothing) -->
    {#if showUpdate}
      {#if updateState.status === 'available'}
        <button
          type="button"
          class="update-chip"
          onclick={installUpdate}
          title={t('update.badgeAria', { version: updateState.version })}
          aria-label={t('update.badgeAria', { version: updateState.version })}
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="update-chip__icon"
            aria-hidden="true"
          >
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
            <polyline points="7 10 12 15 17 10"></polyline>
            <line x1="12" y1="15" x2="12" y2="3"></line>
          </svg>
          <span class="update-chip__label">
            {t('update.badge', { version: updateState.version })}
          </span>
          <span class="update-chip__dot" aria-hidden="true"></span>
        </button>
      {:else}
        <button
          type="button"
          class="update-chip update-chip--busy"
          disabled
          role="progressbar"
          aria-valuenow={updatePct}
          aria-valuemin={0}
          aria-valuemax={100}
          title={updateBusyLabel}
          aria-label={updateBusyLabel}
        >
          <span class="update-chip__fill" style="width: {updatePct}%"></span>
          <span class="update-chip__label">{updateBusyLabel}</span>
        </button>
      {/if}
    {/if}

    <!-- Command Palette Trigger Button -->
    <button
      type="button"
      class="cmd-trigger"
      onclick={openCommandPalette}
      title={t('command.placeholder') || 'Buscar o ejecutar comando (/)'}
      aria-label={t('command.placeholder') || 'Abrir menú de comandos'}
    >
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        class="cmd-trigger__icon"
        aria-hidden="true"
      >
        <circle cx="11" cy="11" r="8"></circle>
        <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
      </svg>
      <span class="cmd-trigger__label">{t('command.action') || 'Comandos'}</span>
      <kbd class="cmd-trigger__kbd">/</kbd>
    </button>

    <!-- Language Toggle -->
    <LanguageToggle />

    <!-- Theme Toggle -->
    <ThemeToggle theme={theme.value} ontoggle={toggleTheme} />

    <!-- Account Chip Trigger -->
    <div class="account-trigger-wrap">
      <button
        type="button"
        class="account-chip"
        class:account-chip--active={popoverOpen}
        onclick={toggleAccountPopover}
        aria-haspopup="dialog"
        aria-expanded={popoverOpen}
        aria-label={t('topchrome.accountAria') || 'Menú de cuenta'}
        title={activeAccount ? activeAccount.username : t('nav.noAccount') || 'Sin cuenta'}
      >
        <div class="account-chip__avatar">
          {#if activeAccount}
            <MonogramTile
              name={activeAccount.username}
              hue={activeAccount.avatar_color ?? undefined}
              avatarUrl={activeAccount.has_avatar ? avatarUrl(activeAccount.username) : undefined}
              size={26}
              shape="circle"
            />
          {:else}
            <span class="account-chip__initial">+</span>
          {/if}
          <span
            class="account-chip__dot"
            class:account-chip__dot--msa={activeAccount?.token_kind === 'msa'}
            class:account-chip__dot--offline={activeAccount?.token_kind === 'offline'}
            class:account-chip__dot--empty={!activeAccount}
          ></span>
        </div>
        <div class="account-chip__body">
          <span
            class="account-chip__name"
            title={activeAccount ? activeAccount.username : t('nav.noAccount') || 'Sin cuenta'}
          >
            {activeAccount ? activeAccount.username : t('nav.noAccount') || 'Sin cuenta'}
          </span>
        </div>
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          class="account-chip__chevron"
          class:account-chip__chevron--rotated={popoverOpen}
          aria-hidden="true"
        >
          <polyline points="6 9 12 15 18 9"></polyline>
        </svg>
      </button>

      <!-- Account Popover Component -->
      <AccountPopover open={popoverOpen} onclose={() => (popoverOpen = false)} />
    </div>
  </div>
</header>

<style>
  .top-chrome {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: var(--topbar-height, 64px);
    z-index: var(--z-popover, 30);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 var(--space-6, 24px);
    background: var(--topbar-bg, rgba(8, 13, 26, 0.82));
    border-bottom: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    backdrop-filter: blur(var(--glass-blur, 16px));
    -webkit-backdrop-filter: blur(var(--glass-blur, 16px));
    user-select: none;
  }

  [data-theme='light'] .top-chrome {
    background: var(--topbar-bg, rgba(244, 246, 251, 0.88));
    border-bottom-color: var(--border, rgba(190, 204, 226, 0.65));
  }

  /* Brand Mark */
  .top-chrome__brand {
    display: flex;
    align-items: center;
  }

  .brand-link {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    text-decoration: none;
    color: var(--text, #e8ecf4);
    outline: none;
    border-radius: var(--radius-md, 0.625rem);
    padding: 4px 6px;
    transition: transform var(--dur-fast, 120ms) ease;
  }

  .brand-link:hover {
    transform: translateY(-1px);
  }

  .brand-link:focus-visible {
    box-shadow: var(--shadow-focus);
  }

  .brand-mark {
    width: 32px;
    height: 32px;
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.2), rgba(6, 182, 212, 0.2));
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.12));
    border-radius: var(--radius-md, 0.625rem);
    display: grid;
    place-items: center;
    box-shadow: 0 0 16px rgba(16, 185, 129, 0.25);
  }

  .brand-mark__glyph {
    font-size: 0.875rem;
    color: var(--accent, #10b981);
    filter: drop-shadow(0 0 4px var(--accent, #10b981));
  }

  .brand-text {
    display: flex;
    flex-direction: column;
    line-height: 1;
    gap: 2px;
  }

  .brand-wordmark {
    font-family: var(--font-display, inherit);
    font-size: var(--text-base, 1rem);
    font-weight: 800;
    letter-spacing: 0.08em;
  }

  .brand-sub {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.5625rem;
    font-weight: 600;
    color: var(--muted-strong, #8e9eb8);
    letter-spacing: 0.12em;
  }

  /* Center */
  .top-chrome__center {
    flex: 1;
  }

  /* Actions Cluster */
  .top-chrome__actions {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  /* Update CTA Chip — same accent semantics as Badge variant="ok". */
  .update-chip {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: 6px 12px;
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.14);
    border: 1px solid rgba(var(--accent-rgb, 16, 185, 129), 0.45);
    border-radius: var(--radius-md, 0.625rem);
    color: var(--accent, #10b981);
    cursor: pointer;
    font-family: var(--font-body, inherit);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    overflow: hidden;
    outline: none;
    transition: all var(--dur-fast, 120ms) ease;
  }

  /* data-theme lives on <html>, above this component's markup — Svelte's
     scoped-CSS analysis prunes a plain descendant selector as unused (that is
     how the dead density CSS shipped), so the ancestor must be :global(). */
  :global([data-theme='light']) .update-chip {
    color: var(--accent-ink, #047857);
  }

  .update-chip:not(:disabled):hover {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.22);
    border-color: var(--accent, #10b981);
  }

  .update-chip:not(:disabled):active {
    transform: translateY(1px);
  }

  .update-chip:focus-visible {
    box-shadow: var(--shadow-focus);
  }

  .update-chip:disabled {
    cursor: progress;
  }

  .update-chip__icon {
    position: relative;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .update-chip__label {
    position: relative;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }

  .update-chip__dot {
    position: relative;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    flex-shrink: 0;
    animation: update-chip-pulse 1.6s ease-in-out infinite;
  }

  /* The progress fill sits behind the label (which is position: relative). */
  .update-chip__fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.3);
    transition: width 160ms ease;
    pointer-events: none;
  }

  @keyframes update-chip-pulse {
    0%,
    100% {
      opacity: 1;
      box-shadow: 0 0 0 0 rgba(var(--accent-rgb, 16, 185, 129), 0.55);
    }
    50% {
      opacity: 0.55;
      box-shadow: 0 0 0 4px rgba(var(--accent-rgb, 16, 185, 129), 0);
    }
  }

  /* Command Palette Trigger */
  .cmd-trigger {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: 6px 12px;
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    color: var(--muted-strong, #8e9eb8);
    cursor: pointer;
    font-family: var(--font-body, inherit);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    outline: none;
    transition: all var(--dur-fast, 120ms) ease;
  }

  [data-theme='light'] .cmd-trigger {
    background: var(--surface-up, rgba(238, 242, 250, 0.9));
  }

  .cmd-trigger:hover {
    color: var(--text, #e8ecf4);
    border-color: var(--accent, #10b981);
    background: var(--surface-solid, rgba(255, 255, 255, 0.06));
  }

  .cmd-trigger:focus-visible {
    box-shadow: var(--shadow-focus);
  }

  .cmd-trigger__icon {
    width: 14px;
    height: 14px;
  }

  .cmd-trigger__label {
    letter-spacing: 0.02em;
  }

  .cmd-trigger__kbd {
    display: inline-block;
    padding: 1px 6px;
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.625rem;
    font-weight: 700;
    line-height: 1.2;
    color: var(--accent, #10b981);
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: var(--radius-xs, 0.25rem);
  }

  /* Account Chip Trigger */
  .account-trigger-wrap {
    position: relative;
  }

  .account-chip {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: 4px 10px 4px 4px;
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    color: var(--text, #e8ecf4);
    cursor: pointer;
    outline: none;
    transition: all var(--dur-fast, 120ms) ease;
  }

  [data-theme='light'] .account-chip {
    background: var(--surface-up, rgba(238, 242, 250, 0.9));
  }

  .account-chip:hover,
  .account-chip--active {
    border-color: var(--accent, #10b981);
    background: var(--surface-solid, rgba(255, 255, 255, 0.08));
  }

  .account-chip:focus-visible {
    box-shadow: var(--shadow-focus);
  }

  .account-chip__avatar {
    position: relative;
    width: 28px;
    height: 28px;
    background: linear-gradient(135deg, var(--surface-up-solid, #161e36), var(--surface-solid, #0d1222));
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-pill, 9999px);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }

  .account-chip__initial {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.6875rem;
    color: var(--accent, #10b981);
  }

  .account-chip__dot {
    position: absolute;
    bottom: -1px;
    right: -1px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--surface-solid, #0d1222);
  }

  .account-chip__dot--msa {
    background: #00a4ef;
  }

  .account-chip__dot--offline {
    background: var(--mode-uhc, #10b981);
  }

  .account-chip__dot--empty {
    background: var(--muted-strong, #8e9eb8);
  }

  .account-chip__body {
    display: flex;
    flex-direction: column;
    min-width: 0;
    max-width: 150px;
  }

  .account-chip__name {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .account-chip__chevron {
    width: 12px;
    height: 12px;
    color: var(--muted-strong, #8e9eb8);
    transition: transform var(--dur-fast, 120ms) ease;
  }

  .account-chip__chevron--rotated {
    transform: rotate(180deg);
  }
</style>
