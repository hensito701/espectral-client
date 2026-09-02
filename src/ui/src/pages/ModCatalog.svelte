<!--
  ============================================================================
  ModCatalog.svelte — Horizon Glass Mod Catalog & Presets Center (#/mods)
  ============================================================================
  Next-generation Minecraft mod management console featuring instance chips,
  performance/branding/QoL preset decks, live SHA-1 verified SSE download
  tracking, density-responsive mod inventory, and integrated Modrinth search.
-->
<script lang="ts">
  import { instances } from '../lib/stores';
  import {
    getConfig,
    getModsPresetInfo,
    installModrinthMod,
    installMods,
    listMods,
    openFolder,
    searchModrinth,
    setModEnabled,
    subscribeEvents,
  } from '../lib/api';
  import type { ModEntry, ModPreset, ModPresetInfo, ModrinthProject } from '../lib/types';
  import { pushToast } from '../lib/toast.svelte';
  import { formatBytes } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { fade, flyY } from '../lib/motion';

  // Horizon Glass UI Primitives
  import Btn from '../components/Btn.svelte';
  import GlassCard from '../components/GlassCard.svelte';
  import Badge from '../components/Badge.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import LoaderBadge from '../components/LoaderBadge.svelte';
  import MonogramTile from '../components/MonogramTile.svelte';
  import GradientText from '../components/GradientText.svelte';

  const STORAGE_KEY_INSTANCE = 'horizon:mods-instance';
  const STORAGE_KEY_DENSITY = 'horizon:density';

  // Pinned standard mod packs
  const PINNED_SET: { slug: string; requires?: string[]; requiredBy?: string[] }[] = [
    { slug: 'sodium', requiredBy: ['iris'] },
    { slug: 'iris', requires: ['sodium'] },
    { slug: 'lithium' },
    { slug: 'ferrite-core' },
    { slug: 'krypton' },
    { slug: 'modmenu' },
    { slug: 'fabric-api' },
  ];

  const QOL_SET: { slug: string }[] = [
    { slug: 'gamma-utils' },
    { slug: 'clear-fog' },
  ];

  // State
  let targetInstance = $state(
    (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_KEY_INSTANCE)) || ''
  );
  let density = $state<'compact' | 'spacious'>(
    (typeof localStorage !== 'undefined' && (localStorage.getItem(STORAGE_KEY_DENSITY) as 'compact' | 'spacious')) || 'spacious'
  );

  let activeTab = $state<'installed' | 'presets' | 'modrinth'>('installed');
  let filterStatus = $state<'all' | 'enabled' | 'disabled'>('all');
  let filterQuery = $state('');

  let mods = $state<ModEntry[]>([]);
  let loadingMods = $state(true);
  let modsError = $state('');

  let presetInfo = $state<ModPresetInfo | null>(null);
  let installingPreset = $state(false);
  let installProgress = $state<{ filename: string; done: number; total: number } | null>(null);

  // Modrinth Search State
  let searchQuery = $state('');
  let searchResults = $state<ModrinthProject[] | null>(null);
  let searching = $state(false);
  let searchError = $state('');
  let installingProjectId = $state<string | null>(null);
  let searchSeq = 0;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Selected Instance derivation
  const selectedInstance = $derived(
    $instances.value.find((inst) => inst.name === targetInstance) ?? null
  );

  const presetAvailable = $derived(presetInfo?.supported !== false);
  const brandingAvailable = $derived(presetInfo?.branding?.supported !== false);
  const pinSlugs = new Set(PINNED_SET.map((p) => p.slug));
  const perfMissing = $derived(
    mods.filter((m) => pinSlugs.has(m.project_slug ?? '') && !m.installed)
  );

  // Filtered mods list
  const filteredMods = $derived.by(() => {
    let list = mods;
    if (filterStatus === 'enabled') list = list.filter((m) => m.enabled);
    if (filterStatus === 'disabled') list = list.filter((m) => !m.enabled);

    const q = filterQuery.trim().toLowerCase();
    if (q) {
      list = list.filter(
        (m) =>
          m.filename.toLowerCase().includes(q) ||
          (m.project_slug && m.project_slug.toLowerCase().includes(q))
      );
    }
    return list;
  });

  const enabledCount = $derived(mods.filter((m) => m.enabled).length);
  const disabledCount = $derived(mods.filter((m) => !m.enabled).length);

  // Sync instance selection with localStorage
  $effect(() => {
    if (typeof localStorage !== 'undefined' && targetInstance) {
      localStorage.setItem(STORAGE_KEY_INSTANCE, targetInstance);
    }
  });

  // Ensure default instance exists
  $effect(() => {
    if ($instances.value.length > 0) {
      const exists = $instances.value.some((inst) => inst.name === targetInstance);
      if (!exists || !targetInstance) {
        targetInstance = $instances.value[0].name;
      }
    }
  });

  // Load mods on targetInstance change
  $effect(() => {
    if (targetInstance) {
      loadMods();
      loadPresetInfo();
    }
  });

  // Global density event listener
  $effect(() => {
    const handleDensityChange = (e: CustomEvent<{ density: 'compact' | 'spacious' }>) => {
      if (e.detail?.density && e.detail.density !== density) {
        density = e.detail.density;
      }
    };
    window.addEventListener('horizon:density-changed', handleDensityChange as EventListener);
    return () => {
      window.removeEventListener('horizon:density-changed', handleDensityChange as EventListener);
    };
  });

  // Subscribe to SSE mod-progress events
  $effect(() => {
    const unsub = subscribeEvents(({ type, data }) => {
      if (type !== 'mod-progress') return;
      const payload = (data ?? {}) as Record<string, unknown>;
      installProgress = {
        filename: typeof payload.filename === 'string' ? payload.filename : '',
        done: typeof payload.done === 'number' ? payload.done : 0,
        total: typeof payload.total === 'number' ? payload.total : 0,
      };

      if (
        typeof payload.total === 'number' &&
        typeof payload.done === 'number' &&
        payload.total > 0 &&
        payload.done >= payload.total
      ) {
        setTimeout(() => {
          installProgress = null;
          loadMods();
          loadPresetInfo();
          pushToast({
            kind: 'ok',
            text: payload.filename
              ? `${payload.filename} ${t('mods.installed') || 'instalado'}`
              : t('mods.installed') || 'Mods actualizados',
          });
        }, 600);
      }
    });

    return () => unsub();
  });

  function setDensity(next: 'compact' | 'spacious') {
    density = next;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(STORAGE_KEY_DENSITY, next);
    }
    if (typeof document !== 'undefined') {
      document.documentElement.dataset.density = next;
    }
    window.dispatchEvent(
      new CustomEvent('horizon:density-changed', { detail: { density: next } })
    );
  }

  async function loadMods() {
    if (!targetInstance) return;
    loadingMods = true;
    modsError = '';
    try {
      mods = await listMods(targetInstance);
    } catch (e) {
      modsError = e instanceof Error ? e.message : String(e);
    } finally {
      loadingMods = false;
    }
  }

  async function loadPresetInfo() {
    if (!targetInstance) return;
    presetInfo = null;
    try {
      presetInfo = await getModsPresetInfo(targetInstance);
    } catch {
      presetInfo = null;
    }
  }

  async function toggleModOptimistic(mod: ModEntry) {
    const nextState = !mod.enabled;
    // Optimistic local update
    const previousState = mod.enabled;
    mod.enabled = nextState;

    try {
      await setModEnabled(targetInstance, mod.filename, nextState);
      pushToast({
        kind: 'ok',
        text: `${modDisplayName(mod)} ${nextState ? t('mods.enabled') : t('mods.disabled')}`,
      });
    } catch (e) {
      // Revert on error
      mod.enabled = previousState;
      pushToast({
        kind: 'err',
        text: e instanceof Error ? e.message : t('mods.toggleError'),
      });
    }
  }

  async function installPreset(preset: ModPreset = 'performance') {
    if (!targetInstance) return;
    installingPreset = true;
    installProgress = { filename: '', done: 0, total: 0 };
    try {
      await installMods(targetInstance, preset);
      pushToast({ kind: 'info', text: t('mods.queued') });
    } catch (e) {
      modsError = e instanceof Error ? e.message : String(e);
      installProgress = null;
      pushToast({
        kind: 'err',
        text: e instanceof Error ? e.message : 'Error installing preset',
      });
    } finally {
      installingPreset = false;
    }
  }

  async function handleOpenFolder() {
    if (!targetInstance) return;
    try {
      const config = await getConfig();
      await openFolder(`${config.data_dir}/instances/${targetInstance}/mods`);
      pushToast({ kind: 'ok', text: t('mods.openFolderSuccess') || 'Carpeta abierta' });
    } catch {
      pushToast({ kind: 'err', text: t('mods.openFolderError') });
    }
  }

  function onSearchInput() {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      runModrinthSearch();
    }, 450);
  }

  async function runModrinthSearch() {
    const q = searchQuery.trim();
    if (!q || !selectedInstance) {
      if (!q) searchResults = null;
      return;
    }

    const seq = ++searchSeq;
    searching = true;
    searchError = '';

    try {
      const loader = selectedInstance.loader === 'neoforge' ? 'neoforge' : 'fabric';
      const resp = await searchModrinth(q, selectedInstance.version, loader, selectedInstance.name);
      if (seq === searchSeq) {
        searchResults = resp.results;
      }
    } catch {
      if (seq === searchSeq) {
        searchError = t('mods.searchError');
        searchResults = null;
      }
    } finally {
      if (seq === searchSeq) {
        searching = false;
      }
    }
  }

  async function handleInstallFromModrinth(project: ModrinthProject) {
    if (!targetInstance) return;
    installingProjectId = project.project_id;
    try {
      await installModrinthMod(targetInstance, project.project_id);
      pushToast({
        kind: 'info',
        text: `${t('mods.installingMod')} ${project.title}…`,
      });

      setTimeout(() => {
        installingProjectId = null;
        loadMods();
      }, 2500);
    } catch (e) {
      pushToast({
        kind: 'err',
        text: e instanceof Error ? e.message : 'Error installing mod',
      });
      installingProjectId = null;
    }
  }
  function modDisplayName(mod: { filename: string; project_slug?: string }): string {
    return mod.filename.replace(/\.jar$/i, '') || mod.project_slug || '';
  }


  function depBadgesFor(slug?: string) {
    if (!slug) return [];
    const pin = PINNED_SET.find((p) => p.slug === slug);
    if (!pin) return [];
    const badges: { text: string; tone: 'accent' | 'ok' | 'warn' }[] = [];
    for (const req of pin.requires ?? []) {
      badges.push({ text: t('mods.requires', { mods: req }), tone: 'warn' });
    }
    for (const by of pin.requiredBy ?? []) {
      badges.push({ text: t('mods.requiredBy', { mods: by }), tone: 'ok' });
    }
    return badges;
  }
