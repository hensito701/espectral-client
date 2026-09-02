<!--
  ============================================================================
  ServerRadar.svelte — Horizon Glass Multiplayer Server Radar Screen (#/servers)
  ============================================================================
  Real-time server telemetry deck monitoring official Espectral network hosts.
  Features live player aggregation metrics, animated beacon cards, manual
  radar re-scan action, custom server connection guides, and zero-state visuals.
-->
<script lang="ts">
  import { servers } from '../lib/stores';
  import ServerRadarCard from '../components/ServerRadarCard.svelte';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import GradientText from '../components/GradientText.svelte';
  import { timeAgo } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';

  let refreshing = $state(false);

  async function handleRefresh() {
    refreshing = true;
    try {
      await $servers.refresh();
      pushToast({ kind: 'ok', text: t('servers.lastUpdated', { time: t('time.now') }) });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast({ kind: 'err', text: t('servers.errorTitle') + ': ' + msg });
    } finally {
      refreshing = false;
    }
  }

  // Aggregate metrics
  const serverList = $derived($servers.value || []);
  const totalServers = $derived(serverList.length);
  const onlineCount = $derived(serverList.filter((s) => s.online).length);
  const offlineCount = $derived(totalServers - onlineCount);

  const totalPlayers = $derived.by(() => {
    return serverList.reduce((acc, curr) => acc + (curr.online ? (curr.players_online || 0) : 0), 0);
  });

  const networkHealth = $derived.by(() => {
    if (totalServers === 0 || $servers.error) return { status: 'offline', label: t('servers.networkOffline'), variant: 'err' as const };
    if (onlineCount === totalServers) return { status: 'optimal', label: t('servers.networkOptimal'), variant: 'ok' as const };
    if (onlineCount > 0) return { status: 'degraded', label: t('servers.networkDegraded'), variant: 'warn' as const };
    return { status: 'offline', label: t('servers.networkOffline'), variant: 'err' as const };
  });

  const lastUpdatedLabel = $derived.by(() => {
    if (!$servers.lastFetched) return t('servers.noData');
    return t('servers.lastUpdated', { time: timeAgo($servers.lastFetched) });
  });

  function navigateTo(hash: string) {
    if (typeof window !== 'undefined') {
      window.location.hash = hash;
    }
  }
</script>

<svelte:head>
  <title>{t('servers.tag')}</title>
</svelte:head>

<div class="server-radar-page">
  <!-- Radar Hero Banner -->
  <header class="radar-hero">
    <div class="radar-hero__content">
      <div class="radar-hero__badge-row">
        <Badge variant="accent" size="sm" dot={true}>
          {t('servers.badge')}
        </Badge>
        <span class="radar-hero__poll-pill">
          {t('servers.autoPollNote')}
        </span>
      </div>

      <h1 class="radar-hero__title">
        <GradientText variant="emerald" size="2xl">
          {t('servers.title')}
        </GradientText>
      </h1>

      <p class="radar-hero__subtitle">
        {t('servers.subtitle')}
      </p>
    </div>

    <!-- Right-aligned action dock -->
    <div class="radar-hero__actions">
      <Btn
        variant="secondary"
        size="md"
        loading={refreshing || ($servers.loading && serverList.length > 0)}
        onclick={handleRefresh}
      >
        {#snippet icon()}
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="spin-on-load">
            <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/>
          </svg>
        {/snippet}
        {refreshing ? t('servers.refreshing') : t('servers.refresh')}
      </Btn>
    </div>
  </header>

  <!-- Telemetry HUD Summary Strip -->
  <section class="telemetry-strip" aria-label="Network Telemetry Summary">
    <!-- Stat 1: Total Players Online -->
    <div class="telemetry-card">
      <div class="telemetry-card__header">
        <span class="telemetry-card__icon text-accent">👥</span>
        <span class="telemetry-card__label">{t('server.players')}</span>
      </div>
      <div class="telemetry-card__value-row">
        <span class="telemetry-card__number font-pixel">{totalPlayers}</span>
        <span class="telemetry-card__sub font-pixel">{t('common.online')}</span>
      </div>
    </div>

    <!-- Stat 2: Active Nodes Ratio -->
    <div class="telemetry-card">
      <div class="telemetry-card__header">
        <span class="telemetry-card__icon text-cyan">📡</span>
        <span class="telemetry-card__label">{t('servers.onlineNodes')}</span>
      </div>
      <div class="telemetry-card__value-row">
        <span class="telemetry-card__number font-pixel">{onlineCount}</span>
        <span class="telemetry-card__denom font-pixel">/ {totalServers}</span>
      </div>
    </div>

    <!-- Stat 3: Overall Network Health -->
    <div class="telemetry-card">
      <div class="telemetry-card__header">
        <span class="telemetry-card__icon text-emerald">🛡️</span>
        <span class="telemetry-card__label">{t('servers.networkHealth')}</span>
      </div>
      <div class="telemetry-card__value-row">
        <Badge variant={networkHealth.variant} size="md" dot={true}>
          {networkHealth.label}
        </Badge>
      </div>
    </div>

    <!-- Stat 4: Telemetry Heartbeat Clock -->
    <div class="telemetry-card telemetry-card--time">
      <div class="telemetry-card__header">
        <span class="telemetry-card__icon">⏱️</span>
        <span class="telemetry-card__label">{lastUpdatedLabel}</span>
      </div>
      <div class="telemetry-card__subtext">
        {#if offlineCount > 0}
          <span class="text-warn">{t('servers.someOffline')}</span>
        {:else if totalServers > 0}
          <span class="text-ok">{t('servers.allOnline')}</span>
        {:else}
          <span>{t('servers.noData')}</span>
        {/if}
      </div>
    </div>
  </section>

  <!-- Server Deck Grid -->
  <section class="server-deck-section">
    {#if $servers.loading && !serverList.length}
      <!-- Loading Skeleton Deck -->
      <div class="server-grid">
        {#each [1, 2, 3] as _}
          <GlassCard elevation="md" class="skeleton-card">
            <div class="skeleton-layout">
              <div class="skeleton-avatar"></div>
              <div class="skeleton-lines">
                <div class="skeleton-bar skeleton-bar--title"></div>
                <div class="skeleton-bar skeleton-bar--sub"></div>
              </div>
            </div>
          </GlassCard>
        {/each}
      </div>
    {:else if $servers.error && !serverList.length}
      <!-- Error Fallback Banner -->
      <GlassCard elevation="lg" featured={true} class="error-banner">
        <div class="error-banner__layout">
          <div class="error-banner__icon">⚠️</div>
          <div class="error-banner__body">
            <h2 class="error-banner__title">{t('servers.errorTitle')}</h2>
            <p class="error-banner__desc">{$servers.error}</p>
          </div>
          <Btn variant="primary" size="md" onclick={handleRefresh}>
            {t('servers.retry')}
          </Btn>
        </div>
      </GlassCard>
    {:else if !serverList.length}
      <!-- Empty State -->
      <GlassCard elevation="md" class="empty-state">
        <div class="empty-state__inner">
          <div class="radar-dish-art">
            <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="radar-dish-svg">
              <circle cx="12" cy="12" r="10" stroke-dasharray="4 4" opacity="0.4"/>
              <circle cx="12" cy="12" r="6" opacity="0.6"/>
              <circle cx="12" cy="12" r="2" fill="currentColor"/>
              <path d="M12 2v4M12 18v4M2 12h4M18 12h4" opacity="0.4"/>
              <line x1="12" y1="12" x2="19" y2="5" stroke="var(--accent)" stroke-width="2"/>
            </svg>
          </div>
          <h2 class="empty-state__title">{t('servers.empty')}</h2>
          <p class="empty-state__desc">{t('servers.noServers')}</p>
          <Btn variant="secondary" size="md" onclick={handleRefresh}>
            {t('servers.refresh')}
          </Btn>
        </div>
      </GlassCard>
    {:else}
      <!-- Active Server Cards Grid -->
      <div class="server-grid">
        {#each serverList as server (server.host)}
          <ServerRadarCard {server} />
        {/each}
      </div>
    {/if}
  </section>

  <!-- Custom Servers Guidance Deck -->
  <section class="custom-servers-guidance">
    <GlassCard elevation="sm" class="guidance-card">
      <div class="guidance-layout">
        <div class="guidance-info">
          <div class="guidance-badge-row">
            <Badge variant="neutral" size="sm">💡 {t('servers.customTitle')}</Badge>
          </div>
          <h3 class="guidance-heading">{t('servers.customTitle')}</h3>
          <p class="guidance-desc">{t('servers.customDesc')}</p>
        </div>
        <div class="guidance-actions">
          <Btn variant="secondary" size="sm" onclick={() => navigateTo('#/instances')}>
            {t('servers.manageInstances')}
          </Btn>
          <Btn variant="ghost" size="sm" onclick={() => navigateTo('#/settings')}>
            {t('servers.openSettings')}
          </Btn>
        </div>
      </div>
    </GlassCard>
  </section>
</div>

<style>
  .server-radar-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-6, 1.5rem);
    width: 100%;
    max-width: 1200px;
    margin: 0 auto;
    padding-bottom: var(--space-8, 2rem);
  }

  /* Hero Header */
  .radar-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 1rem);
    padding: var(--space-2, 0.5rem) 0;
  }

  .radar-hero__content {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }

  .radar-hero__badge-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
  }

  .radar-hero__poll-pill {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-family: var(--font-mono, monospace);
  }

  .radar-hero__title {
    margin: 0;
    line-height: var(--leading-tight, 1.2);
  }

  .radar-hero__subtitle {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #b3c5e3);
    max-width: 620px;
  }

  .radar-hero__actions {
    flex-shrink: 0;
  }

  /* Telemetry Summary Strip */
  .telemetry-strip {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--space-3, 0.75rem);
  }

  .telemetry-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    background: var(--card-bg, rgba(13, 18, 34, 0.75));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-lg, 0.875rem);
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .telemetry-card__header {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
  }

  .telemetry-card__label {
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .telemetry-card__value-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-2, 0.5rem);
  }

  .telemetry-card__number {
    font-size: var(--text-xl, 1.375rem);
    color: var(--text, #e8ecf4);
    line-height: 1;
  }

  .telemetry-card__sub {
    font-size: 0.6875rem;
    color: var(--accent, #10b981);
  }

  .telemetry-card__denom {
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
  }

  .telemetry-card__subtext {
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
  }

  .text-accent {
    color: var(--accent, #10b981);
  }

  .text-cyan {
    color: var(--accent-cyan, #06b6d4);
  }

  .text-emerald {
    color: var(--accent-green, #22c55e);
  }

  .text-ok {
    color: var(--accent-green, #22c55e);
  }

  .text-warn {
    color: var(--accent-alt, #f59e0b);
  }

  /* Server Grid */
  .server-deck-section {
    width: 100%;
  }

  .server-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: var(--space-4, 1rem);
  }

  @media (max-width: 768px) {
    .server-grid {
      grid-template-columns: 1fr;
    }
  }

  /* Skeleton Loading */
  .skeleton-card {
    min-height: 180px;
    padding: var(--space-4, 1rem);
  }

  .skeleton-layout {
    display: flex;
    gap: var(--space-3, 0.75rem);
  }

  .skeleton-avatar {
    width: 48px;
    height: 48px;
    border-radius: var(--radius-md, 0.625rem);
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    animation: shimmer 1.8s infinite;
  }

  .skeleton-lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }

  .skeleton-bar {
    border-radius: var(--radius-xs, 0.25rem);
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    animation: shimmer 1.8s infinite;
  }

  .skeleton-bar--title {
    width: 60%;
    height: 18px;
  }

  .skeleton-bar--sub {
    width: 90%;
    height: 14px;
  }

  /* Error & Empty States */
  .error-banner__layout {
    display: flex;
    align-items: center;
    gap: var(--space-4, 1rem);
    padding: var(--space-4, 1rem);
  }

  .error-banner__icon {
    font-size: 2rem;
  }

  .error-banner__body {
    flex: 1;
  }

  .error-banner__title {
    margin: 0;
    font-size: var(--text-base, 1rem);
    color: var(--accent-red, #ef4444);
  }

  .error-banner__desc {
    margin: 4px 0 0 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #b3c5e3);
  }

  .empty-state {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
  }

  .empty-state__inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    max-width: 400px;
    margin: 0 auto;
  }

  .radar-dish-svg {
    color: var(--muted, #8e9eb8);
  }

  .empty-state__title {
    margin: 0;
    font-size: var(--text-lg, 1.125rem);
    color: var(--text, #e8ecf4);
  }

  .empty-state__desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted, #8e9eb8);
  }

  /* Guidance Deck */
  .guidance-card {
    padding: var(--space-4, 1rem);
  }

  .guidance-layout {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4, 1rem);
    flex-wrap: wrap;
  }

  .guidance-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
    max-width: 700px;
  }

  .guidance-heading {
    margin: 0;
    font-size: var(--text-base, 1rem);
    color: var(--text, #e8ecf4);
  }

  .guidance-desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    line-height: var(--leading-snug, 1.35);
  }

  .guidance-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    flex-shrink: 0;
  }
</style>
