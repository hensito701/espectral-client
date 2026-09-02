<!--
  ============================================================================
  AccountVault.svelte — Horizon Glass PSN Identity Switcher & Skin Pedestal
  Route: #/account
  ============================================================================
  Cinematic identity vault and profile pedestal. Features:
    - Hero Pedestal: Dynamic 3D avatar spotlight with ambient color shift,
      cryptographic signature, active status, last-played and created timestamps.
    - Account Roster Directory: Interactive account cards with instant switching,
      token badges (Microsoft/Offline/Lunar), and inline MSA sign-out confirmation.
    - Identity Forge: Instant offline account creation with validation and auto-switch.
    - Microsoft Device Flow (MSA): Polling loop honoring retry_after/pending,
      copy buttons for user_code and verification_uri, and Azure pending detection.
    - Advanced Azure Client ID: Collapsible configuration panel with get/set client ID.
    - Lunar Token Shield: Dedicated protection indicator and profile migration link.
-->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    getAccounts,
    createAccount,
    setActiveAccount,
    startMsLogin,
    pollMsLogin,
    logoutMsAccount,
    getMsClientId,
    setMsClientId,
    ApiError,
  } from '../lib/api';
  import type { Account, MsDeviceFlow } from '../lib/types';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import Field from '../components/Field.svelte';
  import GradientText from '../components/GradientText.svelte';
  import MonogramTile from '../components/MonogramTile.svelte';
  import { timeAgo } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { useCopy } from '../lib/useCopy.svelte';
  import { flyY, fade, scalePop } from '../lib/motion';

  // --- Accounts State ---
  let accounts = $state<Account[]>([]);
  let activeUsername = $state<string>('');
  let loading = $state<boolean>(true);
  let refreshing = $state<boolean>(false);
  let loadError = $state<string>('');

  // Active account derived
  const activeAccount = $derived<Account | null>(
    accounts.find((a) => a.username === activeUsername) || accounts[0] || null,
  );

  // Sorting: active first, then last_used desc
  const sortedAccounts = $derived.by(() => {
    return [...accounts].sort((a, b) => {
      if (a.username === activeUsername) return -1;
      if (b.username === activeUsername) return 1;
      const tA = a.last_used ? Date.parse(a.last_used) : 0;
      const tB = b.last_used ? Date.parse(b.last_used) : 0;
      return tB - tA;
    });
  });

  // --- Offline Create Form State ---
  let newOfflineUsername = $state<string>('');
  let creatingOffline = $state<boolean>(false);
  let offlineError = $state<string>('');

  const nameValid = $derived(/^[A-Za-z0-9_]{2,16}$/.test(newOfflineUsername.trim()));

  // --- Microsoft (MSA) Device Flow State ---
  let msFlow = $state<MsDeviceFlow | null>(null);
  let msPolling = $state<boolean>(false);
  let msError = $state<string>('');
  let msPending = $state<boolean>(false);
  let msTimer: ReturnType<typeof setInterval> | null = null;

  // Active tab in Identity Forge ('offline' | 'msa')
  let forgeTab = $state<'offline' | 'msa'>('offline');

  // Inline logout confirmation state (account username being confirmed for logout)
  let confirmingLogoutUser = $state<string | null>(null);
  let loggingOut = $state<boolean>(false);

  // --- Advanced Azure Client ID State ---
  let showAdvancedClientId = $state<boolean>(false);
  let clientIdValue = $state<string>('');
  let loadingClientId = $state<boolean>(false);
  let savingClientId = $state<boolean>(false);
  let clientIdError = $state<string>('');

  // Copy helpers
  const codeCopier = useCopy(2000);
  const uriCopier = useCopy(2000);
  const uuidCopier = useCopy(2000);

  // --- Dynamic Ambient Lighting Shift ---
  function updateAmbientForAccount(username: string | undefined): void {
    if (typeof document === 'undefined' || !username) return;
    let hash = 0;
    for (let i = 0; i < username.length; i++) {
      hash = (hash << 5) - hash + username.charCodeAt(i);
      hash |= 0;
    }
    const hue1 = Math.abs(hash % 360);
    const hue2 = (hue1 + 45) % 360;
    document.documentElement.style.setProperty('--ambient-1', `hsla(${hue1}, 70%, 50%, 0.14)`);
    document.documentElement.style.setProperty('--ambient-2', `hsla(${hue2}, 80%, 45%, 0.09)`);
  }

  // --- Account Loading & Selection ---
  async function loadAccounts(silent = false): Promise<void> {
    if (!silent) loading = true;
    else refreshing = true;
    loadError = '';
    try {
      const list = await getAccounts();
      accounts = list;
      const sorted = [...list].sort(
        (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
      );
      if (sorted.length > 0 && !activeUsername) {
        activeUsername = sorted[0].username;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorLoad', { error: loadError }) });
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  async function handleSetActive(username: string): Promise<void> {
    if (activeUsername === username) return;
    try {
      await setActiveAccount(username);
      activeUsername = username;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.switchSuccess', { name: username }) });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: msg }) });
    }
  }

  // --- Offline Create Action ---
  async function handleCreateOffline(): Promise<void> {
    if (!nameValid || creatingOffline) return;
    creatingOffline = true;
    offlineError = '';
    const name = newOfflineUsername.trim();
    try {
      const acc = await createAccount(name);
      await setActiveAccount(acc.username);
      activeUsername = acc.username;
      newOfflineUsername = '';
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: acc.username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.offlineSuccess', { name: acc.username }) });
    } catch (e) {
      offlineError = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: offlineError }) });
    } finally {
      creatingOffline = false;
    }
  }

  // --- Microsoft (MSA) Device Flow Actions ---
  function scheduleMsPoll(delaySec: number): void {
    if (msTimer) clearInterval(msTimer);
    msTimer = setInterval(pollMs, Math.max(delaySec, 3) * 1000);
  }

  function isMsaPendingError(e: unknown): boolean {
    return e instanceof ApiError && e.code === 'MSA_DEVICE_FAILED';
  }

  async function startMs(): Promise<void> {
    msError = '';
    msPending = false;
    try {
      const flow = await startMsLogin();
      msFlow = flow;
      msPolling = true;
      scheduleMsPoll(flow.interval);
    } catch (e) {
      if (isMsaPendingError(e)) {
        msPending = true;
      } else {
        msError = e instanceof Error ? e.message : String(e);
        pushToast({ kind: 'err', text: msError });
      }
    }
  }

  async function pollMs(): Promise<void> {
    if (!msFlow) return;
    try {
      const result = await pollMsLogin(msFlow.flow_id);
      if ('pending' in result && result.pending) {
        scheduleMsPoll(result.retry_after ?? msFlow.interval ?? 5);
        return;
      }
      // Account created successfully
      if (msTimer) {
        clearInterval(msTimer);
        msTimer = null;
      }
      const acc = result as Account;
      msFlow = null;
      msPolling = false;
      await setActiveAccount(acc.username);
      activeUsername = acc.username;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: acc.username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.msSuccess', { name: acc.username }) });
    } catch (e) {
      if (msTimer) {
        clearInterval(msTimer);
        msTimer = null;
      }
      msFlow = null;
      msPolling = false;
      if (isMsaPendingError(e)) {
        msPending = true;
      } else {
        msError = e instanceof Error ? e.message : String(e);
        pushToast({ kind: 'err', text: msError });
      }
    }
  }

  function cancelMs(): void {
    if (msTimer) {
      clearInterval(msTimer);
      msTimer = null;
    }
    msFlow = null;
    msPolling = false;
  }

  // --- Inline MSA Logout ---
  function promptLogout(username: string, e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingLogoutUser = username;
  }

  function cancelLogout(e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingLogoutUser = null;
  }

  async function confirmLogout(username: string, e?: MouseEvent): Promise<void> {
    e?.stopPropagation?.();
    loggingOut = true;
    try {
      await logoutMsAccount(username);
      confirmingLogoutUser = null;
      await loadAccounts(true);
      if (activeUsername === username) {
        activeUsername = accounts[0]?.username || '';
        if (activeUsername) {
          await setActiveAccount(activeUsername);
        }
      }
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: activeUsername } }),
        );
      }
      pushToast({ kind: 'ok', text: `Microsoft account ${username} signed out.` });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: msg }) });
    } finally {
      loggingOut = false;
    }
  }

  // --- Advanced Client ID Management ---
  async function loadClientId(): Promise<void> {
    loadingClientId = true;
    clientIdError = '';
    try {
      const data = await getMsClientId();
      clientIdValue = data.client_id || '';
    } catch (err) {
      clientIdError = err instanceof Error ? err.message : String(err);
    } finally {
      loadingClientId = false;
    }
  }

  async function handleSaveClientId(): Promise<void> {
    savingClientId = true;
    clientIdError = '';
    try {
      await setMsClientId(clientIdValue.trim());
      pushToast({ kind: 'ok', text: t('vault.clientIdSuccess') });
    } catch (err) {
      clientIdError = err instanceof Error ? err.message : String(err);
      pushToast({ kind: 'err', text: clientIdError });
    } finally {
      savingClientId = false;
    }
  }

  function toggleAdvancedClientId(): void {
    showAdvancedClientId = !showAdvancedClientId;
    if (showAdvancedClientId && !clientIdValue) {
      void loadClientId();
    }
  }

  function navigateTo(hash: string): void {
    if (typeof window !== 'undefined') {
      window.location.hash = hash;
    }
  }

  // Effect to sync ambient light with active account
  $effect(() => {
    updateAmbientForAccount(activeAccount?.username);
  });

  onMount(() => {
    void loadAccounts();
  });

  onDestroy(() => {
    if (msTimer) {
      clearInterval(msTimer);
      msTimer = null;
    }
  });
