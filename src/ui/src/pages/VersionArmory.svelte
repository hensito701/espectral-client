<!--
  ============================================================================
  VersionArmory.svelte — Horizon Glass Version Armory (#/library/versions)
  ============================================================================
  Mojang version manifest matrix with chunked virtualized list rendering,
  sticky search and type filtering, live cross-referencing of installed
  instances, JDK requirement chips, and one-click quick instance creation.
-->
<script lang="ts">
  import { versions, instances } from '../lib/stores';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import GradientText from '../components/GradientText.svelte';
  import { useCopy } from '../lib/useCopy.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { t } from '../lib/i18n.svelte';

  type TypeFilter = 'all' | 'release' | 'snapshot' | 'old_beta' | 'old_alpha' | 'installed';

  let searchQuery = $state('');
  let typeFilter = $state<TypeFilter>('all');
  let visibleLimit = $state(100);
  let sentinelEl = $state<HTMLDivElement | null>(null);

  const copier = useCopy(1600);

  const manifest = $derived($versions.value);
  const allVersions = $derived(manifest?.versions || []);

  // Cross-reference map: version id -> array of instance summaries
  const installedMap = $derived.by(() => {
    const map = new Map<string, string[]>();
    const instList = $instances.value || [];
    for (const inst of instList) {
      if (inst.version) {
        const existing = map.get(inst.version) || [];
        existing.push(inst.name);
        map.set(inst.version, existing);
      }
    }
    return map;
  });

  const totalInstalledVersionsCount = $derived(installedMap.size);

  // Filtered versions based on search query and selected filter tab
  const filteredVersions = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return allVersions.filter((entry) => {
      // Type / Installed check
      if (typeFilter === 'installed') {
        if (!installedMap.has(entry.id)) return false;
      } else if (typeFilter !== 'all') {
        if (entry.type !== typeFilter) return false;
      }

      // Search match
      if (q) {
        const matchesId = entry.id.toLowerCase().includes(q);
        const matchesType = entry.type.toLowerCase().includes(q);
        const matchesDate = (entry.release_time || '').includes(q);
        if (!matchesId && !matchesType && !matchesDate) return false;
      }

      return true;
    });
  });

  // Slice for chunked virtualized DOM rendering
  const visibleVersions = $derived(filteredVersions.slice(0, visibleLimit));
  const hasMore = $derived(visibleLimit < filteredVersions.length);

  // Reset pagination on filter or search changes
  $effect(() => {
    // Reading reactivity triggers
    void searchQuery;
    void typeFilter;
    visibleLimit = 100;
  });

  // IntersectionObserver to auto-expand visible list as user reaches bottom sentinel
  $effect(() => {
    if (!sentinelEl) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) {
          if (visibleLimit < filteredVersions.length) {
            visibleLimit = Math.min(visibleLimit + 100, filteredVersions.length);
          }
        }
      },
      { rootMargin: '400px' }
    );
    observer.observe(sentinelEl);
    return () => observer.disconnect();
  });

  function formatDate(iso: string): string {
    if (!iso) return '—';
    try {
      const d = new Date(iso);
      if (Number.isNaN(d.getTime())) return iso.split('T')[0] || iso;
      return d.toLocaleDateString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
      });
    } catch {
      return iso.split('T')[0] || iso;
    }
  }

  function getJavaRequirement(id: string): string {
    // Mojang JDK policy:
    // >= 1.20.5: Java 21
    // >= 1.18: Java 17
    // >= 1.17: Java 16
    // < 1.17: Java 8
    const numMatch = id.match(/^1\.(\d+)(?:\.(\d+))?/);
    if (numMatch) {
      const major = parseInt(numMatch[1], 10);
      const minor = parseInt(numMatch[2] || '0', 10);
      if (major > 20 || (major === 20 && minor >= 5)) return '21';
      if (major >= 18) return '17';
      if (major === 17) return '16';
      return '8';
    }
    // Snapshots: 24w14a+ -> 21
    if (id.startsWith('24w') || id.startsWith('25w') || id.startsWith('26w')) {
      return '21';
    }
    return '21';
  }

  function handleCreateInstance(versionId: string) {
    if (typeof window !== 'undefined') {
      window.location.hash = '#/';
      // Dispatch standard Wave B wizard open event
      window.dispatchEvent(
        new CustomEvent('horizon:open-wizard', {
          detail: { version: versionId },
        })
      );
      pushToast({
        kind: 'info',
        text: t('versions.openInWizard') + ` (${versionId})`,
      });
    }
  }

  function handleCopyVersion(versionId: string, e: MouseEvent) {
    e.stopPropagation();
    copier.copy(versionId);
    pushToast({
      kind: 'ok',
      text: t('versions.copiedVersion', { version: versionId }),
    });
  }

  function handleRetryManifest() {
    $versions.refresh();
  }