</script>

<svelte:head>
  <title>{t('mods.tag')}</title>
</svelte:head>

<div class="mod-catalog" in:flyY={{ y: 8, duration: 180 }}>
  <!-- Page Header -->
  <header class="catalog-header">
    <div class="catalog-header__main">
      <div class="catalog-header__tagline">
        <span class="catalog-header__dot"></span>
        <span class="font-pixel text-xs">{t('mods.kicker')}</span>
      </div>
      <h1 class="catalog-title">
        <GradientText tone="emerald">{t('mods.title')}</GradientText>
      </h1>
      <p class="catalog-subhead muted">
        {t('mods.subhead')}
      </p>
    </div>

    <div class="catalog-header__actions">
      <!-- Density Segmented Control -->
      <div class="density-control" role="group" aria-label="List density">
        <button
          type="button"
          class="density-btn"
          class:active={density === 'spacious'}
          onclick={() => setDensity('spacious')}
          title={t('mods.densitySpacious')}
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="3" width="18" height="7" rx="2" />
            <rect x="3" y="14" width="18" height="7" rx="2" />
          </svg>
          <span class="density-btn__label">{t('mods.densitySpacious')}</span>
        </button>
        <button
          type="button"
          class="density-btn"
          class:active={density === 'compact'}
          onclick={() => setDensity('compact')}
          title={t('mods.densityCompact')}
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="3" y1="5" x2="21" y2="5" />
            <line x1="3" y1="10" x2="21" y2="10" />
            <line x1="3" y1="15" x2="21" y2="15" />
            <line x1="3" y1="20" x2="21" y2="20" />
          </svg>
          <span class="density-btn__label">{t('mods.densityCompact')}</span>
        </button>
      </div>

      <Btn
        variant="secondary"
        size="sm"
        onclick={handleOpenFolder}
        disabled={!targetInstance}
        title={t('mods.openFolder')}
      >
        {#snippet icon()}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
        {/snippet}
        {t('mods.openFolder')}
      </Btn>
    </div>
  </header>

  <!-- Instance Selector Strip -->
  <section class="instance-strip-card glass-panel" aria-label="Instance selector">
    <div class="instance-strip-header">
      <span class="instance-strip-label">{t('mods.instance')}</span>
      {#if selectedInstance}
        <div class="instance-strip-meta">
          <LoaderBadge loader={selectedInstance.loader} version={selectedInstance.version} size="sm" />
          <Badge variant="neutral" mono>{mods.length} {t('mods.count', { count: mods.length })}</Badge>
        </div>
      {/if}
    </div>

    {#if $instances.value.length === 0}
      <div class="instance-strip-empty">
        <p class="muted">{t('mods.noInstances')}</p>
        <Btn variant="primary" size="sm" onclick={() => (window.location.hash = '#/instances')}>
          {t('mods.createFirst')}
        </Btn>
      </div>
    {:else}
      <div class="instance-chips-scroll">
        {#each $instances.value as inst (inst.name)}
          {@const isSelected = inst.name === targetInstance}
          <button
            type="button"
            class="instance-chip"
            class:selected={isSelected}
            onclick={() => {
              targetInstance = inst.name;
            }}
          >
            <span class="instance-chip__glow"></span>
            <span class="instance-chip__name">{inst.name}</span>
            <span class="instance-chip__tag">{inst.loader} · {inst.version}</span>
            {#if isSelected}
              <span class="instance-chip__active-dot"></span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <!-- Active Preset Install Progress SSE Banner -->
  {#if installProgress}
    <div class="install-progress-deck glass-panel" in:flyY={{ y: -6, duration: 160 }}>
      <div class="install-progress-info">
        <div class="install-progress-title">
          <span class="spinner-pulse"></span>
          <span>{installProgress.filename ? t('mods.downloadingFile', { file: installProgress.filename }) : t('mods.downloading')}</span>
        </div>
        <span class="install-progress-stat font-pixel text-xs">
          {installProgress.done} / {installProgress.total}
        </span>
      </div>
      <ProgressBar
        value={installProgress.done}
        max={Math.max(installProgress.total, 1)}
        height="sm"
        tone="accent"
      />
    </div>
  {/if}

  <!-- Navigation View Tabs -->
  <nav class="catalog-tabs" aria-label="Catalog sections">
    <button
      type="button"
      class="catalog-tab-btn"
      class:active={activeTab === 'installed'}
      onclick={() => (activeTab = 'installed')}
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
        <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
        <line x1="12" y1="22.08" x2="12" y2="12" />
      </svg>
      <span>{t('mods.installedTab')}</span>
      <span class="catalog-tab-badge">{mods.length}</span>
    </button>

    <button
      type="button"
      class="catalog-tab-btn"
      class:active={activeTab === 'presets'}
      onclick={() => (activeTab = 'presets')}
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
      </svg>
      <span>{t('mods.presetsTab')}</span>
      {#if perfMissing.length > 0}
        <span class="catalog-tab-badge catalog-tab-badge--warn">{perfMissing.length}</span>
      {/if}
    </button>

    <button
      type="button"
      class="catalog-tab-btn"
      class:active={activeTab === 'modrinth'}
      onclick={() => (activeTab = 'modrinth')}
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm1 14.93V17h-2v-.07A6.006 6.006 0 0 1 6.07 12H6v-2h.07A6.006 6.006 0 0 1 11 5.07V5h2v.07A6.006 6.006 0 0 1 17.93 10H18v2h-.07A6.006 6.006 0 0 1 13 16.93z" />
      </svg>
      <span>{t('mods.modrinthTab')}</span>
    </button>
  </nav>

  <!-- View 1: Installed Mods Inventory -->
  {#if activeTab === 'installed'}
    <div class="tab-pane" in:fade={{ duration: 120 }}>
      <!-- Toolbar Filter Strip -->
      <div class="inventory-toolbar glass-panel">
        <div class="inventory-filters">
          <button
            type="button"
            class="filter-chip"
            class:active={filterStatus === 'all'}
            onclick={() => (filterStatus = 'all')}
          >
            {t('mods.filterAll')} ({mods.length})
          </button>
          <button
            type="button"
            class="filter-chip"
            class:active={filterStatus === 'enabled'}
            onclick={() => (filterStatus = 'enabled')}
          >
            <span class="dot dot--ok"></span>
            {t('mods.filterEnabled')} ({enabledCount})
          </button>
          <button
            type="button"
            class="filter-chip"
            class:active={filterStatus === 'disabled'}
            onclick={() => (filterStatus = 'disabled')}
          >
            <span class="dot dot--muted"></span>
            {t('mods.filterDisabled')} ({disabledCount})
          </button>
        </div>

        <div class="inventory-search">
          <svg class="search-lens" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="text"
            class="inventory-search-input"
            placeholder="Filtrar por nombre o archivo…"
            bind:value={filterQuery}
          />
          {#if filterQuery}
            <button
              type="button"
              class="search-clear"
              onclick={() => (filterQuery = '')}
              title="Limpiar"
            >
              ×
            </button>
          {/if}
        </div>
      </div>

      <!-- Mod Rows Feed -->
      {#if loadingMods && mods.length === 0}
        <div class="loading-state glass-panel">
          <div class="spinner-pulse"></div>
          <p class="muted">{t('mods.loading')}</p>
        </div>
      {:else if modsError}
        <div class="error-banner glass-panel">
          <span class="error-icon">!</span>
          <span>{modsError}</span>
          <Btn variant="ghost" size="sm" onclick={loadMods}>Reintentar</Btn>
        </div>
      {:else if filteredMods.length === 0}
        <div class="empty-state glass-panel">
          <div class="empty-state__icon">📦</div>
          <h3>{mods.length === 0 ? t('mods.noMods') : t('mods.noResults')}</h3>
          <p class="muted">
            {mods.length === 0
              ? 'Instala el conjunto de rendimiento o explora Modrinth para añadir mods.'
              : 'Prueba con otros términos de búsqueda o cambia los filtros de estado.'}
          </p>
          {#if mods.length === 0}
            <Btn
              variant="primary"
              size="md"
              onclick={() => (activeTab = 'presets')}
            >
              {t('mods.installPerformance')}
            </Btn>
          {/if}
        </div>
      {:else}
        <div class="mods-feed" class:density-compact={density === 'compact'}>
          {#each filteredMods as mod (mod.filename)}
            {@const depBadges = depBadgesFor(mod.project_slug)}
            <div class="mod-entry glass-panel" class:mod-entry--disabled={!mod.enabled}>
              <div class="mod-entry__media">
                <MonogramTile name={modDisplayName(mod)} size={density === 'compact' ? 32 : 42} shape="rounded" />
              </div>

              <div class="mod-entry__info">
                <div class="mod-entry__title-row">
                  <span class="mod-entry__name">{modDisplayName(mod)}</span>
                  {#if mod.version_number}
                    <span class="mod-entry__ver badge badge--neutral badge--sm font-pixel">{mod.version_number}</span>
                  {/if}
                  {#if mod.project_slug}
                    <span class="mod-entry__slug font-mono">{mod.project_slug}</span>
                  {/if}
                </div>

                <div class="mod-entry__meta-row muted">
                  <span class="mod-entry__filename font-mono">{mod.filename}</span>
                  {#if mod.size}
                    <span class="mod-entry__size">· {formatBytes(mod.size)}</span>
                  {/if}
                  {#if mod.sha1}
                    <span class="mod-entry__sha1 font-mono">· {mod.sha1.slice(0, 8)}</span>
                  {/if}
                </div>

                {#if depBadges.length > 0}
                  <div class="mod-entry__deps">
                    {#each depBadges as badge}
                      <Badge tone={badge.tone} size="sm">{badge.text}</Badge>
                    {/each}
                  </div>
                {/if}
              </div>

              <div class="mod-entry__actions">
                <button
                  type="button"
                  class="switch"
                  class:on={mod.enabled}
                  role="switch"
                  aria-checked={mod.enabled}
                  aria-label={modDisplayName(mod)}
                  onclick={() => toggleModOptimistic(mod)}
                  title={mod.enabled ? t('mods.disabled') : t('mods.enabled')}
                >
                  <span class="switch__thumb"></span>
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- View 2: Presets & Recommended Bundles -->
  {#if activeTab === 'presets'}
    <div class="tab-pane" in:fade={{ duration: 120 }}>
      {#if presetInfo && !presetInfo.supported && presetInfo.note}
        <div class="note-banner glass-panel">
          <span class="note-icon">ℹ</span>
          <span>{presetInfo.note}</span>
        </div>
      {/if}

      <div class="presets-grid">
        <!-- Preset 1: Performance Bundle -->
        <GlassCard featured={true} elevation="md">
          <div class="preset-card-head">
            <div class="preset-card-title-group">
              <span class="preset-icon-badge preset-icon-badge--emerald">⚡</span>
              <div>
                <h3 class="preset-title">{t('mods.performance')}</h3>
                <span class="preset-subtitle font-pixel text-xs">{PINNED_SET.length} mods verificados</span>
              </div>
            </div>
            {#if perfMissing.length === 0}
              <Badge tone="ok" dot>{t('mods.upToDate')}</Badge>
            {:else}
              <Badge tone="warn" dot>{perfMissing.length} pendientes</Badge>
            {/if}
          </div>

          <p class="preset-desc muted">
            {t('mods.performanceDesc')}
          </p>

          <div class="pinned-pills">
            {#each PINNED_SET as pin}
              {@const isInstalled = mods.some((m) => m.project_slug === pin.slug && m.installed)}
              <div class="pinned-pill" class:installed={isInstalled}>
                <span class="pinned-pill__dot"></span>
                <span class="font-mono">{pin.slug}</span>
                {#if pin.requires}
                  <span class="pinned-pill__req muted">({pin.requires.join(', ')})</span>
                {/if}
              </div>
            {/each}
          </div>

          {#snippet footerSnippet()}
            <div class="preset-card-footer">
              <span class="preset-target-label muted">
                {t('mods.installTarget', { name: targetInstance || '—' })}
              </span>
              <Btn
                variant="primary"
                size="md"
                onclick={() => installPreset('performance')}
                loading={installingPreset}
                disabled={installingPreset || !presetAvailable}
              >
                {perfMissing.length > 0 ? t('mods.installUpdate') : t('mods.installPerformance')}
              </Btn>
            </div>
          {/snippet}
        </GlassCard>

        <!-- Preset 2: Espectral Menu Branding -->
        <GlassCard elevation="md">
          <div class="preset-card-head">
            <div class="preset-card-title-group">
              <span class="preset-icon-badge preset-icon-badge--cyan">✨</span>
              <div>
                <h3 class="preset-title">{t('mods.branding')}</h3>
                <span class="preset-subtitle font-pixel text-xs">Propio & Ligero</span>
              </div>
            </div>
          </div>

          <p class="preset-desc muted">
            {t('mods.brandingDesc')}
          </p>

          {#if presetInfo?.branding?.note}
            <div class="preset-mini-note muted font-mono text-xs">
              {presetInfo.branding.note}
            </div>
          {/if}

          {#snippet footerSnippet()}
            <div class="preset-card-footer">
              <span class="preset-target-label muted">
                {targetInstance}
              </span>
              <Btn
                variant="secondary"
                size="md"
                onclick={() => installPreset('branding')}
                loading={installingPreset}
                disabled={installingPreset || !brandingAvailable}
              >
                {t('mods.installBranding')}
              </Btn>
            </div>
          {/snippet}
        </GlassCard>

        <!-- Preset 3: Quality of Life (QoL) -->
        <GlassCard elevation="md">
          <div class="preset-card-head">
            <div class="preset-card-title-group">
              <span class="preset-icon-badge preset-icon-badge--purple">🔮</span>
              <div>
                <h3 class="preset-title">{t('mods.qolTitle')}</h3>
                <span class="preset-subtitle font-pixel text-xs">Brillo & Claridad</span>
              </div>
            </div>
          </div>

          {#if presetInfo?.loader === 'vanilla'}
            <p class="preset-desc muted">
              {t('mods.qolVanillaNote')}
            </p>
          {:else}
            <p class="preset-desc muted">
              {t('mods.qolDesc')}
            </p>

            <div class="pinned-pills">
              {#each QOL_SET as pin}
                {@const isInstalled = mods.some((m) => m.project_slug === pin.slug && m.installed)}
                <div class="pinned-pill" class:installed={isInstalled}>
                  <span class="pinned-pill__dot"></span>
                  <span class="font-mono">{pin.slug}</span>
                </div>
              {/each}
            </div>
          {/if}

          {#snippet footerSnippet()}
            <div class="preset-card-footer">
              <span class="preset-target-label muted">
                {targetInstance}
              </span>
              <Btn
                variant="secondary"
                size="md"
                onclick={() => installPreset('qol')}
                loading={installingPreset}
                disabled={installingPreset || !presetAvailable || presetInfo?.loader === 'vanilla'}
              >
                {t('mods.installQol')}
              </Btn>
            </div>
          {/snippet}
        </GlassCard>
      </div>
    </div>
  {/if}

  <!-- View 3: Modrinth Search & Discover -->
  {#if activeTab === 'modrinth'}
    <div class="tab-pane" in:fade={{ duration: 120 }}>
      <div class="modrinth-search-header glass-panel">
        <div class="modrinth-chip">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="#1bd96a">
            <path d="M12.252.004a11.78 11.78 0 0 0-2.5.266A12.3 12.3 0 0 0 0 12.055a12.3 12.3 0 0 0 9.752 11.785 11.78 11.78 0 0 0 2.5.266c6.8 0 12.3-5.5 12.3-12.3s-5.5-12.3-12.3-12.3zm0 2.05c5.66 0 10.25 4.59 10.25 10.25s-4.59 10.25-10.25 10.25S2 17.964 2 12.304 6.59 2.054 12.252 2.054zm-2.05 4.1v4.1H6.102v4.1h4.1v4.1h4.1v-4.1h4.1v-4.1h-4.1v-4.1z" />
          </svg>
          <span class="font-pixel text-xs">Modrinth Matrix</span>
        </div>

        <div class="modrinth-search-input-wrap">
          <input
            type="text"
            class="modrinth-input"
            placeholder={t('mods.searchPlaceholder')}
            bind:value={searchQuery}
            oninput={onSearchInput}
            onkeydown={(e) => {
              if (e.key === 'Enter') runModrinthSearch();
            }}
          />
          <Btn
            variant="primary"
            size="sm"
            onclick={runModrinthSearch}
            loading={searching}
            disabled={!selectedInstance || searching}
          >
            {t('mods.search')}
          </Btn>
        </div>

        {#if selectedInstance}
          <div class="modrinth-target-hint muted font-mono text-xs">
            {t('mods.searchHint', { loader: selectedInstance.loader, version: selectedInstance.version })}
          </div>
        {/if}
      </div>

      <!-- Modrinth Results -->
      {#if searchError}
        <div class="error-banner glass-panel">
          <span>{searchError}</span>
        </div>
      {/if}

      {#if searching}
        <div class="loading-state glass-panel">
          <div class="spinner-pulse"></div>
          <p class="muted">{t('mods.searching')}</p>
        </div>
      {:else if searchResults !== null}
        {#if searchResults.length === 0}
          <div class="empty-state glass-panel">
            <div class="empty-state__icon">🔍</div>
            <h3>{t('mods.noResults')}</h3>
            <p class="muted">Intenta buscar con palabras clave más generales en inglés o nombres exactos.</p>
          </div>
        {:else}
          <div class="modrinth-grid">
            {#each searchResults as project (project.project_id)}
              <div class="modrinth-card glass-panel">
                <div class="modrinth-card__top">
                  {#if project.icon_url}
                    <img class="modrinth-card__icon" src={project.icon_url} alt="" loading="lazy" />
                  {:else}
                    <MonogramTile name={project.title} size={44} shape="rounded" />
                  {/if}

                  <div class="modrinth-card__headings">
                    <h4 class="modrinth-card__title">{project.title}</h4>
                    <span class="modrinth-card__downloads muted">
                      {t('mods.downloads', { count: project.downloads.toLocaleString() })}
                    </span>
                  </div>
                </div>

                <p class="modrinth-card__desc muted">
                  {project.description}
                </p>

                <div class="modrinth-card__footer">
                  {#if project.installed}
                    <Badge tone="ok">{t('mods.alreadyInstalled')}</Badge>
                  {:else}
                    <Btn
                      variant="secondary"
                      size="sm"
                      onclick={() => handleInstallFromModrinth(project)}
                      loading={installingProjectId === project.project_id}
                      disabled={installingProjectId !== null}
                    >
                      {installingProjectId === project.project_id ? t('mods.installingMod') : t('mods.install')}
                    </Btn>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {:else}
        <!-- Initial Modrinth discover landing -->
        <div class="modrinth-discover-hint glass-panel">
          <div class="discover-spark">✨</div>
          <h3>Explora miles de mods optimizados</h3>
          <p class="muted">
            Escribe en la barra superior para buscar shaders, minimapas, optimizaciones y herramientas compatibles con tu versión activa.
          </p>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* ==========================================================================
     ModCatalog.svelte Layout & Styles
     ========================================================================== */
  .mod-catalog {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 1rem);
    width: 100%;
    max-width: 1360px;
    margin: 0 auto;
    padding-bottom: var(--space-8, 2rem);
  }

  /* Header */
  .catalog-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 1rem);
    flex-wrap: wrap;
  }

  .catalog-header__tagline {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--accent, #10b981);
    margin-bottom: 0.25rem;
  }

  .catalog-header__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    box-shadow: 0 0 8px var(--accent, #10b981);
  }

  .catalog-title {
    font-size: var(--text-2xl, 1.75rem);
    font-weight: 700;
    line-height: var(--leading-tight, 1.2);
  }

  .catalog-subhead {
    font-size: var(--text-sm, 0.875rem);
    margin-top: 0.25rem;
  }

  .catalog-header__actions {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
  }

  /* Density Switcher */
  .density-control {
    display: flex;
    align-items: center;
    background: rgba(var(--bg-rgb, 6, 10, 20), 0.6);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 0.5rem);
    padding: 2px;
  }

  .density-btn {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--transition-fast, 0.15s);
  }

  .density-btn:hover {
    color: var(--text, #e8ecf4);
  }

  .density-btn.active {
    background: rgba(255, 255, 255, 0.1);
    color: var(--accent, #10b981);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  }

  /* Instance Selector Strip */
  .instance-strip-card {
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }

  .instance-strip-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .instance-strip-label {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
  }

  .instance-strip-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .instance-chips-scroll {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    overflow-x: auto;
    padding-bottom: 2px;
    scrollbar-width: thin;
  }

  .instance-chip {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.85rem;
    border-radius: var(--radius-md, 0.5rem);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    color: var(--text, #e8ecf4);
    font-size: 0.8125rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: all var(--transition-fast, 0.15s);
  }

  .instance-chip:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.3);
  }

  .instance-chip.selected {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.12);
    border-color: var(--accent, #10b981);
    color: #ffffff;
    box-shadow: 0 0 16px rgba(var(--accent-rgb, 16, 185, 129), 0.2);
  }

  .instance-chip__tag {
    font-size: 0.6875rem;
    font-weight: 400;
    color: var(--muted, #8e9eb8);
  }

  .instance-chip.selected .instance-chip__tag {
    color: rgba(255, 255, 255, 0.75);
  }

  .instance-chip__active-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    box-shadow: 0 0 6px var(--accent, #10b981);
  }

  .instance-strip-empty {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0;
  }

  /* Progress Deck */
  .install-progress-deck {
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.08);
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.3);
  }

  .install-progress-info {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 0.8125rem;
  }

  .install-progress-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  /* Catalog View Tabs */
  .catalog-tabs {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    border-bottom: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    padding-bottom: 0.25rem;
  }

  .catalog-tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0.5rem 0.9rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast, 0.15s);
  }

  .catalog-tab-btn:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.04);
  }

  .catalog-tab-btn.active {
    color: var(--accent, #10b981);
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.1);
  }

  .catalog-tab-badge {
    font-size: 0.6875rem;
    padding: 0.1rem 0.4rem;
    border-radius: 9999px;
    background: rgba(255, 255, 255, 0.08);
    color: var(--text, #e8ecf4);
  }

  .catalog-tab-badge--warn {
    background: rgba(var(--accent-alt-rgb, 245, 158, 11), 0.2);
    color: var(--accent-alt, #f59e0b);
  }

  /* Inventory Toolbar */
  .inventory-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 0.75rem;
    margin-bottom: var(--space-3, 0.75rem);
    flex-wrap: wrap;
  }

  .inventory-filters {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: transparent;
    border: 1px solid transparent;
    color: var(--muted, #8e9eb8);
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--transition-fast, 0.15s);
  }

  .filter-chip:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.04);
  }

  .filter-chip.active {
    background: rgba(255, 255, 255, 0.08);
    border-color: var(--border, rgba(255, 255, 255, 0.12));
    color: var(--text, #e8ecf4);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .dot--ok {
    background: var(--accent, #10b981);
    box-shadow: 0 0 6px var(--accent, #10b981);
  }

  .dot--muted {
    background: var(--muted, #8e9eb8);
  }

  .inventory-search {
    position: relative;
    display: flex;
    align-items: center;
    min-width: 240px;
  }

  .search-lens {
    position: absolute;
    left: 0.6rem;
    color: var(--muted, #8e9eb8);
    pointer-events: none;
  }

  .inventory-search-input {
    width: 100%;
    padding: 0.35rem 1.8rem 0.35rem 2rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    color: var(--text, #e8ecf4);
    font-size: 0.8125rem;
  }

  .inventory-search-input:focus {
    border-color: var(--accent, #10b981);
    outline: none;
  }

  .search-clear {
    position: absolute;
    right: 0.5rem;
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 1.1rem;
    cursor: pointer;
  }

  /* Mods Feed */
  .mods-feed {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .mods-feed.density-compact .mod-entry {
    padding: 0.35rem 0.6rem;
  }

  .mods-feed.density-compact .mod-entry__name {
    font-size: 0.8125rem;
  }

  .mod-entry {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.65rem 0.85rem;
    transition: all var(--transition-fast, 0.15s);
  }

  .mod-entry--disabled {
    opacity: 0.6;
  }

  .mod-entry__info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .mod-entry__title-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .mod-entry__name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--text, #e8ecf4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mod-entry__slug {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
  }

  .mod-entry__meta-row {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.75rem;
    flex-wrap: wrap;
  }

  .mod-entry__filename {
    opacity: 0.85;
  }

  .mod-entry__deps {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    margin-top: 0.2rem;
    flex-wrap: wrap;
  }

  .mod-entry__actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  /* Presets Grid */
  .presets-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: var(--space-4, 1rem);
  }

  .preset-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .preset-card-title-group {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .preset-icon-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-md, 0.5rem);
    font-size: 1rem;
  }

  .preset-icon-badge--emerald {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.15);
    border: 1px solid rgba(var(--accent-rgb, 16, 185, 129), 0.3);
  }

  .preset-icon-badge--cyan {
    background: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.15);
    border: 1px solid rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.3);
  }

  .preset-icon-badge--purple {
    background: rgba(var(--accent-purple-rgb, 168, 85, 247), 0.15);
    border: 1px solid rgba(var(--accent-purple-rgb, 168, 85, 247), 0.3);
  }

  .preset-title {
    font-size: 1rem;
    font-weight: 700;
  }

  .preset-subtitle {
    color: var(--muted, #8e9eb8);
  }

  .preset-desc {
    font-size: 0.8125rem;
    line-height: var(--leading-body, 1.6);
    margin-bottom: 0.75rem;
  }

  .pinned-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-bottom: 0.75rem;
  }

  .pinned-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.5rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
  }

  .pinned-pill.installed {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.1);
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.3);
    color: var(--text, #e8ecf4);
  }

  .pinned-pill__dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--muted, #8e9eb8);
  }

  .pinned-pill.installed .pinned-pill__dot {
    background: var(--accent, #10b981);
    box-shadow: 0 0 6px var(--accent, #10b981);
  }

  .preset-card-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  .preset-target-label {
    font-size: 0.75rem;
  }

  /* Modrinth Search */
  .modrinth-search-header {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: var(--space-4, 1rem);
    margin-bottom: var(--space-4, 1rem);
  }

  .modrinth-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    color: #1bd96a;
  }

  .modrinth-search-input-wrap {
    display: flex;
    gap: 0.5rem;
  }

  .modrinth-input {
    flex: 1;
    padding: 0.5rem 0.85rem;
    border-radius: var(--radius-md, 0.5rem);
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    color: var(--text, #e8ecf4);
    font-size: 0.875rem;
  }

  .modrinth-input:focus {
    border-color: #1bd96a;
    box-shadow: 0 0 12px rgba(27, 217, 106, 0.25);
    outline: none;
  }

  .modrinth-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
    gap: var(--space-4, 1rem);
  }

  .modrinth-card {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: var(--space-4, 1rem);
    min-height: 180px;
  }

  .modrinth-card__top {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }

  .modrinth-card__icon {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md, 0.5rem);
    object-fit: cover;
    background: rgba(0, 0, 0, 0.3);
  }

  .modrinth-card__headings {
    flex: 1;
    min-width: 0;
  }

  .modrinth-card__title {
    font-size: 0.9375rem;
    font-weight: 700;
    color: var(--text, #e8ecf4);
    line-height: var(--leading-tight, 1.2);
  }

  .modrinth-card__downloads {
    font-size: 0.75rem;
    margin-top: 0.15rem;
    display: inline-block;
  }

  .modrinth-card__desc {
    font-size: 0.8125rem;
    line-height: var(--leading-body, 1.6);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    margin-bottom: 0.75rem;
  }

  .modrinth-card__footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    border-top: 1px solid var(--border, rgba(255, 255, 255, 0.06));
    padding-top: 0.6rem;
  }

  .modrinth-discover-hint {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .discover-spark {
    font-size: 2rem;
  }

  /* Universal Feedback States */
  .loading-state,
  .empty-state,
  .error-banner,
  .note-banner {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .empty-state__icon {
    font-size: 2rem;
  }

  .error-banner {
    background: rgba(var(--accent-red-rgb, 239, 68, 68), 0.1);
    border-color: rgba(var(--accent-red-rgb, 239, 68, 68), 0.3);
    color: var(--accent-red, #ef4444);
    flex-direction: row;
    justify-content: center;
    padding: 0.75rem 1rem;
  }

  .note-banner {
    background: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.1);
    border-color: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.3);
    color: var(--text, #e8ecf4);
    flex-direction: row;
    justify-content: flex-start;
    padding: 0.75rem 1rem;
    margin-bottom: 1rem;
  }

  .spinner-pulse {
    width: 16px;
    height: 16px;
    border: 2px solid var(--accent, #10b981);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Switch Toggle Component */
  .switch {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 42px;
    height: 24px;
    padding: 2px;
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.14));
    border-radius: 9999px;
    cursor: pointer;
    transition: background-color 0.2s cubic-bezier(0.16, 1, 0.3, 1),
      border-color 0.2s cubic-bezier(0.16, 1, 0.3, 1),
      box-shadow 0.2s ease;
    flex-shrink: 0;
  }

  .switch:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  .switch.on {
    background: var(--accent, #10b981);
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.8);
    box-shadow: 0 0 12px rgba(var(--accent-rgb, 16, 185, 129), 0.35);
  }

  .switch__thumb {
    width: 18px;
    height: 18px;
    background: #ffffff;
    border-radius: 50%;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
    transform: translateX(0);
  }

  .switch.on .switch__thumb {
    transform: translateX(18px);
  }

  .switch:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