</script>

<svelte:head>
  <title>{t('vault.tag')}</title>
</svelte:head>

<div class="account-vault-page">
  <!-- Top Command Header -->
  <header class="vault-hero">
    <div class="vault-hero__meta">
      <div class="vault-hero__badge-row">
        <Badge variant="accent" size="sm" dot={true}>
          {t('vault.badge')}
        </Badge>
        <span class="vault-hero__count-pill">
          {t('vault.rosterCount', { count: accounts.length })}
        </span>
      </div>

      <h1 class="vault-hero__title">
        <GradientText variant="emerald" size="2xl">
          {t('vault.title')}
        </GradientText>
      </h1>

      <p class="vault-hero__subtitle">
        {t('vault.subtitle')}
      </p>
    </div>

    <div class="vault-hero__actions">
      <Btn
        variant="secondary"
        size="md"
        loading={refreshing}
        onclick={() => loadAccounts(true)}
        ariaLabel={t('vault.refresh')}
      >
        {#snippet icon()}
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67" />
          </svg>
        {/snippet}
        {refreshing ? t('vault.refreshing') : t('vault.refresh')}
      </Btn>
    </div>
  </header>

  <!-- Hero Identity Pedestal Spotlight -->
  <section class="pedestal-section" aria-label="Active Identity Spotlight">
    <div class="pedestal-card">
      <div class="pedestal-glow" aria-hidden="true"></div>

      <div class="pedestal-layout">
        <!-- 3D Avatar Stage & Ring -->
        <div class="pedestal-stage">
          <div class="pedestal-ring" aria-hidden="true"></div>
          <div class="pedestal-avatar-wrapper">
            <MonogramTile
              name={activeAccount?.username || '?'}
              size={104}
              shape="rounded"
              className="pedestal-avatar"
            />
          </div>
          <div class="pedestal-floor" aria-hidden="true"></div>
        </div>

        <!-- Pedestal Identity Details -->
        <div class="pedestal-info">
          <div class="pedestal-info__top">
            <Badge variant="ok" dot={true} size="md">
              {t('vault.heroActiveBadge')}
            </Badge>

            {#if activeAccount?.token_kind === 'msa'}
              <Badge variant="accent" size="md">
                {t('vault.cardKindMsa')}
              </Badge>
            {:else}
              <Badge variant="neutral" size="md">
                {t('vault.cardKindOffline')}
              </Badge>
            {/if}
          </div>

          <div class="pedestal-username" title={activeAccount?.username || '—'}>
            {activeAccount?.username || t('vault.heroNoActive')}
          </div>

          <p class="pedestal-signature">
            <span class="shield-dot">✦</span>
            {t('vault.heroSkinHint')}
          </p>

          <!-- Pedestal Metadata Grid -->
          {#if activeAccount}
            <div class="pedestal-meta-grid">
              <div class="meta-item">
                <span class="meta-item__label">{t('account.lastUsed')}</span>
                <span class="meta-item__val">
                  {activeAccount.last_used ? timeAgo(activeAccount.last_used) : t('vault.heroNever')}
                </span>
              </div>

              <div class="meta-item">
                <span class="meta-item__label">{t('account.created')}</span>
                <span class="meta-item__val">
                  {activeAccount.created_at ? timeAgo(activeAccount.created_at) : '—'}
                </span>
              </div>

              <div class="meta-item meta-item--uuid">
                <span class="meta-item__label">UUID</span>
                <div class="uuid-copy-wrap">
                  <code class="uuid-code" title={activeAccount.uuid}>
                    {activeAccount.uuid.slice(0, 14)}…
                  </code>
                  <button
                    type="button"
                    class="uuid-copy-btn"
                    onclick={() => uuidCopier.copy(activeAccount?.uuid || '')}
                    title={t('vault.heroCopyUuid')}
                    aria-label={t('vault.heroCopyUuid')}
                  >
                    {#if uuidCopier.copied}
                      <span class="text-ok font-mono">✓</span>
                    {:else}
                      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                      </svg>
                    {/if}
                  </button>
                </div>
              </div>
            </div>
          {:else}
            <p class="pedestal-prompt">{t('vault.heroSelectPrompt')}</p>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <!-- Main Split Workbench: Directory on Left, Identity Forge on Right -->
  <div class="vault-workbench">
    <!-- Left Column: Synchronized Roster Grid -->
    <section class="workbench-col workbench-col--roster" aria-label="Account Directory">
      <div class="section-head">
        <div class="section-head__title-group">
          <h2 class="section-head__title">{t('vault.rosterTitle')}</h2>
          <span class="section-head__sub">{t('vault.rosterSubtitle')}</span>
        </div>
        <Badge variant="neutral" size="sm">{accounts.length}</Badge>
      </div>

      {#if loading && accounts.length === 0}
        <div class="roster-loading">
          <span class="spinner"></span>
          <span>{t('vault.loading')}</span>
        </div>
      {:else if accounts.length === 0}
        <div class="roster-empty">
          <div class="roster-empty__icon">👤</div>
          <h3 class="roster-empty__title">{t('vault.rosterEmpty')}</h3>
          <p class="roster-empty__desc">{t('vault.rosterEmptyHint')}</p>
        </div>
      {:else}
        <div class="roster-grid">
          {#each sortedAccounts as acc (acc.username)}
            {@const isActive = acc.username === activeUsername}
            {@const isConfirmingLogout = confirmingLogoutUser === acc.username}

            <div
              class="account-card"
              class:account-card--active={isActive}
              tabindex="0"
              role="button"
              aria-pressed={isActive}
              onclick={() => handleSetActive(acc.username)}
              onkeydown={(e) => {
                if (e.target !== e.currentTarget) return;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  handleSetActive(acc.username);
                }
              }}
            >
              <div class="account-card__head">
                <div class="account-card__avatar-wrap">
                  <MonogramTile
                    name={acc.username}
                    size={46}
                    shape="rounded"
                  />
                  {#if isActive}
                    <span class="active-pulse-dot" title={t('vault.cardActive')}></span>
                  {/if}
                </div>

                <div class="account-card__title-wrap">
                  <div class="account-card__name-row">
                    <span class="account-card__name">{acc.username}</span>
                    {#if isActive}
                      <Badge variant="ok" size="sm">{t('vault.cardActive')}</Badge>
                    {/if}
                  </div>

                  <div class="account-card__tags">
                    {#if acc.token_kind === 'msa'}
                      <Badge variant="accent" size="sm">{t('vault.cardKindMsa')}</Badge>
                    {:else}
                      <Badge variant="neutral" size="sm">{t('vault.cardKindOffline')}</Badge>
                    {/if}
                    <code class="account-card__uuid" title={acc.uuid}>{acc.uuid.slice(0, 8)}…</code>
                  </div>
                </div>
              </div>

              <!-- Card Timestamps & Metadata -->
              <div class="account-card__footer">
                <div class="account-card__timestamps">
                  <span class="card-ts">
                    {t('vault.cardLastUsed', {
                      time: acc.last_used ? timeAgo(acc.last_used) : t('vault.cardNever'),
                    })}
                  </span>
                </div>

                <div class="account-card__actions">
                  {#if !isActive}
                    <Btn
                      variant="ghost"
                      size="sm"
                      onclick={(e) => {
                        e.stopPropagation();
                        handleSetActive(acc.username);
                      }}
                    >
                      {t('vault.cardSetActive')}
                    </Btn>
                  {/if}

                  {#if acc.token_kind === 'msa'}
                    <Btn
                      variant="danger"
                      size="sm"
                      onclick={(e) => promptLogout(acc.username, e)}
                    >
                      {t('vault.cardLogoutMsa')}
                    </Btn>
                  {/if}
                </div>
              </div>

              <!-- Inline Confirmation Sheet for MSA Sign-Out -->
              {#if isConfirmingLogout}
                <div
                  class="inline-confirm-overlay"
                  transition:fade={{ duration: 120 }}
                  onclick={(e) => e.stopPropagation()}
                  role="presentation"
                >
                  <p class="inline-confirm__text">
                    {t('vault.cardLogoutConfirm', { name: acc.username })}
                  </p>
                  <div class="inline-confirm__actions">
                    <Btn
                      variant="danger"
                      size="sm"
                      loading={loggingOut}
                      onclick={(e) => confirmLogout(acc.username, e)}
                    >
                      {t('vault.cardLogoutBtn')}
                    </Btn>
                    <Btn
                      variant="ghost"
                      size="sm"
                      disabled={loggingOut}
                      onclick={(e) => cancelLogout(e)}
                    >
                      {t('vault.cardLogoutCancel')}
                    </Btn>
                  </div>
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <!-- Right Column: Identity Forge & Advanced Controls -->
    <section class="workbench-col workbench-col--forge" aria-label="Identity Forge">
      <!-- Identity Forge Container -->
      <GlassCard title={t('vault.addTitle')} subtitle={t('vault.addSubtitle')} elevation="md">
        <!-- Segmented Tab Navigation -->
        <div class="forge-tabs">
          <button
            type="button"
            class="forge-tab"
            class:forge-tab--active={forgeTab === 'offline'}
            onclick={() => (forgeTab = 'offline')}
          >
            <span class="tab-icon">⚡</span>
            <span>{t('vault.tabOffline')}</span>
          </button>
          <button
            type="button"
            class="forge-tab"
            class:forge-tab--active={forgeTab === 'msa'}
            onclick={() => (forgeTab = 'msa')}
          >
            <span class="tab-icon">🌐</span>
            <span>{t('vault.tabMicrosoft')}</span>
          </button>
        </div>

        <!-- Tab 1: Offline Account Forge -->
        {#if forgeTab === 'offline'}
          <div class="forge-pane" transition:fade={{ duration: 120 }}>
            <p class="forge-desc">{t('vault.offlineDesc')}</p>

            <form
              class="offline-form"
              onsubmit={(e) => {
                e.preventDefault();
                handleCreateOffline();
              }}
            >
              <Field
                label={t('vault.offlineLabel')}
                hint={t('vault.offlineValidationHint')}
                error={offlineError}
                required={true}
              >
                <div class="input-with-monogram">
                  <div class="input-monogram-preview">
                    <MonogramTile
                      name={newOfflineUsername.trim() || '?'}
                      size={32}
                      shape="rounded"
                    />
                  </div>
                  <input
                    type="text"
                    class="input text-input"
                    bind:value={newOfflineUsername}
                    placeholder={t('vault.offlinePlaceholder')}
                    maxlength="16"
                    autocomplete="off"
                    spellcheck="false"
                  />
                </div>
              </Field>

              <div class="forge-submit-row">
                <Btn
                  variant="primary"
                  size="md"
                  type="submit"
                  disabled={!nameValid || creatingOffline}
                  loading={creatingOffline}
                  block={true}
                >
                  {#snippet icon()}
                    <span class="btn-icon">⚡</span>
                  {/snippet}
                  {creatingOffline ? t('vault.offlineCreating') : t('vault.offlineCreateBtn')}
                </Btn>
              </div>
            </form>
          </div>

        <!-- Tab 2: Microsoft OAuth Device Flow -->
        {:else}
          <div class="forge-pane" transition:fade={{ duration: 120 }}>
            <p class="forge-desc">{t('vault.msDesc')}</p>

            {#if msFlow}
              <div class="ms-device-flow-card" transition:scalePop={{ start: 0.96, duration: 160 }}>
                <!-- Step 1: Verification URI -->
                <div class="flow-step">
                  <div class="flow-step__head">
                    <span class="flow-step__number">1</span>
                    <span class="flow-step__label">{t('vault.msStep1')}</span>
                  </div>
                  <div class="flow-step__body">
                    <div class="uri-pill">
                      <a
                        href={msFlow.verification_uri}
                        target="_blank"
                        rel="noopener"
                        class="uri-link"
                      >
                        {msFlow.verification_uri}
                        <span class="external-icon">↗</span>
                      </a>
                      <div class="uri-actions">
                        <Btn
                          variant="ghost"
                          size="sm"
                          onclick={() => uriCopier.copy(msFlow?.verification_uri || '')}
                        >
                          {uriCopier.copied ? t('vault.msLinkCopied') : t('vault.msCopyLink')}
                        </Btn>
                      </div>
                    </div>
                  </div>
                </div>

                <!-- Step 2: User Code -->
                <div class="flow-step">
                  <div class="flow-step__head">
                    <span class="flow-step__number">2</span>
                    <span class="flow-step__label">{t('vault.msStep2')}</span>
                  </div>
                  <div class="flow-step__body">
                    <div class="code-banner">
                      <span class="code-display font-pixel">{msFlow.user_code}</span>
                      <Btn
                        variant="primary"
                        size="sm"
                        onclick={() => codeCopier.copy(msFlow?.user_code || '')}
                      >
                        {#snippet icon()}
                          <span>📋</span>
                        {/snippet}
                        {codeCopier.copied ? t('vault.msCodeCopied') : t('vault.msCopyCode')}
                      </Btn>
                    </div>
                  </div>
                </div>

                <!-- Waiting state pulse -->
                <div class="flow-waiting">
                  <span class="pulse-ring"></span>
                  <span class="flow-waiting__text">{t('vault.msWaiting')}</span>
                </div>

                <div class="flow-cancel">
                  <Btn variant="ghost" size="sm" onclick={cancelMs}>
                    {t('vault.msCancelBtn')}
                  </Btn>
                </div>
              </div>
            {:else}
              <div class="ms-start-box">
                <Btn
                  variant="primary"
                  size="lg"
                  block={true}
                  loading={msPolling}
                  onclick={startMs}
                >
                  {#snippet icon()}
                    <span class="btn-icon">🌐</span>
                  {/snippet}
                  {msPolling ? t('vault.msStarting') : t('vault.msStartBtn')}
                </Btn>
              </div>
            {/if}

            {#if msPending}
              <div class="ms-alert ms-alert--pending" transition:fade={{ duration: 140 }}>
                <div class="ms-alert__head">
                  <span class="alert-icon">⏳</span>
                  <span class="alert-title">{t('vault.msaPending')}</span>
                </div>
                <p class="alert-desc">{t('vault.msaPendingHint')}</p>
              </div>
            {:else if msError}
              <div class="ms-alert ms-alert--error" transition:fade={{ duration: 140 }}>
                <span class="alert-icon">⚠️</span>
                <span class="alert-desc">{msError}</span>
              </div>
            {/if}
          </div>
        {/if}
      </GlassCard>

      <!-- Advanced Azure Client ID Configuration Accordion -->
      <div class="advanced-section">
        <button
          type="button"
          class="advanced-toggle"
          onclick={toggleAdvancedClientId}
          aria-expanded={showAdvancedClientId}
        >
          <div class="advanced-toggle__label">
            <span class="advanced-icon">⚙️</span>
            <span>{t('vault.clientIdToggle')}</span>
          </div>
          <span class="advanced-caret" class:advanced-caret--open={showAdvancedClientId}>▼</span>
        </button>

        {#if showAdvancedClientId}
          <div class="advanced-pane" transition:flyY={{ y: -8, duration: 150 }}>
            <Field
              label={t('vault.clientIdLabel')}
              hint={t('vault.clientIdHint')}
              error={clientIdError}
            >
              <input
                type="text"
                class="input text-input"
                bind:value={clientIdValue}
                placeholder={t('vault.clientIdPlaceholder')}
                disabled={loadingClientId || savingClientId}
              />
            </Field>

            <div class="advanced-actions">
              <Btn
                variant="secondary"
                size="sm"
                loading={savingClientId}
                disabled={loadingClientId}
                onclick={handleSaveClientId}
              >
                {savingClientId ? t('vault.clientIdSaving') : t('vault.clientIdSave')}
              </Btn>
              <Btn
                variant="ghost"
                size="sm"
                disabled={savingClientId || loadingClientId}
                onclick={() => {
                  clientIdValue = '';
                  void handleSaveClientId();
                }}
              >
                {t('vault.clientIdReset')}
              </Btn>
            </div>
          </div>
        {/if}
      </div>

      <!-- Lunar Token Shield & Migration Card -->
      <div class="lunar-shield-card">
        <div class="lunar-shield-card__head">
          <div class="lunar-shield-card__title-wrap">
            <span class="lunar-icon">🌙</span>
            <span class="lunar-title">{t('vault.lunarShieldTitle')}</span>
          </div>
          <Badge variant="purple" size="sm">{t('vault.lunarBadge')}</Badge>
        </div>

        <p class="lunar-shield-card__desc">
          {t('vault.lunarShieldDesc')}
        </p>

        <div class="lunar-shield-card__action">
          <Btn
            variant="ghost"
            size="sm"
            onclick={() => navigateTo('#/library/import')}
          >
            {#snippet icon()}
              <span>📦</span>
            {/snippet}
            {t('vault.lunarImportCta')}
          </Btn>
        </div>
      </div>
    </section>
  </div>
</div>

<style>
  /* ==========================================================================
     AccountVault Horizon Glass Styles
     ========================================================================== */

  .account-vault-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-6, 24px);
    padding: var(--space-6, 24px) var(--space-8, 32px) var(--space-12, 48px);
    max-width: 1400px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* --- Top Hero Header --- */
  .vault-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    flex-wrap: wrap;
  }

  .vault-hero__meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .vault-hero__badge-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .vault-hero__count-pill {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    font-weight: 500;
  }

  .vault-hero__title {
    margin: 0;
    line-height: 1.1;
  }

  .vault-hero__subtitle {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    max-width: 680px;
  }

  .vault-hero__actions {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  /* --- Hero Pedestal Spotlight Section --- */
  .pedestal-section {
    position: relative;
    width: 100%;
  }

  .pedestal-card {
    position: relative;
    background: linear-gradient(135deg, rgba(16, 24, 40, 0.75), rgba(8, 14, 26, 0.85));
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-xl, 16px);
    padding: var(--space-6, 24px) var(--space-8, 32px);
    overflow: hidden;
    box-shadow: 0 16px 36px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }

  .pedestal-glow {
    position: absolute;
    top: -50%;
    left: 20%;
    width: 600px;
    height: 300px;
    background: radial-gradient(ellipse at center, var(--ambient-1, rgba(16, 185, 129, 0.15)), transparent 70%);
    pointer-events: none;
    z-index: 0;
  }

  .pedestal-layout {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: var(--space-8, 32px);
    flex-wrap: wrap;
  }

  /* 3D Pedestal Stage */
  .pedestal-stage {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-width: 140px;
  }

  .pedestal-avatar-wrapper {
    position: relative;
    z-index: 2;
    filter: drop-shadow(0 12px 24px rgba(0, 0, 0, 0.6));
    transition: transform var(--dur-med, 200ms) var(--ease-spring-soft);
  }

  .pedestal-avatar-wrapper:hover {
    transform: translateY(-4px) scale(1.02);
  }

  .pedestal-ring {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 140px;
    height: 140px;
    border-radius: 50%;
    border: 1px dashed var(--accent, #10b981);
    opacity: 0.35;
    animation: ring-rotate 30s linear infinite;
    pointer-events: none;
  }

  @keyframes ring-rotate {
    from {
      transform: translate(-50%, -50%) rotate(0deg);
    }
    to {
      transform: translate(-50%, -50%) rotate(360deg);
    }
  }

  .pedestal-floor {
    position: absolute;
    bottom: -10px;
    width: 120px;
    height: 16px;
    background: radial-gradient(ellipse at center, rgba(16, 185, 129, 0.4), transparent 70%);
    border-radius: 50%;
    filter: blur(4px);
  }

  /* Pedestal Identity Details */
  .pedestal-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    flex: 1;
    min-width: 280px;
  }

  .pedestal-info__top {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .pedestal-username {
    font-size: var(--text-2xl, 1.75rem);
    font-weight: 700;
    color: var(--text, #f1f5f9);
    letter-spacing: -0.02em;
    line-height: 1.2;
  }

  .pedestal-signature {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    display: flex;
    align-items: center;
    gap: var(--space-1, 4px);
  }

  .shield-dot {
    color: var(--accent, #10b981);
  }

  .pedestal-meta-grid {
    display: flex;
    align-items: center;
    gap: var(--space-6, 24px);
    margin-top: var(--space-3, 12px);
    flex-wrap: wrap;
  }

  .meta-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .meta-item__label {
    font-size: var(--text-xs, 0.75rem);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim, #5c6b84);
    font-weight: 600;
  }

  .meta-item__val {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #f1f5f9);
    font-weight: 500;
  }

  .meta-item--uuid {
    min-width: 180px;
  }

  .uuid-copy-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .uuid-code {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    background: rgba(0, 0, 0, 0.3);
    padding: 2px 6px;
    border-radius: var(--radius-sm, 4px);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }

  .uuid-copy-btn {
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm, 4px);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color var(--dur-fast, 120ms), background var(--dur-fast, 120ms);
  }

  .uuid-copy-btn:hover {
    color: var(--text, #f1f5f9);
    background: rgba(255, 255, 255, 0.08);
  }

  .pedestal-prompt {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    margin: 0;
  }

  /* --- Main Split Workbench --- */
  .vault-workbench {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(360px, 1fr);
    gap: var(--space-6, 24px);
    align-items: start;
  }

  @media (max-width: 1024px) {
    .vault-workbench {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .workbench-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .section-head__title-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .section-head__title {
    margin: 0;
    font-size: var(--text-lg, 1.125rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .section-head__sub {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  /* --- Roster Grid Cards --- */
  .roster-loading,
  .roster-empty {
    padding: var(--space-8, 32px);
    text-align: center;
    background: rgba(16, 24, 40, 0.4);
    border: 1px dashed var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2, 8px);
    color: var(--text-muted, #8b9bb4);
  }

  .roster-empty__icon {
    font-size: 2rem;
    opacity: 0.5;
  }

  .roster-empty__title {
    margin: 0;
    color: var(--text, #f1f5f9);
    font-size: var(--text-base, 1rem);
  }

  .roster-empty__desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    max-width: 360px;
  }

  .roster-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .account-card {
    position: relative;
    background: rgba(16, 24, 40, 0.55);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    cursor: pointer;
    outline: none;
    transition: transform var(--dur-fast, 120ms) var(--ease-spring-soft),
      border-color var(--dur-fast, 120ms),
      background var(--dur-fast, 120ms),
      box-shadow var(--dur-fast, 120ms);
  }

  .account-card:hover {
    background: rgba(22, 34, 56, 0.7);
    border-color: rgba(255, 255, 255, 0.16);
    transform: translateY(-2px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  }

  .account-card:focus-visible {
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 0 2px rgba(16, 185, 129, 0.4);
  }

  .account-card--active {
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.08), rgba(16, 24, 40, 0.7));
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 20px rgba(16, 185, 129, 0.12);
  }

  .account-card__head {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .account-card__avatar-wrap {
    position: relative;
  }

  .active-pulse-dot {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    box-shadow: 0 0 8px var(--accent, #10b981);
    border: 2px solid #0b1320;
  }

  .account-card__title-wrap {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }

  .account-card__name-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .account-card__name {
    font-size: var(--text-base, 1rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .account-card__tags {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .account-card__uuid {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-dim, #5c6b84);
  }

  .account-card__footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    padding-top: var(--space-2, 8px);
    margin-top: 2px;
  }

  .card-ts {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  .account-card__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* Inline Logout Confirmation Overlay */
  .inline-confirm-overlay {
    position: absolute;
    inset: 0;
    background: rgba(12, 18, 30, 0.95);
    backdrop-filter: blur(6px);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    z-index: 10;
  }

  .inline-confirm__text {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #f1f5f9);
    font-weight: 500;
  }

  .inline-confirm__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* --- Identity Forge Pane & Tabs --- */
  .forge-tabs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    background: rgba(0, 0, 0, 0.3);
    padding: 4px;
    border-radius: var(--radius-md, 8px);
    margin-bottom: var(--space-4, 16px);
  }

  .forge-tab {
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2, 8px);
    transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
  }

  .forge-tab:hover {
    color: var(--text, #f1f5f9);
  }

  .forge-tab--active {
    background: var(--surface-raised, rgba(255, 255, 255, 0.08));
    color: var(--text, #f1f5f9);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
  }

  .forge-pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .forge-desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    line-height: 1.4;
  }

  .offline-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .input-with-monogram {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .input-monogram-preview {
    flex-shrink: 0;
  }

  .text-input {
    width: 100%;
    box-sizing: border-box;
  }

  .forge-submit-row {
    margin-top: var(--space-2, 8px);
  }

  /* Microsoft Device Flow */
  .ms-device-flow-card {
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .flow-step {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .flow-step__head {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .flow-step__number {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    color: #060a14;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .flow-step__label {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .uri-pill {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
    background: rgba(0, 0, 0, 0.4);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    border-radius: var(--radius-sm, 6px);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }

  .uri-link {
    color: var(--accent, #10b981);
    font-size: var(--text-sm, 0.875rem);
    text-decoration: none;
    display: flex;
    align-items: center;
    gap: 4px;
    word-break: break-all;
  }

  .uri-link:hover {
    text-decoration: underline;
  }

  .code-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.15), rgba(6, 182, 212, 0.1));
    border: 1px solid var(--accent, #10b981);
    border-radius: var(--radius-md, 8px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
  }

  .code-display {
    font-size: var(--text-xl, 1.25rem);
    font-weight: 700;
    color: #fff;
    letter-spacing: 0.15em;
  }

  .flow-waiting {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-2, 8px) 0;
  }

  .pulse-ring {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    animation: pulse 1.5s infinite;
  }

  @keyframes pulse {
    0% {
      transform: scale(0.95);
      box-shadow: 0 0 0 0 rgba(16, 185, 129, 0.7);
    }
    70% {
      transform: scale(1);
      box-shadow: 0 0 0 8px rgba(16, 185, 129, 0);
    }
    100% {
      transform: scale(0.95);
      box-shadow: 0 0 0 0 rgba(16, 185, 129, 0);
    }
  }

  .flow-waiting__text {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
  }

  .flow-cancel {
    display: flex;
    justify-content: flex-end;
  }

  .ms-alert {
    border-radius: var(--radius-md, 8px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    font-size: var(--text-sm, 0.875rem);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .ms-alert--pending {
    background: rgba(245, 158, 11, 0.12);
    border: 1px solid rgba(245, 158, 11, 0.35);
    color: #fbbf24;
  }

  .ms-alert--error {
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.35);
    color: #f87171;
  }

  .ms-alert__head {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-weight: 600;
  }

  .alert-desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    line-height: 1.4;
  }

  /* --- Advanced Client ID Accordion --- */
  .advanced-section {
    background: rgba(16, 24, 40, 0.4);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    overflow: hidden;
  }

  .advanced-toggle {
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 500;
    display: flex;
    align-items: center;
    justify-content: space-between;
    cursor: pointer;
    transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
  }

  .advanced-toggle:hover {
    background: rgba(255, 255, 255, 0.03);
    color: var(--text, #f1f5f9);
  }

  .advanced-toggle__label {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .advanced-caret {
    font-size: 0.65rem;
    transition: transform var(--dur-fast, 120ms);
  }

  .advanced-caret--open {
    transform: rotate(180deg);
  }

  .advanced-pane {
    padding: var(--space-4, 16px);
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    background: rgba(0, 0, 0, 0.2);
  }

  .advanced-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* --- Lunar Token Shield Card --- */
  .lunar-shield-card {
    background: linear-gradient(135deg, rgba(168, 85, 247, 0.08), rgba(16, 24, 40, 0.5));
    border: 1px solid rgba(168, 85, 247, 0.25);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .lunar-shield-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .lunar-shield-card__title-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .lunar-icon {
    font-size: 1.1rem;
  }

  .lunar-title {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .lunar-shield-card__desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    line-height: 1.4;
  }

  .lunar-shield-card__action {
    display: flex;
    justify-content: flex-end;
  }

  /* Helper Spinner */
  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, #10b981);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .text-ok {
    color: var(--accent, #10b981);
  }
</style>