</script>

<svelte:head>
  <title>{t('versions.tag')}</title>
</svelte:head>

<div class="version-armory-page">
  <!-- Armory Hero Header -->
  <header class="armory-hero">
    <div class="armory-hero__content">
      <div class="armory-hero__badge-row">
        <Badge variant="accent" size="sm" dot={true}>
          {t('versions.badge')}
        </Badge>
        <span class="armory-hero__manifest-count">
          {t('versions.totalVersions', { count: allVersions.length })}
        </span>
      </div>

      <h1 class="armory-hero__title">
        <GradientText variant="emerald" size="2xl">
          {t('versions.title')}
        </GradientText>
      </h1>

      <p class="armory-hero__subtitle">
        {t('versions.subtitle')}
      </p>
    </div>

    <!-- Quick Stats Cards -->
    <div class="armory-hero__stats">
      {#if manifest?.latest_release}
        <div class="quick-stat-card">
          <span class="quick-stat-card__label">{t('versions.latestRelease')}</span>
          <div class="quick-stat-card__val-row">
            <span class="quick-stat-card__version font-mono">{manifest.latest_release}</span>
            <Badge variant="ok" size="sm">Stable</Badge>
          </div>
        </div>
      {/if}

      {#if manifest?.latest_snapshot}
        <div class="quick-stat-card">
          <span class="quick-stat-card__label">{t('versions.latestSnapshot')}</span>
          <div class="quick-stat-card__val-row">
            <span class="quick-stat-card__version font-mono">{manifest.latest_snapshot}</span>
            <Badge variant="purple" size="sm">Snap</Badge>
          </div>
        </div>
      {/if}
    </div>
  </header>

  <!-- Sticky Search & Filter Command Matrix -->
  <section class="armory-controls-sticky">
    <GlassCard elevation="md" class="controls-glass-card">
      <div class="controls-layout">
        <!-- Search Input -->
        <div class="search-field-wrap">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="search-icon">
            <circle cx="11" cy="11" r="8"/>
            <line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
          <input
            type="search"
            class="search-input"
            placeholder={t('versions.searchPlaceholder')}
            aria-label={t('versions.searchAria')}
            bind:value={searchQuery}
          />
          {#if searchQuery}
            <button
              type="button"
              class="clear-search-btn"
              onclick={() => (searchQuery = '')}
              aria-label="Clear search"
            >
              ✕
            </button>
          {/if}
        </div>

        <!-- Filter Segment Chips -->
        <div class="filter-chips-rail" role="group" aria-label={t('versions.filterAria')}>
          <button
            type="button"
            class="filter-chip"
            class:filter-chip--active={typeFilter === 'all'}
            onclick={() => (typeFilter = 'all')}
          >
            {t('versions.filterAll')}
          </button>

          <button
            type="button"
            class="filter-chip"
            class:filter-chip--active={typeFilter === 'release'}
            onclick={() => (typeFilter = 'release')}
          >
            {t('versions.filterRelease')}
          </button>

          <button
            type="button"
            class="filter-chip"
            class:filter-chip--active={typeFilter === 'snapshot'}
            onclick={() => (typeFilter = 'snapshot')}
          >
            {t('versions.filterSnapshot')}
          </button>

          <button
            type="button"
            class="filter-chip"
            class:filter-chip--active={typeFilter === 'old_beta'}
            onclick={() => (typeFilter = 'old_beta')}
          >
            {t('versions.filterBeta')}
          </button>

          <button
            type="button"
            class="filter-chip"
            class:filter-chip--active={typeFilter === 'old_alpha'}
            onclick={() => (typeFilter = 'old_alpha')}
          >
            {t('versions.filterAlpha')}
          </button>

          <button
            type="button"
            class="filter-chip filter-chip--installed"
            class:filter-chip--active={typeFilter === 'installed'}
            onclick={() => (typeFilter = 'installed')}
          >
            {t('versions.filterInstalled')} ({totalInstalledVersionsCount})
          </button>
        </div>
      </div>

      <!-- Result Counter Line -->
      <div class="controls-meta-bar">
        <span class="results-count-text">
          {t('versions.showingCount', {
            shown: visibleVersions.length,
            total: filteredVersions.length,
          })}
        </span>
      </div>
    </GlassCard>
  </section>

  <!-- Versions Armory List -->
  <section class="armory-matrix-section">
    {#if $versions.loading && !manifest}
      <!-- Loading State -->
      <div class="loading-state">
        <div class="loading-spinner"></div>
        <p class="loading-text">{t('versions.loading')}</p>
      </div>
    {:else if $versions.error && !manifest}
      <!-- Error State -->
      <GlassCard elevation="lg" featured={true} class="error-card">
        <div class="error-layout">
          <span class="error-icon">⚠️</span>
          <div class="error-body">
            <h3 class="error-title">{t('versions.error', { error: $versions.error })}</h3>
          </div>
          <Btn variant="primary" size="md" onclick={handleRetryManifest}>
            {t('versions.retry')}
          </Btn>
        </div>
      </GlassCard>
    {:else if !filteredVersions.length}
      <!-- Empty Filter Results -->
      <GlassCard elevation="md" class="empty-results-card">
        <div class="empty-results-inner">
          <span class="empty-icon">🔍</span>
          <h3 class="empty-title">
            {searchQuery ? t('versions.noResults', { query: searchQuery }) : t('versions.none')}
          </h3>
          <p class="empty-desc">{t('versions.noResultsDesc')}</p>
          {#if searchQuery || typeFilter !== 'all'}
            <Btn
              variant="secondary"
              size="sm"
              onclick={() => {
                searchQuery = '';
                typeFilter = 'all';
              }}
            >
              {t('versions.filterAll')}
            </Btn>
          {/if}
        </div>
      </GlassCard>
    {:else}
      <!-- Version Matrix Rows -->
      <div class="version-matrix-table">
        {#each visibleVersions as entry (entry.id)}
          {@const installedInstances = installedMap.get(entry.id)}
          {@const isLatestRelease = manifest?.latest_release === entry.id}
          {@const isLatestSnapshot = manifest?.latest_snapshot === entry.id}
          {@const javaTier = getJavaRequirement(entry.id)}

          <div class="version-row-wrap">
            <GlassCard
              elevation="sm"
              interactive={true}
              featured={isLatestRelease || !!installedInstances}
              class="version-row-card"
            >
              <div class="version-row-content">
                <!-- Left: Identity & Version Mono -->
                <div class="version-identity">
                  <button
                    type="button"
                    class="version-id-btn"
                    title={t('versions.createInstanceTooltip', { version: entry.id })}
                    onclick={(e) => handleCopyVersion(entry.id, e)}
                  >
                    <span class="version-id-text font-mono">{entry.id}</span>
                  </button>

                  <div class="version-badges-cluster">
                    {#if isLatestRelease}
                      <Badge variant="accent" size="sm" dot={true}>
                        {t('versions.latestRelease')}
                      </Badge>
                    {/if}

                    {#if isLatestSnapshot}
                      <Badge variant="purple" size="sm" dot={true}>
                        {t('versions.latestSnapshot')}
                      </Badge>
                    {/if}

                    {#if entry.type === 'release'}
                      <Badge variant="neutral" size="sm">
                        {t('versions.typeRelease')}
                      </Badge>
                    {:else if entry.type === 'snapshot'}
                      <Badge variant="purple" size="sm">
                        {t('versions.typeSnapshot')}
                      </Badge>
                    {:else if entry.type === 'old_beta'}
                      <Badge variant="warn" size="sm">
                        {t('versions.typeBeta')}
                      </Badge>
                    {:else if entry.type === 'old_alpha'}
                      <Badge variant="gold" size="sm">
                        {t('versions.typeAlpha')}
                      </Badge>
                    {/if}

                    <!-- JDK Requirement Badge -->
                    <span class="jdk-chip font-pixel" title={t('versions.requiresJdk', { java: javaTier })}>
                      JDK {javaTier}
                    </span>
                  </div>
                </div>

                <!-- Center: Installed Instance Cross-Reference -->
                <div class="version-crossref">
                  {#if installedInstances && installedInstances.length > 0}
                    <div class="installed-tag-wrap">
                      <Badge variant="ok" size="sm" dot={true}>
                        {t('versions.installedIn', { instances: installedInstances.join(', ') })}
                      </Badge>
                    </div>
                  {/if}
                </div>

                <!-- Right: Date & Launch Action -->
                <div class="version-actions-dock">
                  <div class="version-date-chip">
                    <span class="date-text">{formatDate(entry.release_time)}</span>
                  </div>

                  <Btn
                    variant="secondary"
                    size="sm"
                    title={t('versions.createInstanceTooltip', { version: entry.id })}
                    onclick={() => handleCreateInstance(entry.id)}
                  >
                    {#snippet icon()}
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <line x1="12" y1="5" x2="12" y2="19"/>
                        <line x1="5" y1="12" x2="19" y2="12"/>
                      </svg>
                    {/snippet}
                    {t('versions.createInstanceAction')}
                  </Btn>
                </div>
              </div>
            </GlassCard>
          </div>
        {/each}
      </div>

      <!-- Infinite Scroll Sentinel & Load More Indicator -->
      <div bind:this={sentinelEl} class="sentinel-anchor">
        {#if hasMore}
          <div class="load-more-indicator">
            <span class="load-more-text">
              {t('versions.loadMore', { remaining: filteredVersions.length - visibleLimit })}
            </span>
          </div>
        {:else if filteredVersions.length > 0}
          <div class="end-manifest-indicator">
            <span class="end-manifest-text">{t('versions.allLoaded')}</span>
          </div>
        {/if}
      </div>
    {/if}
  </section>
</div>

<style>
  .version-armory-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-5, 1.25rem);
    width: 100%;
    max-width: var(--content-max, 82rem);
    margin: 0 auto;
    padding: var(--space-6, 24px) var(--space-6, 24px) var(--space-12, 48px);
  }

  /* Hero Header */
  .armory-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 1rem);
    padding: var(--space-2, 0.5rem) 0;
    flex-wrap: wrap;
  }

  .armory-hero__content {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
    max-width: 680px;
  }

  .armory-hero__badge-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
  }

  .armory-hero__manifest-count {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-family: var(--font-mono, monospace);
  }

  .armory-hero__title {
    margin: 0;
    line-height: var(--leading-tight, 1.2);
  }

  .armory-hero__subtitle {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #b3c5e3);
  }

  .armory-hero__stats {
    display: flex;
    gap: var(--space-3, 0.75rem);
    flex-shrink: 0;
  }

  .quick-stat-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 0.25rem);
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    background: var(--card-bg, rgba(13, 18, 34, 0.75));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-lg, 0.875rem);
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .quick-stat-card__label {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .quick-stat-card__val-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
  }

  .quick-stat-card__version {
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  /* Sticky Controls Matrix */
  .armory-controls-sticky {
    position: sticky;
    top: 72px;
    z-index: 15;
    backdrop-filter: blur(var(--glass-blur, 16px));
  }

  .controls-layout {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 0.75rem);
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    flex-wrap: wrap;
  }

  .search-field-wrap {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 260px;
  }

  .search-icon {
    position: absolute;
    left: 12px;
    color: var(--muted, #8e9eb8);
    pointer-events: none;
  }

  .search-input {
    width: 100%;
    height: 40px;
    padding: 0 36px 0 38px;
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    color: var(--text, #e8ecf4);
    font-size: var(--text-sm, 0.875rem);
    transition: border-color var(--dur-fast, 120ms), box-shadow var(--dur-fast, 120ms);
  }

  .search-input:focus {
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 0 2px rgba(16, 185, 129, 0.25);
  }

  .clear-search-btn {
    position: absolute;
    right: 10px;
    background: none;
    border: none;
    color: var(--muted, #8e9eb8);
    cursor: pointer;
    font-size: 14px;
    padding: 4px;
  }

  .clear-search-btn:hover {
    color: var(--text, #e8ecf4);
  }

  .filter-chips-rail {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    overflow-x: auto;
    padding-bottom: 2px;
  }

  .filter-chip {
    padding: 6px 14px;
    border-radius: var(--radius-pill, 9999px);
    background: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    color: var(--muted-strong, #b3c5e3);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all var(--dur-fast, 120ms) var(--ease-out-expo, ease-out);
  }

  .filter-chip:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    color: var(--text, #e8ecf4);
    border-color: var(--border-focus, rgba(16, 185, 129, 0.6));
  }

  .filter-chip--active {
    background: rgba(16, 185, 129, 0.18);
    border-color: var(--accent, #10b981);
    color: var(--accent, #10b981);
    font-weight: 600;
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.25);
  }

  .filter-chip--installed.filter-chip--active {
    background: rgba(34, 197, 94, 0.2);
    border-color: var(--accent-green, #22c55e);
    color: var(--accent-green, #22c55e);
  }

  .controls-meta-bar {
    padding: 0 var(--space-4, 1rem) var(--space-2, 0.5rem);
  }

  .results-count-text {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
    font-family: var(--font-mono, monospace);
  }

  /* Matrix Table & Rows */
  .version-matrix-table {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }

  .version-row-wrap {
    width: 100%;
  }

  .version-row-content {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 0.75rem);
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
  }

  .version-identity {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    flex-shrink: 0;
    min-width: 240px;
  }

  .version-id-btn {
    all: unset;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
  }

  .version-id-text {
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
    letter-spacing: -0.02em;
    transition: color var(--dur-fast, 120ms);
  }

  .version-id-btn:hover .version-id-text {
    color: var(--accent, #10b981);
    text-decoration: underline;
  }

  .version-badges-cluster {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
  }

  .jdk-chip {
    font-size: 0.5625rem;
    padding: 2px 6px;
    border-radius: var(--radius-xs, 0.25rem);
    background: var(--surface-up-solid, #161e36);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    color: var(--accent-gold, #ffd700);
    letter-spacing: 0.02em;
  }

  .version-crossref {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    padding: 0 var(--space-2, 0.5rem);
  }

  .installed-tag-wrap {
    display: flex;
    align-items: center;
  }

  .version-actions-dock {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    flex-shrink: 0;
  }

  .version-date-chip {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-family: var(--font-mono, monospace);
  }

  /* Loading, Error & Empty states */
  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3, 0.75rem);
    padding: var(--space-12, 3rem) 0;
  }

  .loading-spinner {
    width: 36px;
    height: 36px;
    border: 3px solid var(--border, rgba(40, 58, 96, 0.45));
    border-top-color: var(--accent, #10b981);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .loading-text {
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #b3c5e3);
  }

  .error-card {
    padding: var(--space-4, 1rem);
  }

  .error-layout {
    display: flex;
    align-items: center;
    gap: var(--space-4, 1rem);
  }

  .error-icon {
    font-size: 1.75rem;
  }

  .error-body {
    flex: 1;
  }

  .error-title {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--accent-red, #ef4444);
  }

  .empty-results-card {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
  }

  .empty-results-inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    max-width: 420px;
    margin: 0 auto;
  }

  .empty-icon {
    font-size: 2rem;
    opacity: 0.6;
  }

  .empty-title {
    margin: 0;
    font-size: var(--text-base, 1rem);
    color: var(--text, #e8ecf4);
  }

  .empty-desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  /* Sentinel */
  .sentinel-anchor {
    padding: var(--space-4, 1rem) 0;
    text-align: center;
  }

  .load-more-text,
  .end-manifest-text {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-family: var(--font-mono, monospace);
  }
</style>
