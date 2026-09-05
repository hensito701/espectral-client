<script lang="ts">
  /* ==========================================================================
     AccountPopover — PSN-Style Profile & Identity Switcher Popover
     Layer 30 (z-index: 30 / --z-popover): Floating glass card displaying the
     active account, quick-switch roster, offline account creator, and MSA shortcut.
     ========================================================================== */

  import { onMount } from 'svelte';
  import { getAccounts, setActiveAccount, createAccount, avatarUrl } from '../lib/api';
  import type { Account } from '../lib/types';
  import MonogramTile from './MonogramTile.svelte';
  import { t } from '../lib/i18n.svelte';
  import { timeAgo } from '../lib/format';
  import { scalePop } from '../lib/motion';

  interface Props {
    open: boolean;
    onclose: () => void;
  }

  let { open = false, onclose }: Props = $props();

  let accounts = $state<Account[]>([]);
  let activeUsername = $state<string>('');
  let loading = $state(false);
  let error = $state('');

  // Quick offline create input
  let newUsername = $state('');
  let creating = $state(false);
  let createError = $state('');

  let popoverEl: HTMLElement | null = $state(null);

  const isValidName = $derived(/^[A-Za-z0-9_]{2,16}$/.test(newUsername.trim()));

  const activeAccount = $derived(
    accounts.find((a) => a.username === activeUsername) || accounts[0] || null,
  );

  const switchList = $derived(
    accounts.filter((a) => a.username !== activeAccount?.username),
  );

  async function loadAccounts(): Promise<void> {
    loading = true;
    error = '';
    try {
      const list = await getAccounts();
      accounts = list;
      const sorted = [...list].sort(
        (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
      );
      if (sorted.length > 0) {
        activeUsername = sorted[0].username;
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function handleSwitch(username: string): Promise<void> {
    try {
      await setActiveAccount(username);
      activeUsername = username;
      await loadAccounts();
      if (typeof window !== 'undefined') {
        window.dispatchEvent(new CustomEvent('horizon:account-changed', { detail: { username } }));
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleCreateOffline(): Promise<void> {
    if (!isValidName) {
      createError = t('account.errorInvalidName') || '2-16 chars (A-Z, 0-9, _)';
      return;
    }
    creating = true;
    createError = '';
    try {
      const acc = await createAccount(newUsername.trim());
      newUsername = '';
      await setActiveAccount(acc.username);
      await loadAccounts();
      if (typeof window !== 'undefined') {
        window.dispatchEvent(new CustomEvent('horizon:account-changed', { detail: { username: acc.username } }));
      }
    } catch (e) {
      createError = e instanceof Error ? e.message : String(e);
    } finally {
      creating = false;
    }
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    }
  }

  function navigateTo(hash: string): void {
    if (typeof window !== 'undefined') {
      window.location.hash = hash;
    }
    onclose();
  }

  // Reload accounts whenever popover opens
  $effect(() => {
    if (open) {
      void loadAccounts();
    }
  });

  // Outside click & global escape listener
  onMount(() => {
    const onWindowClick = (e: MouseEvent) => {
      if (!open) return;
      const target = e.target as Node | null;
      if (popoverEl && !popoverEl.contains(target)) {
        onclose();
      }
    };

    const onWindowKey = (e: KeyboardEvent) => {
      if (!open) return;
      if (e.key === 'Escape') {
        onclose();
      }
    };

    const onAccountChanged = () => {
      void loadAccounts();
    };

    window.addEventListener('mousedown', onWindowClick, true);
    window.addEventListener('keydown', onWindowKey, true);
    window.addEventListener('horizon:account-changed', onAccountChanged);

    return () => {
      window.removeEventListener('mousedown', onWindowClick, true);
      window.removeEventListener('keydown', onWindowKey, true);
      window.removeEventListener('horizon:account-changed', onAccountChanged);
    };
  });
</script>

{#if open}
  <div
    class="account-popover"
    bind:this={popoverEl}
    role="dialog"
    aria-modal="true"
    aria-label={t('account.profileTitle') || 'Perfiles de Jugador'}
    tabindex="-1"
    onkeydown={handleKeyDown}
    in:scalePop={{ start: 0.95, duration: 180 }}
    out:scalePop={{ start: 0.95, duration: 120 }}
  >
    <!-- Header -->
    <header class="popover-header">
      <div class="popover-header__title-wrap">
        <span class="popover-header__pixel-dot"></span>
        <h2 class="popover-header__title">{t('account.profileTitle') || 'Perfiles de Juego'}</h2>
      </div>
      <button
        type="button"
        class="popover-close-btn"
        onclick={onclose}
        aria-label={t('common.close') || 'Cerrar'}
        title="Esc"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="icon-sm">
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>
    </header>

    <!-- Body -->
    <div class="popover-body">
      {#if loading && accounts.length === 0}
        <div class="popover-loading">
          <span class="loading-spinner"></span>
          <span class="loading-text">{t('common.loading') || 'Cargando cuentas…'}</span>
        </div>
      {:else if activeAccount}
        <!-- Active Account Spotlight Card -->
        <div class="active-card">
          <div class="active-card__avatar">
            <MonogramTile
              name={activeAccount.username}
              hue={activeAccount.avatar_color ?? undefined}
              avatarUrl={activeAccount.has_avatar ? avatarUrl(activeAccount.username) : undefined}
              size={44}
              shape="circle"
            />
            <span
              class="avatar-status-dot"
              class:avatar-status-dot--msa={activeAccount.token_kind === 'msa'}
            ></span>
          </div>

          <div class="active-card__info">
            <div class="active-card__top">
              <span class="active-badge">{t('account.active') || 'ACTIVA'}</span>
              <span class="type-badge" class:type-badge--msa={activeAccount.token_kind === 'msa'}>
                {activeAccount.token_kind === 'msa' ? 'Microsoft' : 'Offline'}
              </span>
            </div>
            <div class="active-card__name" title={activeAccount.username}>
              {activeAccount.username}
            </div>
            {#if activeAccount.last_used}
              <div class="active-card__meta">
                {t('account.lastUsed') || 'Uso'}: {timeAgo(activeAccount.last_used)}
              </div>
            {/if}
          </div>
        </div>
      {:else}
        <!-- No accounts empty state -->
        <div class="no-accounts">
          <p class="no-accounts__msg">{t('account.noAccounts') || 'No hay cuentas registradas'}</p>
        </div>
      {/if}
      {#if error}
        <div class="form-error" role="alert">{error}</div>
      {/if}

      <!-- Switch Account List (if > 1 account) -->
      {#if switchList.length > 0}
        <div class="switch-section">
          <div class="section-label">
            <span>{t('account.switchTitle') || 'Cambiar de cuenta'}</span>
            <span class="section-count">{switchList.length}</span>
          </div>

          <div class="switch-list">
            {#each switchList as acc (acc.username)}
              <button
                type="button"
                class="switch-item"
                onclick={() => handleSwitch(acc.username)}
                title="{t('account.switchTo') || 'Usar'} {acc.username}"
              >
                <div class="switch-item__avatar">
                  <MonogramTile
                    name={acc.username}
                    hue={acc.avatar_color ?? undefined}
                    avatarUrl={acc.has_avatar ? avatarUrl(acc.username) : undefined}
                    size={32}
                    shape="circle"
                  />
                </div>
                <div class="switch-item__body">
                  <div class="switch-item__name">{acc.username}</div>
                  <div class="switch-item__type">
                    {acc.token_kind === 'msa' ? 'Microsoft' : 'Offline'}
                  </div>
                </div>
                <span class="switch-item__action">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="icon-xs">
                    <polyline points="9 18 15 12 9 6"></polyline>
                  </svg>
                </span>
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Quick Add Offline Account Form -->
      <div class="add-section">
        <label for="popover-offline-input" class="section-label">
          {t('account.createOffline') || '+ Nueva cuenta offline'}
        </label>
        <form
          class="add-form"
          onsubmit={(e) => {
            e.preventDefault();
            void handleCreateOffline();
          }}
        >
          <input
            id="popover-offline-input"
            type="text"
            class="add-input"
            placeholder={t('account.usernamePlaceholder') || 'Nombre de jugador…'}
            bind:value={newUsername}
            maxlength="16"
            autocomplete="off"
            spellcheck="false"
          />
          <button
            type="submit"
            class="add-btn"
            disabled={!isValidName || creating}
            title={t('account.add') || 'Añadir'}
          >
            {#if creating}
              <span class="loading-spinner-sm"></span>
            {:else}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" class="icon-sm">
                <line x1="12" y1="5" x2="12" y2="19"></line>
                <line x1="5" y1="12" x2="19" y2="12"></line>
              </svg>
            {/if}
          </button>
        </form>
        {#if createError}
          <div class="form-error">{createError}</div>
        {/if}
      </div>
    </div>

    <!-- Footer Actions -->
    <footer class="popover-footer">
      <button
        type="button"
        class="footer-btn footer-btn--msa"
        onclick={() => navigateTo('#/account')}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="icon-sm">
          <rect x="1" y="1" width="10" height="10" fill="#f25022" stroke="none"></rect>
          <rect x="13" y="1" width="10" height="10" fill="#7fba00" stroke="none"></rect>
          <rect x="1" y="13" width="10" height="10" fill="#00a4ef" stroke="none"></rect>
          <rect x="13" y="13" width="10" height="10" fill="#ffb900" stroke="none"></rect>
        </svg>
        <span>{t('account.msLogin') || 'Login Microsoft (MSA)'}</span>
      </button>

      <button
        type="button"
        class="footer-btn footer-btn--vault"
        onclick={() => navigateTo('#/account')}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="icon-sm">
          <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path>
          <circle cx="12" cy="7" r="4"></circle>
        </svg>
        <span>{t('account.manageAll') || 'Gestionar Cuentas'}</span>
      </button>
    </footer>
  </div>
{/if}

<style>
  .account-popover {
    position: fixed;
    top: calc(var(--topbar-height, 64px) + 8px);
    right: 16px;
    width: 320px;
    max-width: calc(100vw - 32px);
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-xl, 1.25rem);
    box-shadow: var(--shadow-lg), 0 0 24px rgba(0, 0, 0, 0.5);
    z-index: var(--z-popover, 30);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    color: var(--text, #e8ecf4);
    backdrop-filter: blur(var(--glass-blur, 16px));
    outline: none;
  }

  [data-theme='light'] .account-popover {
    background: var(--surface-solid, #ffffff);
    box-shadow: var(--shadow-lg), 0 4px 20px rgba(0, 0, 0, 0.12);
  }

  /* Header */
  .popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3, 12px) var(--space-4, 16px);
    border-bottom: 1px solid var(--border, rgba(40, 58, 96, 0.35));
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.4));
  }

  .popover-header__title-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .popover-header__pixel-dot {
    width: 6px;
    height: 6px;
    background: var(--accent, #10b981);
    box-shadow: 0 0 8px var(--accent, #10b981);
  }

  .popover-header__title {
    font-family: var(--font-display, inherit);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    margin: 0;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .popover-close-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm, 0.375rem);
    color: var(--muted, #8e9eb8);
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .popover-close-btn:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.08);
  }

  /* Body */
  .popover-body {
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    max-height: 420px;
    overflow-y: auto;
  }

  /* Active Spotlight Card */
  .active-card {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-3, 12px);
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.12), rgba(6, 182, 212, 0.06));
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: var(--radius-lg, 0.875rem);
  }

  .active-card__avatar {
    position: relative;
    width: 44px;
    height: 44px;
    flex-shrink: 0;
    background: linear-gradient(135deg, var(--accent, #10b981), var(--accent-cyan, #06b6d4));
    border-radius: var(--radius-pill, 9999px);
    display: grid;
    place-items: center;
    color: #060a14;
    font-weight: 800;
    font-size: 1.25rem;
    box-shadow: 0 4px 12px rgba(16, 185, 129, 0.25);
  }

  .avatar-letter {
    font-family: var(--font-mono-retro, monospace);
    font-size: 1rem;
    color: #ffffff;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.5);
  }

  .avatar-status-dot {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--mode-uhc, #10b981);
    border: 2px solid var(--surface-solid, #0d1222);
  }

  .avatar-status-dot--msa {
    background: #00a4ef;
  }

  .active-card__info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .active-card__top {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .active-badge {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.5625rem;
    padding: 1px 4px;
    background: var(--accent, #10b981);
    color: #060a14;
    border-radius: var(--radius-xs, 0.25rem);
    font-weight: 700;
  }

  .type-badge {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
  }

  .type-badge--msa {
    color: #38bdf8;
  }

  .active-card__name {
    font-weight: 700;
    font-size: var(--text-md, 0.9375rem);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .active-card__meta {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  /* Switch Section */
  .switch-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .section-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .section-count {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.625rem;
    padding: 1px 5px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-pill, 9999px);
  }

  .switch-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 140px;
    overflow-y: auto;
  }

  .switch-item {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: var(--space-2, 8px);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-md, 0.625rem);
    color: var(--text, #e8ecf4);
    cursor: pointer;
    text-align: left;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .switch-item:hover {
    background: rgba(255, 255, 255, 0.05);
    border-color: var(--border, rgba(40, 58, 96, 0.45));
  }

  .switch-item__avatar {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-pill, 9999px);
    background: var(--surface-up-solid, #161e36);
    display: grid;
    place-items: center;
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.75rem;
    color: var(--accent-cyan, #06b6d4);
  }

  .switch-item__body {
    flex: 1;
    min-width: 0;
  }

  .switch-item__name {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .switch-item__type {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
  }

  .switch-item__action {
    color: var(--muted, #8e9eb8);
  }

  .switch-item:hover .switch-item__action {
    color: var(--accent, #10b981);
  }

  /* Add Offline Section */
  .add-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .add-form {
    display: flex;
    gap: 6px;
  }

  .add-input {
    flex: 1;
    min-width: 0;
    background: var(--surface-up-solid, #161e36);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    padding: 6px 10px;
    color: var(--text, #e8ecf4);
    font-family: var(--font-body, inherit);
    font-size: var(--text-sm, 0.875rem);
    outline: none;
    transition: border-color var(--dur-fast, 120ms) ease;
  }

  [data-theme='light'] .add-input {
    background: var(--surface-up-solid, #e6ebf5);
  }

  .add-input:focus {
    border-color: var(--accent, #10b981);
  }

  .add-btn {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    background: var(--accent, #10b981);
    border: none;
    border-radius: var(--radius-md, 0.625rem);
    color: #060a14;
    cursor: pointer;
    transition: opacity var(--dur-fast, 120ms) ease, transform var(--dur-fast, 120ms) ease;
  }

  .add-btn:hover:not(:disabled) {
    opacity: 0.9;
    transform: scale(1.04);
  }

  .add-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .form-error {
    font-size: var(--text-xs, 0.75rem);
    color: var(--accent-red, #ef4444);
  }

  /* Footer */
  .popover-footer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-3, 12px) var(--space-4, 16px);
    border-top: 1px solid var(--border, rgba(40, 58, 96, 0.35));
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.4));
  }

  .footer-btn {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    width: 100%;
    padding: 8px 12px;
    border-radius: var(--radius-md, 0.625rem);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    cursor: pointer;
    border: 1px solid transparent;
    transition: all var(--dur-fast, 120ms) ease;
    text-align: left;
  }

  .footer-btn--msa {
    background: rgba(0, 164, 239, 0.1);
    border-color: rgba(0, 164, 239, 0.25);
    color: #38bdf8;
  }

  .footer-btn--msa:hover {
    background: rgba(0, 164, 239, 0.2);
    border-color: rgba(0, 164, 239, 0.5);
  }

  .footer-btn--vault {
    background: rgba(255, 255, 255, 0.04);
    border-color: var(--border, rgba(40, 58, 96, 0.45));
    color: var(--text, #e8ecf4);
  }

  .footer-btn--vault:hover {
    background: rgba(255, 255, 255, 0.08);
    border-color: var(--accent, #10b981);
    color: var(--accent, #10b981);
  }

  /* Icons */
  .icon-sm {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }

  .icon-xs {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .loading-spinner,
  .loading-spinner-sm {
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.2);
    border-top-color: var(--accent, #10b981);
    animation: spin 0.8s linear infinite;
  }

  .loading-spinner {
    width: 20px;
    height: 20px;
  }

  .loading-spinner-sm {
    width: 14px;
    height: 14px;
    border-color: rgba(0, 0, 0, 0.2);
    border-top-color: #060a14;
  }

  .popover-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2, 8px);
    padding: var(--space-6, 24px) 0;
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
  }

  .no-accounts {
    padding: var(--space-4, 16px) 0;
    text-align: center;
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
