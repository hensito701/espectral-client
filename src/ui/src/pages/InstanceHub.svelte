<!--
  ============================================================================
  InstanceHub.svelte — Horizon Glass Console Game Hub & Workbench
  Route: #/instances/:name
  ============================================================================
  Cinematic game hub for an individual Minecraft instance:
  - Header: Back affordance, giant display typography, LoaderBadge, live state,
    and dynamic ambient hue sync.
  - Action Deck: TelemetryCapsule flight deck.
  - Tabbed Workbench: Configuration, Servers, Mods, Danger Zone.
  - Live Log Console: Compact launch log view with PiP HUD toggle.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import {
    getInstance,
    patchInstance,
    getInstanceServers,
    putInstanceServers,
    importInstanceOptions,
    getAotStatus,
    trainAot,
    getInstanceClient,
    patchInstanceClient,
    deleteInstance,
    getJvm,
    installMods,
    getModsPresetInfo,
    openFolder,
    subscribeEvents,
  } from '../lib/api';
  import { instances, liveLaunches, launchLog, servers as networkServers } from '../lib/stores';
  import type {
    InstanceSummary,
    ServerEntry,
    OptionsPair,
    AotStatus,
    ClientInfo,
    JvmInfo,
    ModPresetInfo,
  } from '../lib/types';
  import TelemetryCapsule from '../components/TelemetryCapsule.svelte';
  import LoaderBadge from '../components/LoaderBadge.svelte';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import Field from '../components/Field.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import { memLabel } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { flyY, fade } from '../lib/motion';

  // Props
  let { name = '' }: { name?: string } = $props();

  const decodedName = $derived(decodeURIComponent(name));

  // Loader Hue lookup for ambient stage sync
  const LOADER_HUES: Record<string, number> = {
    vanilla: 160,
    fabric: 195,
    neoforge: 25,
    forge: 215,
    quilt: 280,
  };

  // State
  let loading = $state(true);
  let error = $state('');
  let summary = $state<InstanceSummary | null>(null);
  let jvm = $state<JvmInfo | null>(null);
  let presetInfo = $state<ModPresetInfo | null>(null);

  // Active Tab
  type WorkbenchTab = 'config' | 'servers' | 'mods' | 'danger';
  let activeTab = $state<WorkbenchTab>('config');

  // Config tab state
  let memory = $state(3072);
  let jdkOverride = $state('');
  let aotAutoTrain = $state(false);
  let savingConfig = $state(false);
  let aotStatus = $state<AotStatus | null>(null);
  let trainingAot = $state(false);
  let trainProgress = $state<{ done: number; total: number; phase?: string } | null>(null);

  // Options import state
  let options = $state<OptionsPair[]>([]);
  let importOptionsFrom = $state('');
  let importingOptions = $state(false);
  let optionsNote = $state('');

  // Client Suite state
  let clientInfo = $state<ClientInfo | null>(null);
  let togglingFeature = $state('');

  // Servers tab state
  let servers = $state<ServerEntry[]>([]);
  let editingServers = $state(false);
  let serverDraft = $state<ServerEntry[]>([]);
  let serverErr = $state('');
  let savingServers = $state(false);

  // Mods tab state
  let installProgress = $state<{ filename: string; done: number; total: number } | null>(null);
  let installingPreset = $state(false);

  // Danger tab state
  let deleting = $state(false);

  // Log container ref for auto-scroll
  let logContainer = $state<HTMLDivElement | undefined>(undefined);
  let stickToBottom = $state(true);

  // Running status derived from liveLaunches store
  const isInstanceRunning = $derived(
    Boolean(
      (summary as (InstanceSummary & { running?: boolean }) | null)?.running ||
        $liveLaunches.value.some((l) => l.instance === decodedName && l.running)
    )
  );

  // Log buffer derived from launchLog store for this instance
  const instanceLogBuffer = $derived.by(() => {
    const buffers = Object.values($launchLog.buffers);
    const matching = buffers.filter((b) => b.instance === decodedName);
    if (matching.length > 0) return matching[matching.length - 1];
    if ($launchLog.key && $launchLog.buffers[$launchLog.key]?.instance === decodedName) {
      return $launchLog.buffers[$launchLog.key];
    }
    return null;
  });

  const logLines = $derived(instanceLogBuffer?.lines ?? ($launchLog.lines.length > 0 ? $launchLog.lines : []));
  const isLogRunning = $derived(Boolean(instanceLogBuffer?.running || $launchLog.running));

  // Dynamic ambient hue shift
  $effect(() => {
    if (typeof document !== 'undefined' && summary?.loader) {
      const hue = LOADER_HUES[summary.loader.toLowerCase()] ?? 160;
      document.documentElement.style.setProperty('--ambient-hue', String(hue));
    }
  });

  onDestroy(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.style.removeProperty('--ambient-hue');
    }
  });

  // Load all instance data
  async function loadData(inst: string) {
    if (!inst) return;
    loading = true;
    error = '';
    optionsNote = '';
    serverErr = '';
    try {
      const [detail, jvmData, clientData] = await Promise.all([
        getInstance(inst),
        getJvm().catch(() => null),
        getInstanceClient(inst).catch(() => null),
      ]);

      summary = detail.summary;
      memory = detail.summary.memory_mb;
      servers = detail.servers;
      options = detail.options;
      aotStatus = detail.aot;
      jvm = jvmData;
      clientInfo = clientData;

      // Extract existing config values
      if ('jdk_path_override' in detail.summary) {
        jdkOverride = (detail.summary as unknown as { jdk_path_override?: string }).jdk_path_override || '';
      }
      if ('aot_auto_train' in detail.summary) {
        aotAutoTrain = Boolean((detail.summary as unknown as { aot_auto_train?: boolean }).aot_auto_train);
      }

      // Check preset info
      getModsPresetInfo(inst)
        .then((p) => (presetInfo = p))
        .catch(() => (presetInfo = null));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // Effect to re-load on instance param change
  $effect(() => {
    const inst = decodedName;
    if (inst) {
      void loadData(inst);
    }
  });

  // Subscribe to real-time events (train progress, mod installation, launch lifecycle)
  $effect(() => {
    if (!decodedName) return;
    const unsub = subscribeEvents(({ type, data }) => {
      const payload = (data ?? {}) as Record<string, unknown>;

      // AOT training progress
      if (type === 'train-progress') {
        trainingAot = true;
        trainProgress = {
          done: typeof payload.done === 'number' ? payload.done : 0,
          total: typeof payload.total === 'number' ? payload.total : 100,
          phase: typeof payload.phase === 'string' ? payload.phase : undefined,
        };
      } else if (type === 'train-done') {
        trainingAot = false;
        trainProgress = null;
        if (payload.ok === false) {
          pushToast({
            kind: 'err',
            text: t('instance.trainAotError', { error: String(payload.error || 'Unknown error') }),
          });
        } else {
          pushToast({ kind: 'ok', text: t('instance.trainAotSuccess') });
          void getAotStatus(decodedName).then((st) => (aotStatus = st)).catch(() => {});
          void $instances.refresh();
        }
      }

      // Mod installation progress
      if (type === 'mod-progress') {
        installProgress = {
          filename: typeof payload.filename === 'string' ? payload.filename : '',
          done: typeof payload.done === 'number' ? payload.done : 0,
          total: typeof payload.total === 'number' ? payload.total : 0,
        };
      }

      // Refresh on launch exit
      if (type === 'launch-exit') {
        if (payload.instance === decodedName || !payload.instance) {
          void $instances.refresh();
          void $liveLaunches.refresh();
          void getInstanceClient(decodedName).then((c) => (clientInfo = c)).catch(() => {});
        }
      }
    });

    return () => unsub();
  });

  // --- ACTIONS ---

  // Memory & JVM Save
  async function handleSaveMemory() {
    if (!decodedName || savingConfig) return;
    savingConfig = true;
    try {
      summary = await patchInstance(decodedName, { memory_mb: memory });
      await $instances.refresh();
      pushToast({ kind: 'ok', text: t('instance.memorySaved', { mem: memLabel(memory) }) });
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    } finally {
      savingConfig = false;
    }
  }

  async function handleSaveJdk() {
    if (!decodedName || savingConfig) return;
    savingConfig = true;
    try {
      summary = await patchInstance(decodedName, {
        jdk_path_override: jdkOverride.trim() || '',
      });
      await $instances.refresh();
      pushToast({ kind: 'ok', text: t('instance.jdkSaved') });
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    } finally {
      savingConfig = false;
    }
  }

  async function handleToggleAotAutoTrain() {
    if (!decodedName) return;
    try {
      const next = !aotAutoTrain;
      aotAutoTrain = next;
      summary = await patchInstance(decodedName, { aot_auto_train: next });
      await $instances.refresh();
    } catch (e) {
      aotAutoTrain = !aotAutoTrain;
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    }
  }

  // AOT Manual Training
  async function handleTrainAot() {
    if (!decodedName || trainingAot) return;
    trainingAot = true;
    trainProgress = { done: 0, total: 100, phase: 'init' };
    pushToast({ kind: 'info', text: t('instance.trainAotStarted') });
    try {
      await trainAot(decodedName);
    } catch (e) {
      trainingAot = false;
      trainProgress = null;
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    }
  }

  // Options Import
  async function handleImportOptions() {
    if (!decodedName || !importOptionsFrom || importingOptions) return;
    importingOptions = true;
    optionsNote = '';
    try {
      const res = await importInstanceOptions(decodedName, importOptionsFrom);
      options = res.options;
      if (res.copied) {
        optionsNote = t('instance.optionsImported', { source: importOptionsFrom, count: res.count });
        pushToast({ kind: 'ok', text: optionsNote });
      } else {
        optionsNote = t('instance.optionsImportNone');
      }
    } catch (e) {
      optionsNote = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: optionsNote });
    } finally {
      importingOptions = false;
    }
  }

  // Client Feature Toggle
  async function handleToggleFeature(id: string, next: boolean) {
    if (!decodedName || togglingFeature !== '') return;
    togglingFeature = id;
    try {
      const updated = await patchInstanceClient(decodedName, {
        features: { [id]: { enabled: next } },
      });
      if (updated && updated.config) {
        clientInfo = updated;
      }
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    } finally {
      togglingFeature = '';
    }
  }

  // Servers Editing
  function startEditServers() {
    serverDraft = servers.map((s) => ({ ...s }));
    serverErr = '';
    editingServers = true;
  }

  function addServerRow() {
    serverDraft = [
      ...serverDraft,
      { name: '', ip: '', hidden: false, accept_textures: true, has_icon: false },
    ];
  }

  function removeServerRow(index: number) {
    serverDraft = serverDraft.filter((_, idx) => idx !== index);
  }

  async function handleSaveServers() {
    if (savingServers) return;
    serverErr = '';
    const cleaned = serverDraft
      .map((s) => ({ ...s, name: s.name.trim(), ip: s.ip.trim() }))
      .filter((s) => s.name || s.ip);

    for (const s of cleaned) {
      if (s.ip && !/^[^:]+(:\d+)?$/.test(s.ip)) {
        serverErr = t('instance.invalidIp', { ip: s.ip });
        return;
      }
    }

    savingServers = true;
    try {
      await putInstanceServers(decodedName, cleaned);
      servers = await getInstanceServers(decodedName);
      editingServers = false;
      pushToast({ kind: 'ok', text: t('instance.serversSaved') });
    } catch (e) {
      serverErr = e instanceof Error ? e.message : String(e);
    } finally {
      savingServers = false;
    }
  }

  // Mods Performance Preset
  async function handleInstallPerfPreset() {
    if (!decodedName || installingPreset) return;
    installingPreset = true;
    installProgress = { filename: '', done: 0, total: 100 };
    try {
      await installMods(decodedName, 'performance');
      pushToast({ kind: 'ok', text: t('instance.presetInstalled') });
      setTimeout(() => {
        void loadData(decodedName);
      }, 5000);
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    } finally {
      installingPreset = false;
    }
  }

  // Delete Instance
  async function handleDeleteInstance() {
    if (!decodedName || deleting) return;
    deleting = true;
    try {
      await deleteInstance(decodedName);
      await $instances.refresh();
      pushToast({ kind: 'ok', text: t('instance.deleteSuccess', { name: decodedName }) });
      window.location.hash = '#/';
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
      deleting = false;
    }
  }

  // Open Folder
  async function handleOpenFolder() {
    if (!decodedName) return;
    try {
      // Open instance directory via engine api
      await openFolder(decodedName);
    } catch (e) {
      pushToast({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
    }
  }

  // Toggle PiP HUD log
  function handleToggleHud() {
    window.dispatchEvent(new CustomEvent('horizon:toggle-pip-log'));
  }

  // Clear Log
  function handleClearLog() {
    $launchLog.clear();
  }

  // Log auto-scroll
  function onLogScroll() {
    if (!logContainer) return;
    const dist = logContainer.scrollHeight - logContainer.scrollTop - logContainer.clientHeight;
    stickToBottom = dist < 32;
  }

  $effect(() => {
    logLines;
    if (logContainer && stickToBottom) {
      logContainer.scrollTop = logContainer.scrollHeight;
    }
  });

  // Tab keyboard accessibility
  function handleTabKey(e: KeyboardEvent) {
    const tabs: WorkbenchTab[] = ['config', 'servers', 'mods', 'danger'];
    const currentIndex = tabs.indexOf(activeTab);
    let nextIndex = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      e.preventDefault();
      nextIndex = (currentIndex + 1) % tabs.length;
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      e.preventDefault();
      nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
    }
    if (nextIndex !== -1) {
      activeTab = tabs[nextIndex];
      document.getElementById(`tab-${tabs[nextIndex]}`)?.focus();
    }
  }

  function formatBytes(bytes?: number): string {
    if (!bytes || bytes <= 0) return '0 KB';
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
</script>

<svelte:head>
  <title>{t('instance.tag', { name: decodedName || 'Minecraft' })}</title>
</svelte:head>

<div class="instance-hub">
  <!-- Cinematic Header Stage -->
  <header class="hub-header">
    <div class="hub-header__nav">
      <a href="#/" class="back-link">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <polyline points="15 18 9 12 15 6" />
        </svg>
        <span>{t('instance.back')}</span>
      </a>

      {#if summary}
        <button
          type="button"
          class="folder-btn"
          onclick={handleOpenFolder}
          title={t('instance.openFolderTitle')}
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          <span>{t('instance.openFolder')}</span>
        </button>
      {/if}
    </div>

    <div class="hub-header__title-row">
      <div class="hub-header__meta">
        <h1 class="instance-title">{decodedName}</h1>

        {#if summary}
          <div class="meta-badges">
            <LoaderBadge loader={summary.loader} version={summary.version} size="lg" />

            {#if isInstanceRunning}
              <Badge variant="ok" dot={true} size="md">{t('instance.running')}</Badge>
            {:else}
              <Badge variant="neutral" size="md">{t('instance.stopped')}</Badge>
            {/if}

            {#if summary.modpack}
              <Badge variant="gold" size="md">
                {summary.modpack}{summary.modpack_version ? ` ${summary.modpack_version}` : ''}
              </Badge>
            {/if}

            {#if summary.imported_from}
              <Badge variant="purple" size="md" title={t('instance.importedFrom', { source: summary.imported_from })}>
                {t('instance.imported')}
              </Badge>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </header>

  <!-- Flight Deck: Telemetry Capsule -->
  <section class="hub-action-deck" aria-label="Action Deck">
    <TelemetryCapsule instanceName={summary?.name ?? decodedName} running={isInstanceRunning} />
  </section>

  <!-- Main Content State Check -->
  {#if loading}
    <div class="hub-state-card" in:fade>
      <div class="loading-spinner"></div>
      <p class="state-text">{t('instance.loading')}</p>
    </div>
  {:else if error}
    <GlassCard elevation="md" class="hub-state-card hub-state-card--error">
      <div class="state-icon error-icon">✕</div>
      <h2 class="state-title">{t('instance.notFound')}</h2>
      <p class="state-desc">{error || t('instance.notFoundHint')}</p>
      <Btn variant="primary" onclick={() => (window.location.hash = '#/')}>
        {t('instance.back')}
      </Btn>
    </GlassCard>
  {:else if summary}
    <!-- Tabbed Workbench -->
    <div class="hub-workbench" in:flyY={{ y: 15, duration: 220 }}>
      <!-- Tab Navigation Navigation Header -->
      <!-- svelte-ignore a11y_interactive_supports_focus -->
      <div
        class="workbench-tabs"
        role="tablist"
        aria-label="Workbench Sections"
        onkeydown={handleTabKey}
      >
        <button
          type="button"
          role="tab"
          id="tab-config"
          aria-selected={activeTab === 'config'}
          aria-controls="panel-config"
          tabindex={activeTab === 'config' ? 0 : -1}
          class="tab-btn"
          class:tab-btn--active={activeTab === 'config'}
          onclick={() => (activeTab = 'config')}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          <span>{t('instance.tabConfig')}</span>
        </button>

        <button
          type="button"
          role="tab"
          id="tab-servers"
          aria-selected={activeTab === 'servers'}
          aria-controls="panel-servers"
          tabindex={activeTab === 'servers' ? 0 : -1}
          class="tab-btn"
          class:tab-btn--active={activeTab === 'servers'}
          onclick={() => (activeTab = 'servers')}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="2" y="2" width="20" height="8" rx="2" ry="2" />
            <rect x="2" y="14" width="20" height="8" rx="2" ry="2" />
            <line x1="6" y1="6" x2="6.01" y2="6" />
            <line x1="6" y1="18" x2="6.01" y2="18" />
          </svg>
          <span>{t('instance.tabServers')}</span>
          {#if servers.length > 0}
            <span class="tab-count">{servers.length}</span>
          {/if}
        </button>

        <button
          type="button"
          role="tab"
          id="tab-mods"
          aria-selected={activeTab === 'mods'}
          aria-controls="panel-mods"
          tabindex={activeTab === 'mods' ? 0 : -1}
          class="tab-btn"
          class:tab-btn--active={activeTab === 'mods'}
          onclick={() => (activeTab = 'mods')}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
            <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
            <line x1="12" y1="22.08" x2="12" y2="12" />
          </svg>
          <span>{t('instance.tabMods')}</span>
          {#if summary.mod_count > 0}
            <span class="tab-count">{summary.enabled_mod_count}/{summary.mod_count}</span>
          {/if}
        </button>

        <button
          type="button"
          role="tab"
          id="tab-danger"
          aria-selected={activeTab === 'danger'}
          aria-controls="panel-danger"
          tabindex={activeTab === 'danger' ? 0 : -1}
          class="tab-btn tab-btn--danger"
          class:tab-btn--active={activeTab === 'danger'}
          onclick={() => (activeTab = 'danger')}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
          <span>{t('instance.tabDanger')}</span>
        </button>
      </div>

      <!-- Tab 1: CONFIGURATION -->
      {#if activeTab === 'config'}
        <div
          id="panel-config"
          role="tabpanel"
          aria-labelledby="tab-config"
          class="workbench-panel"
          in:fade={{ duration: 150 }}
        >
          <div class="panel-grid">
            <!-- Memory Allocation Card -->
            <GlassCard title={t('instance.memoryTitle')} subtitle={t('instance.memoryHint')} elevation="md">
              <div class="memory-setting">
                <div class="memory-slider-row">
                  <div class="slider-header">
                    <span class="slider-val-tag">{memLabel(memory)}</span>
                    <span class="slider-minmax">1 GB – 16 GB</span>
                  </div>
                  <input
                    type="range"
                    min="1024"
                    max="16384"
                    step="512"
                    bind:value={memory}
                    class="range-slider"
                    aria-label={t('instance.memoryTitle')}
                  />
                </div>

                <div class="memory-presets">
                  {#each [2048, 3072, 4096, 6144, 8192, 12288] as mb}
                    <button
                      type="button"
                      class="preset-pill"
                      class:preset-pill--active={memory === mb}
                      onclick={() => (memory = mb)}
                    >
                      {mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`}
                    </button>
                  {/each}
                </div>

                <div class="card-action-footer">
                  <Btn
                    variant="primary"
                    size="sm"
                    loading={savingConfig}
                    disabled={savingConfig || memory === summary.memory_mb}
                    onclick={handleSaveMemory}
                  >
                    {t('instance.saveMemory')}
                  </Btn>
                </div>
              </div>
            </GlassCard>

            <!-- JVM Environment & Override Card -->
            <GlassCard title={t('instance.jvmTitle')} elevation="md">
              <div class="jvm-details">
                <div class="jvm-spec-grid">
                  <div class="spec-cell">
                    <span class="spec-label">{t('instance.jvmSource')}</span>
                    <Badge variant="accent" size="sm">{jvm?.source ?? 'Bundled'}</Badge>
                  </div>
                  <div class="spec-cell">
                    <span class="spec-label">{t('instance.jvmVersion')}</span>
                    <span class="spec-value">{jvm ? `${jvm.version} (${jvm.vendor})` : '—'}</span>
                  </div>
                </div>

                {#if jvm?.path}
                  <div class="jvm-path-display">
                    <span class="spec-label">{t('instance.jvmPath')}</span>
                    <code class="code-box">{jvm.path}</code>
                  </div>
                {/if}

                <Field
                  label={t('instance.jdkOverrideLabel')}
                  hint={t('instance.jdkOverrideHint')}
                >
                  <input
                    type="text"
                    bind:value={jdkOverride}
                    placeholder={t('instance.jdkOverridePlaceholder')}
                    class="glass-input"
                  />
                </Field>

                <div class="card-action-footer">
                  <Btn
                    variant="secondary"
                    size="sm"
                    loading={savingConfig}
                    disabled={savingConfig}
                    onclick={handleSaveJdk}
                  >
                    {t('instance.saveJdk')}
                  </Btn>
                </div>
              </div>
            </GlassCard>

            <!-- AOT Acceleration Card -->
            <GlassCard title={t('instance.aotTitle')} subtitle={t('instance.aotSubtitle')} elevation="md">
              <div class="aot-settings">
                <div class="aot-stats-grid">
                  <div class="aot-stat-item">
                    <span class="stat-label">{t('instance.aotStatusLabel')}</span>
                    {#if aotStatus?.cache_exists}
                      <Badge variant="ok" dot={true} size="md">{t('instance.aotReady')}</Badge>
                    {:else}
                      <Badge variant="muted" size="md">{t('instance.aotNone')}</Badge>
                    {/if}
                  </div>

                  <div class="aot-stat-item">
                    <span class="stat-label">{t('instance.aotCacheSize')}</span>
                    <span class="stat-num">{formatBytes(aotStatus?.cache_size_bytes)}</span>
                  </div>

                  <div class="aot-stat-item">
                    <span class="stat-label">{t('instance.aotProofLabel')}</span>
                    {#if aotStatus?.proof?.using_aot_linked_classes}
                      <Badge variant="accent" size="sm">{t('instance.aotProofOk')}</Badge>
                    {:else}
                      <span class="stat-desc">{t('instance.aotProofNone')}</span>
                    {/if}
                  </div>
                </div>

                <label class="toggle-checkbox-row">
                  <input
                    type="checkbox"
                    checked={aotAutoTrain}
                    onchange={handleToggleAotAutoTrain}
                    class="glass-checkbox"
                  />
                  <div class="toggle-texts">
                    <span class="toggle-title">{t('instance.aotAutoTrain')}</span>
                    <span class="toggle-hint">{t('instance.aotAutoTrainHint')}</span>
                  </div>
                </label>

                {#if trainProgress}
                  <div class="train-progress-wrap" in:fade>
                    <ProgressBar
                      value={trainProgress.done}
                      max={Math.max(trainProgress.total, 1)}
                      tone="accent"
                      label={t('instance.trainingAot')}
                      showValue={true}
                    />
                  </div>
                {/if}

                <div class="card-action-footer">
                  <Btn
                    variant="secondary"
                    size="sm"
                    loading={trainingAot}
                    disabled={trainingAot || isInstanceRunning}
                    onclick={handleTrainAot}
                  >
                    {t('instance.trainAotBtn')}
                  </Btn>
                </div>
              </div>
            </GlassCard>

            <!-- Options & Preferences Import Card -->
            <GlassCard
              title={t('instance.optionsTitle')}
              subtitle={t('instance.optionsSubtitle')}
              footer={t('instance.optionsCount', { count: options.length })}
              elevation="md"
            >
              <div class="options-import-deck">
                {#if $instances.value.some((i) => i.name !== decodedName)}
                  <Field label={t('instance.importOptionsFrom')}>
                    <div class="import-input-row">
                      <select bind:value={importOptionsFrom} class="glass-select">
                        <option value="">{t('instance.selectInstance')}</option>
                        {#each $instances.value.filter((i) => i.name !== decodedName) as inst}
                          <option value={inst.name}>{inst.name} ({inst.version})</option>
                        {/each}
                      </select>
                      <Btn
                        variant="secondary"
                        size="sm"
                        disabled={!importOptionsFrom || importingOptions}
                        loading={importingOptions}
                        onclick={handleImportOptions}
                      >
                        {t('instance.importOptionsBtn')}
                      </Btn>
                    </div>
                  </Field>
                {/if}

                {#if optionsNote}
                  <p class="options-feedback" in:fade>{optionsNote}</p>
                {/if}

                {#if options.length > 0}
                  <details class="options-preview">
                    <summary>{t('instance.optionsCount', { count: options.length })}</summary>
                    <div class="options-scroller">
                      {#each options.slice(0, 30) as [k, v]}
                        <div class="option-row">
                          <span class="option-key">{k}</span>
                          <span class="option-val">{v}</span>
                        </div>
                      {/each}
                    </div>
                  </details>
                {/if}
              </div>
            </GlassCard>

            <!-- Espectral Client Suite Toggles Card -->
            {#if clientInfo?.supported && clientInfo?.registry?.length}
              <GlassCard title={t('instance.clientTitle')} subtitle={t('instance.clientSubtitle')} elevation="md">
                <div class="client-features-list">
                  {#each clientInfo.registry as feat}
                    {@const isEnabled = clientInfo.config?.features?.[feat.id]?.enabled ?? feat.defaultEnabled}
                    <div class="client-feature-row">
                      <div class="feature-meta">
                        <span class="feature-name">{feat.name}</span>
                        <span class="feature-desc">{feat.description}</span>
                        {#if feat.keybind}
                          <span class="feature-keybind">{t('instance.keybind', { key: feat.keybind })}</span>
                        {/if}
                      </div>
                      <button
                        type="button"
                        class="toggle-switch"
                        class:toggle-switch--on={isEnabled}
                        disabled={togglingFeature === feat.id}
                        onclick={() => handleToggleFeature(feat.id, !isEnabled)}
                        aria-label={feat.name}
                      >
                        <span class="toggle-knob"></span>
                      </button>
                    </div>
                  {/each}
                </div>
              </GlassCard>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Tab 2: SERVERS -->
      {#if activeTab === 'servers'}
        <div
          id="panel-servers"
          role="tabpanel"
          aria-labelledby="tab-servers"
          class="workbench-panel"
          in:fade={{ duration: 150 }}
        >
          <GlassCard title={t('instance.serversTitle')} subtitle={t('instance.serversSubtitle')} elevation="md">
            {#if editingServers}
              <div class="servers-editor" in:fade>
                <div class="editor-rows">
                  {#each serverDraft as _, idx}
                    <div class="server-draft-row">
                      <input
                        type="text"
                        bind:value={serverDraft[idx].name}
                        placeholder={t('instance.serverName')}
                        class="glass-input input-name"
                      />
                      <input
                        type="text"
                        bind:value={serverDraft[idx].ip}
                        placeholder={t('instance.serverIp')}
                        class="glass-input input-ip"
                      />
                      <button
                        type="button"
                        class="remove-row-btn"
                        onclick={() => removeServerRow(idx)}
                        title={t('instance.removeServer')}
                        aria-label={t('instance.removeServer')}
                      >
                        ✕
                      </button>
                    </div>
                  {/each}
                </div>

                {#if serverErr}
                  <p class="server-err-msg">{serverErr}</p>
                {/if}

                <div class="editor-actions-row">
                  <Btn variant="ghost" size="sm" onclick={addServerRow}>
                    {t('instance.addServer')}
                  </Btn>
                  <div class="editor-commit-group">
                    <Btn variant="ghost" size="sm" onclick={() => (editingServers = false)}>
                      {t('instance.cancelEdit')}
                    </Btn>
                    <Btn
                      variant="primary"
                      size="sm"
                      loading={savingServers}
                      disabled={savingServers}
                      onclick={handleSaveServers}
                    >
                      {t('instance.saveServers')}
                    </Btn>
                  </div>
                </div>
              </div>
            {:else}
              <div class="servers-display-deck">
                {#if servers.length === 0}
                  <div class="empty-servers-state">
                    <p class="empty-text">{t('instance.noServers')}</p>
                  </div>
                {:else}
                  <div class="servers-grid">
                    {#each servers as s}
                      {@const netMatch = $networkServers.value.find((net) => net.ip === s.ip || net.hostname === s.ip || net.host === s.ip)}
                      <div class="server-card-item">
                        <div class="server-item-header">
                          <span class="server-item-name">{s.name || s.ip}</span>
                          {#if netMatch}
                            <Badge variant={netMatch.online ? 'ok' : 'err'} dot={true} size="sm">
                              {netMatch.online
                                ? t('instance.serverPlayers', { online: netMatch.players_online, max: netMatch.players_max })
                                : t('instance.serverOffline')}
                            </Badge>
                          {:else}
                            <code class="server-ip-pill">{s.ip}</code>
                          {/if}
                        </div>
                        {#if netMatch?.online && netMatch.motd_clean?.length}
                          <p class="server-motd">{netMatch.motd_clean.join(' ')}</p>
                        {/if}
                      </div>
                    {/each}
                  </div>
                {/if}

                <div class="card-action-footer">
                  <Btn variant="secondary" size="sm" onclick={startEditServers}>
                    {servers.length > 0 ? t('instance.editServers') : t('instance.addServer')}
                  </Btn>
                </div>
              </div>
            {/if}
          </GlassCard>
        </div>
      {/if}

      <!-- Tab 3: MODS -->
      {#if activeTab === 'mods'}
        <div
          id="panel-mods"
          role="tabpanel"
          aria-labelledby="tab-mods"
          class="workbench-panel"
          in:fade={{ duration: 150 }}
        >
          <div class="panel-grid">
            <GlassCard title={t('instance.modsTitle')} subtitle={t('instance.modsSubtitle')} elevation="md">
              <div class="mods-overview-deck">
                <div class="mods-stat-badge-row">
                  <Badge variant="accent" size="md">
                    {t('instance.modsSummary', { enabled: summary.enabled_mod_count, total: summary.mod_count })}
                  </Badge>
                </div>

                <div class="mods-action-pills">
                  <Btn variant="primary" onclick={() => (window.location.hash = '#/mods')}>
                    {t('instance.goToMods')}
                  </Btn>
                  <Btn variant="secondary" onclick={handleOpenFolder}>
                    {t('instance.openModsFolder')}
                  </Btn>
                </div>
              </div>
            </GlassCard>

            {#if presetInfo?.supported}
              <GlassCard title={t('instance.perfPresetTitle')} subtitle={t('instance.perfPresetDesc')} elevation="md">
                <div class="perf-preset-deck">
                  {#if installProgress}
                    <div class="preset-progress" in:fade>
                      <ProgressBar
                        value={installProgress.done}
                        max={Math.max(installProgress.total, 1)}
                        label={installProgress.filename ? `Instalando ${installProgress.filename}…` : t('instance.installingPreset')}
                        tone="cyan"
                      />
                    </div>
                  {/if}

                  <div class="card-action-footer">
                    <Btn
                      variant="secondary"
                      size="sm"
                      loading={installingPreset}
                      disabled={installingPreset || isInstanceRunning}
                      onclick={handleInstallPerfPreset}
                    >
                      {t('instance.installPerfPreset')}
                    </Btn>
                  </div>
                </div>
              </GlassCard>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Tab 4: DANGER ZONE -->
      {#if activeTab === 'danger'}
        <div
          id="panel-danger"
          role="tabpanel"
          aria-labelledby="tab-danger"
          class="workbench-panel"
          in:fade={{ duration: 150 }}
        >
          <GlassCard
            title={t('instance.dangerTitle')}
            subtitle={t('instance.dangerSubtitle')}
            elevation="lg"
            class="danger-card"
          >
            <div class="danger-zone-content">
              <div class="danger-warning-box">
                <div class="warning-icon">⚠</div>
                <div class="warning-text">
                  <h3 class="warning-title">{t('instance.deleteInstanceTitle')}</h3>
                  <p class="warning-desc">{t('instance.deleteWarning')}</p>
                </div>
              </div>

              <div class="danger-action-row">
                <Btn
                  variant="danger"
                  size="md"
                  loading={deleting}
                  disabled={deleting}
                  onclick={handleDeleteInstance}
                >
                  {t('instance.deleteBtn')}
                </Btn>
              </div>
            </div>
          </GlassCard>
        </div>
      {/if}
    </div>

    <!-- Live Log Console Flight Deck -->
    <section class="hub-log-section" aria-label="Console Logs">
      <GlassCard elevation="md" class="console-card">
        {#snippet header()}
          <div class="console-header">
            <div class="console-title-group">
              <span class="live-dot" class:live-dot--active={isLogRunning}></span>
              <span class="console-heading">{t('instance.logTitle')}</span>
              <Badge variant={isLogRunning ? 'ok' : 'muted'} size="sm">
                {isLogRunning ? t('instance.logLive') : t('instance.logIdle')}
              </Badge>
              {#if logLines.length > 0}
                <span class="log-line-count">{logLines.length} líneas</span>
              {/if}
            </div>

            <div class="console-actions">
              <Btn variant="ghost" size="sm" onclick={handleToggleHud}>
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                  <polyline points="15 3 21 3 21 9" />
                  <polyline points="9 21 3 21 3 15" />
                  <line x1="21" y1="3" x2="14" y2="10" />
                  <line x1="3" y1="21" x2="10" y2="14" />
                </svg>
                <span>{t('instance.openInHud')}</span>
              </Btn>
              <Btn variant="ghost" size="sm" onclick={handleClearLog}>
                {t('instance.clearLog')}
              </Btn>
            </div>
          </div>
        {/snippet}

        <div
          class="console-output"
          bind:this={logContainer}
          onscroll={onLogScroll}
          role="region"
          aria-label="Console Output"
        >
          {#each logLines as line}
            <div
              class="console-row"
              class:console-row--error={/(ERROR|SEVERE|Exception|Caused by|Failed)/.test(line)}
              class:console-row--warn={/(WARN|WARNING)/.test(line)}
            >
              {line}
            </div>
          {:else}
            <div class="console-empty">{t('instance.emptyLog')}</div>
          {/each}
        </div>
      </GlassCard>
    </section>
  {/if}
</div>

<style>
  /* ==========================================================================
     InstanceHub Layout & Glass Styling
     ========================================================================== */

  .instance-hub {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
    padding: 1.75rem 2rem 3.5rem;
    max-width: 1320px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* Header */
  .hub-header {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .hub-header__nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .back-link {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 500;
    text-decoration: none;
    transition: color 150ms ease, transform 150ms ease;
    border-radius: var(--radius-sm, 0.375rem);
    padding: 0.25rem 0.5rem;
    margin-left: -0.5rem;
  }

  .back-link:hover {
    color: var(--text, #e8ecf4);
    transform: translateX(-2px);
  }

  .back-link:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  .folder-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.08));
    color: var(--muted-strong, #b3c5e3);
    padding: 0.35rem 0.75rem;
    border-radius: var(--radius-md, 0.5rem);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 500;
    cursor: pointer;
    transition: all 150ms ease;
  }

  .folder-btn:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    color: var(--text, #e8ecf4);
    border-color: var(--border-specular, rgba(255, 255, 255, 0.16));
  }

  .folder-btn:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  .hub-header__title-row {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 1.5rem;
  }

  .hub-header__meta {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .instance-title {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-hero, clamp(2rem, 1.5rem + 1.8vw, 3rem));
    font-weight: 700;
    line-height: var(--leading-tight, 1.2);
    letter-spacing: -0.02em;
    color: var(--text, #e8ecf4);
    margin: 0;
    text-shadow: 0 4px 24px rgba(0, 0, 0, 0.4);
  }

  .meta-badges {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
  }

  /* Action Deck */
  .hub-action-deck {
    width: 100%;
  }

  /* State Card */
  .hub-state-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    padding: 4rem 2rem;
    text-align: center;
  }

  .loading-spinner {
    width: 2.25rem;
    height: 2.25rem;
    border: 3px solid rgba(255, 255, 255, 0.1);
    border-top-color: var(--accent, #10b981);
    border-radius: 50%;
    animation: spin 750ms linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .state-text {
    color: var(--muted, #8e9eb8);
    font-size: var(--text-md, 0.9375rem);
    margin: 0;
  }

  .state-icon.error-icon {
    font-size: 2rem;
    color: var(--accent-red, #ef4444);
  }

  .state-title {
    font-family: var(--font-display, sans-serif);
    font-size: var(--text-xl, 1.375rem);
    color: var(--text, #e8ecf4);
    margin: 0;
  }

  .state-desc {
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
    max-width: 480px;
    margin: 0;
  }

  /* Workbench Tabs */
  .hub-workbench {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .workbench-tabs {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem;
    background: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 0.75rem);
    width: fit-content;
  }

  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    padding: 0.5rem 0.9rem;
    border-radius: var(--radius-md, 0.5rem);
    cursor: pointer;
    transition: all 160ms ease;
  }

  .tab-btn:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.04);
  }

  .tab-btn:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  .tab-btn--active {
    background: var(--surface-up-solid, #161e36);
    color: var(--text, #e8ecf4);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
    border: 1px solid var(--border-specular, rgba(255, 255, 255, 0.16));
  }

  .tab-btn--danger.tab-btn--active {
    border-color: rgba(239, 68, 68, 0.4);
    color: #fca5a5;
  }

  .tab-count {
    background: rgba(255, 255, 255, 0.08);
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
    font-size: 0.7rem;
    font-weight: 700;
  }

  /* Workbench Panels */
  .workbench-panel {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .panel-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(28rem, 1fr));
    gap: 1.25rem;
    align-items: start;
  }

  @media (max-width: 768px) {
    .panel-grid {
      grid-template-columns: 1fr;
    }
  }

  /* Memory Setting */
  .memory-setting {
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
  }

  .memory-slider-row {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .slider-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .slider-val-tag {
    font-family: var(--font-display, sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--accent, #10b981);
  }

  .slider-minmax {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .range-slider {
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.1);
    outline: none;
    accent-color: var(--accent, #10b981);
    cursor: pointer;
  }

  .memory-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .preset-pill {
    background: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.08));
    color: var(--muted-strong, #b3c5e3);
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    cursor: pointer;
    transition: all 140ms ease;
  }

  .preset-pill:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    color: var(--text, #e8ecf4);
  }

  .preset-pill--active {
    background: rgba(16, 185, 129, 0.18);
    border-color: var(--accent, #10b981);
    color: var(--accent, #10b981);
  }

  .card-action-footer {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.5rem;
  }

  /* JVM Specs */
  .jvm-details {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .jvm-spec-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.75rem;
    background: rgba(0, 0, 0, 0.2);
    padding: 0.75rem;
    border-radius: var(--radius-md, 0.5rem);
  }

  .spec-cell {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .spec-label {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
    font-weight: 600;
  }

  .spec-value {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .jvm-path-display {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .code-box {
    font-family: var(--font-mono, monospace);
    font-size: 0.725rem;
    background: rgba(0, 0, 0, 0.3);
    padding: 0.5rem 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
    color: var(--muted-strong, #b3c5e3);
    word-break: break-all;
    user-select: all;
  }

  .glass-input, .glass-select {
    width: 100%;
    background: rgba(8, 12, 24, 0.7);
    border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 0.5rem);
    color: var(--text, #e8ecf4);
    font-family: var(--font-body, sans-serif);
    font-size: var(--text-sm, 0.875rem);
    padding: 0.55rem 0.75rem;
    outline: none;
    transition: border-color 150ms ease, box-shadow 150ms ease;
    box-sizing: border-box;
  }

  .glass-input:focus, .glass-select:focus {
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 0 2px rgba(16, 185, 129, 0.2);
  }

  /* AOT Card */
  .aot-settings {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .aot-stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(8rem, 1fr));
    gap: 0.75rem;
    background: rgba(0, 0, 0, 0.2);
    padding: 0.75rem;
    border-radius: var(--radius-md, 0.5rem);
  }

  .aot-stat-item {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .stat-label {
    font-size: 0.7rem;
    color: var(--muted, #8e9eb8);
    font-weight: 600;
  }

  .stat-num {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .stat-desc {
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
  }

  .toggle-checkbox-row {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    cursor: pointer;
    user-select: none;
  }

  .glass-checkbox {
    width: 1.15rem;
    height: 1.15rem;
    margin-top: 0.15rem;
    accent-color: var(--accent, #10b981);
    cursor: pointer;
  }

  .toggle-texts {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .toggle-title {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .toggle-hint {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  /* Options Import Deck */
  .options-import-deck {
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }

  .import-input-row {
    display: flex;
    gap: 0.5rem;
  }

  .options-feedback {
    font-size: var(--text-xs, 0.75rem);
    color: var(--accent-green, #22c55e);
    margin: 0;
  }

  .options-preview {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    cursor: pointer;
  }

  .options-scroller {
    max-height: 180px;
    overflow-y: auto;
    background: rgba(0, 0, 0, 0.25);
    border-radius: var(--radius-sm, 0.375rem);
    padding: 0.5rem;
    margin-top: 0.5rem;
  }

  .option-row {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.2rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .option-key {
    font-family: var(--font-mono, monospace);
    color: var(--muted-strong, #b3c5e3);
  }

  .option-val {
    color: var(--text, #e8ecf4);
    font-weight: 500;
  }

  /* Client Features List */
  .client-features-list {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .client-feature-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.6rem 0.75rem;
    background: rgba(0, 0, 0, 0.2);
    border-radius: var(--radius-md, 0.5rem);
  }

  .feature-meta {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .feature-name {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .feature-desc {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .feature-keybind {
    font-size: 0.7rem;
    font-weight: 700;
    color: var(--accent-alt, #f59e0b);
  }

  .toggle-switch {
    position: relative;
    width: 2.75rem;
    height: 1.5rem;
    background: rgba(255, 255, 255, 0.12);
    border: none;
    border-radius: 999px;
    cursor: pointer;
    padding: 2px;
    transition: background-color 200ms ease;
    flex-shrink: 0;
  }

  .toggle-switch:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  .toggle-switch--on {
    background: var(--accent, #10b981);
  }

  .toggle-knob {
    display: block;
    width: 1.25rem;
    height: 1.25rem;
    background: #ffffff;
    border-radius: 50%;
    transition: transform 200ms ease;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
  }

  .toggle-switch--on .toggle-knob {
    transform: translateX(1.25rem);
  }

  /* Servers Tab */
  .servers-editor, .servers-display-deck {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .editor-rows {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .server-draft-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .input-name {
    flex: 1;
  }

  .input-ip {
    flex: 1.5;
  }

  .remove-row-btn {
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: #fca5a5;
    width: 2.25rem;
    height: 2.25rem;
    border-radius: var(--radius-md, 0.5rem);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 140ms ease;
  }

  .remove-row-btn:hover {
    background: rgba(239, 68, 68, 0.3);
  }

  .server-err-msg {
    color: var(--accent-red, #ef4444);
    font-size: var(--text-xs, 0.75rem);
    margin: 0;
  }

  .editor-actions-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .editor-commit-group {
    display: flex;
    gap: 0.5rem;
  }

  .servers-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: 0.75rem;
  }

  .server-card-item {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid var(--border-subtle, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 0.5rem);
    padding: 0.75rem;
  }

  .server-item-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .server-item-name {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .server-ip-pill {
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    background: rgba(255, 255, 255, 0.06);
    padding: 0.15rem 0.4rem;
    border-radius: var(--radius-sm, 0.25rem);
    color: var(--muted-strong, #b3c5e3);
  }

  .server-motd {
    font-size: 0.75rem;
    color: var(--muted, #8e9eb8);
    margin: 0;
    line-height: 1.35;
  }

  .empty-servers-state {
    padding: 2rem 1rem;
    text-align: center;
  }

  .empty-text {
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
    margin: 0;
  }

  /* Mods Tab */
  .mods-overview-deck, .perf-preset-deck {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .mods-action-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  /* Danger Tab */
  :global(.danger-card) {
    border-color: rgba(239, 68, 68, 0.35) !important;
    background: radial-gradient(circle at top right, rgba(239, 68, 68, 0.08), transparent 70%), var(--card-bg, rgba(13, 18, 34, 0.75)) !important;
  }

  .danger-zone-content {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .danger-warning-box {
    display: flex;
    align-items: flex-start;
    gap: 0.85rem;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.25);
    border-radius: var(--radius-md, 0.5rem);
    padding: 0.85rem 1rem;
  }

  .warning-icon {
    font-size: 1.35rem;
    color: var(--accent-red, #ef4444);
    line-height: 1;
  }

  .warning-text {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .warning-title {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    color: #fca5a5;
    margin: 0;
  }

  .warning-desc {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted-strong, #b3c5e3);
    line-height: 1.4;
    margin: 0;
  }

  .danger-action-row {
    display: flex;
    justify-content: flex-end;
  }

  /* Console Section */
  .hub-log-section {
    width: 100%;
  }

  .console-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    gap: 1rem;
  }

  .console-title-group {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .live-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--muted, #8e9eb8);
  }

  .live-dot--active {
    background: var(--accent-green, #22c55e);
    box-shadow: 0 0 8px var(--accent-green, #22c55e);
    animation: pulse 1.5s infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.5; transform: scale(0.85); }
  }

  .console-heading {
    font-family: var(--font-display, sans-serif);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .log-line-count {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .console-actions {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .console-output {
    background: var(--log-bg, #080c18);
    border-radius: var(--radius-md, 0.5rem);
    padding: 0.75rem 1rem;
    height: 220px;
    overflow-y: auto;
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    line-height: 1.5;
    color: var(--muted-strong, #b3c5e3);
    user-select: text;
    outline: none;
    border: 1px solid rgba(255, 255, 255, 0.04);
  }

  .console-output:focus-visible {
    border-color: var(--accent, #10b981);
  }

  .console-row {
    white-space: pre-wrap;
    word-break: break-all;
  }

  .console-row--error {
    color: var(--accent-red, #ef4444);
  }

  .console-row--warn {
    color: var(--accent-alt, #f59e0b);
  }

  .console-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--muted, #8e9eb8);
    font-style: italic;
  }
</style>
