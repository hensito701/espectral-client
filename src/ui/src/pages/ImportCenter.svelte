<!--
  ============================================================================
  ImportCenter.svelte — Horizon Glass Data Migration & Modpack Center (#/library/import)
  ============================================================================
  Two-lane hero layout:
    LANE A: .mrpack Modpack auto-installer (drag & drop, file picker, memory allocation,
            live SSE download progress & extract tracking, instant launch routing).
    LANE B: Launcher profile migrator (detects Vanilla, FastClient, Lunar Client profiles,
            displays options/servers preview, protected Lunar token-chain shield,
            configurable overwrite policies, detailed migration result breakdown).
    BOTTOM: Session-local import history log with direct navigation links.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import {
    getImportSources,
    importProfile,
    pickMrpackFile,
    importMrpack,
    getInstance,
    subscribeEvents,
  } from '../lib/api';
  import type {
    ImportSource,
    ImportResult,
    OverwritePolicy,
  } from '../lib/types';
  import type { SseEvent } from '../lib/sse';
  import { instances } from '../lib/stores';
  import { pushToast } from '../lib/toast.svelte';
  import { t } from '../lib/i18n.svelte';
  import { flyY, scalePop } from '../lib/motion';

  import GlassCard from '../components/GlassCard.svelte';
  import Field from '../components/Field.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import ProgressBar from '../components/ProgressBar.svelte';
  import GradientText from '../components/GradientText.svelte';

  /* ==========================================================================
     Types & Session History
     ========================================================================== */
  interface RecentImportItem {
    id: string;
    timestamp: Date;
    type: 'mrpack' | 'profile';
    sourceKind?: string;
    sourceLabel?: string;
    instanceName: string;
    summary: string;
    copiedCount?: number;
    skippedCount?: number;
    serversCount?: number;
    optionsCount?: number;
  }

  // Session-local imports history ($state)
  let recentImports = $state<RecentImportItem[]>([]);

  /* ==========================================================================
     LANE A: .mrpack Modpack Installer State
     ========================================================================== */
  let mrpackPath = $state('');
  let mrpackFileName = $state('');
  let mrpackMemoryMb = $state(3072);
  let mrpackBusy = $state(false);
  let browsingMrpack = $state(false);
  let isDraggingMrpack = $state(false);
  let hiddenFileInput: HTMLInputElement | null = $state(null);

  type MrpackPhase = 'idle' | 'post' | 'files' | 'overrides' | 'done' | 'error';
  let mrpackPhase = $state<MrpackPhase>('idle');
  let mrpackDone = $state(0);
  let mrpackTotal = $state(0);
  let mrpackErr = $state('');
  let mrpackResultInstance = $state('');
  let mrpackAlreadyExists = $state(false);

  const mrpackWorking = $derived(
    mrpackBusy || mrpackPhase === 'post' || mrpackPhase === 'files' || mrpackPhase === 'overrides'
  );

  let mrpackWatchdog: ReturnType<typeof setTimeout> | null = null;

  /* ==========================================================================
     LANE B: Launcher Profiles Migrator State
     ========================================================================== */
  let sources = $state<ImportSource[]>([]);
  let sourcesLoading = $state(true);
  let sourcesError = $state('');

  let selectedSourceId = $state('');
  let targetInstance = $state('');
  let overwritePolicy = $state<OverwritePolicy>('never');
  let profileImporting = $state(false);
  let profileErr = $state('');
  let profileResult = $state<ImportResult | null>(null);

  // Auto-subscribe store to avoid nested snippet subscription issues
  const instanceList = $derived($instances.value);
  const selectedSource = $derived(sources.find((s) => s.id === selectedSourceId) ?? null);

  /* ==========================================================================
     LANE A: Actions & Handlers
     ========================================================================== */
  function extractFilename(fullPath: string): string {
    if (!fullPath) return '';
    const clean = fullPath.replace(/\\/g, '/');
    const parts = clean.split('/');
    return parts[parts.length - 1] || fullPath;
  }

  async function handleBrowseMrpack() {
    if (browsingMrpack || mrpackWorking) return;
    browsingMrpack = true;
    try {
      const picked = await pickMrpackFile();
      if (picked?.path) {
        mrpackPath = picked.path;
        mrpackFileName = extractFilename(picked.path);
        mrpackErr = '';
        if (mrpackPhase === 'error' || mrpackPhase === 'done') {
          mrpackPhase = 'idle';
        }
      }
    } catch {
      // Fallback: trigger hidden input if native picker is unavailable
      if (hiddenFileInput) {
        hiddenFileInput.click();
      }
    } finally {
      browsingMrpack = false;
    }
  }

  function handleFileInputChange(e: Event) {
    const input = e.target as HTMLInputElement;
    const files = input.files;
    if (files && files.length > 0) {
      const file = files[0];
      // In Tauri / desktop webview, webkitRelativePath or path property may exist
      const filePath = (file as unknown as { path?: string }).path || file.name;
      mrpackPath = filePath;
      mrpackFileName = file.name;
      mrpackErr = '';
      if (mrpackPhase === 'error' || mrpackPhase === 'done') {
        mrpackPhase = 'idle';
      }
    }
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!mrpackWorking) {
      isDraggingMrpack = true;
    }
  }

  function handleDragEnter(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!mrpackWorking) {
      isDraggingMrpack = true;
    }
  }

  function handleDragLeave(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    isDraggingMrpack = false;
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    isDraggingMrpack = false;
    if (mrpackWorking) return;

    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const file = files[0];
      if (file.name.toLowerCase().endsWith('.mrpack')) {
        const filePath = (file as unknown as { path?: string }).path || file.name;
        mrpackPath = filePath;
        mrpackFileName = file.name;
        mrpackErr = '';
        if (mrpackPhase === 'error' || mrpackPhase === 'done') {
          mrpackPhase = 'idle';
        }
      } else {
        mrpackErr = 'Solo se admiten archivos con extensión .mrpack';
      }
    }
  }

  function handleClearMrpack() {
    if (mrpackWorking) return;
    mrpackPath = '';
    mrpackFileName = '';
    mrpackPhase = 'idle';
    mrpackErr = '';
    mrpackDone = 0;
    mrpackTotal = 0;
  }

  async function checkMrpackStuck() {
    mrpackWatchdog = null;
    if (mrpackPhase !== 'post' || !mrpackResultInstance) return;
    try {
      await getInstance(mrpackResultInstance);
      mrpackPhase = 'done';
      void instances.refresh();
    } catch {
      /* wait for SSE or error */
    }
  }

  async function doInstallMrpack() {
    const filePath = mrpackPath.trim();
    if (!filePath || mrpackWorking) return;

    mrpackBusy = true;
    mrpackErr = '';
    mrpackPhase = 'post';
    mrpackDone = 0;
    mrpackTotal = 0;

    try {
      const res = await importMrpack(filePath, mrpackMemoryMb);
      mrpackResultInstance = res.summary?.name || '';
      mrpackAlreadyExists = (res as { already_exists?: boolean }).already_exists === true;

      if (mrpackWatchdog) clearTimeout(mrpackWatchdog);
      mrpackWatchdog = setTimeout(checkMrpackStuck, 15_000);
    } catch (e) {
      mrpackPhase = 'error';
      mrpackErr = e instanceof Error ? e.message : String(e);
      pushToast({
        kind: 'err',
        text: t('import.mrpack.phaseError', { error: mrpackErr }),
      });
    } finally {
      mrpackBusy = false;
    }
  }

  function resetMrpackFlow() {
    mrpackPath = '';
    mrpackFileName = '';
    mrpackPhase = 'idle';
    mrpackDone = 0;
    mrpackTotal = 0;
    mrpackErr = '';
    mrpackResultInstance = '';
    mrpackAlreadyExists = false;
  }

  /* ==========================================================================
     LANE B: Actions & Handlers
     ========================================================================== */
  async function loadSources() {
    sourcesLoading = true;
    sourcesError = '';
    try {
      sources = await getImportSources();
      if (sources.length > 0 && !selectedSourceId) {
        selectedSourceId = sources[0].id;
      }
    } catch (e) {
      sourcesError = e instanceof Error ? e.message : String(e);
    } finally {
      sourcesLoading = false;
    }
  }

  function getKindAccent(kind: string): { variant: 'vanilla' | 'fabric' | 'purple' | 'accent'; label: string; icon: string } {
    switch (kind) {
      case 'vanilla':
        return { variant: 'vanilla', label: 'Vanilla .minecraft', icon: '⛏️' };
      case 'fastclient':
        return { variant: 'accent', label: 'FastClient', icon: '⚡' };
      case 'lunar':
        return { variant: 'purple', label: 'Lunar Client', icon: '🌙' };
      default:
        return { variant: 'fabric', label: kind, icon: '📦' };
    }
  }

  async function doImportProfile() {
    if (!targetInstance || !selectedSource || profileImporting) return;
    profileImporting = true;
    profileErr = '';
    profileResult = null;

    try {
      const res = await importProfile(targetInstance, selectedSource.id, overwritePolicy);
      profileResult = res;

      // Add to session-local recent imports
      const item: RecentImportItem = {
        id: `profile-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        timestamp: new Date(),
        type: 'profile',
        sourceKind: selectedSource.kind,
        sourceLabel: selectedSource.label || selectedSource.id,
        instanceName: targetInstance,
        summary: t('import.recent.copiedSummary', {
          copied: res.copied.length,
          servers: res.servers_parsed.length,
        }),
        copiedCount: res.copied.length,
        skippedCount: res.skipped.length,
        serversCount: res.servers_parsed.length,
        optionsCount: res.options_keys,
      };
      recentImports = [item, ...recentImports];

      pushToast({
        kind: 'ok',
        text: `Ajustes de ${selectedSource.label} importados en ${targetInstance}`,
        href: `#/instances/${encodeURIComponent(targetInstance)}`,
      });
    } catch (e) {
      profileErr = e instanceof Error ? e.message : String(e);
      pushToast({
        kind: 'err',
        text: profileErr,
      });
    } finally {
      profileImporting = false;
    }
  }

  function resetProfileResult() {
    profileResult = null;
    profileErr = '';
  }

  function openInstanceWizard() {
    window.dispatchEvent(new CustomEvent('horizon:open-wizard'));
  }

  /* ==========================================================================
     SSE Lifecycle for .mrpack progress
     ========================================================================== */
  $effect(() => {
    return subscribeEvents((ev: SseEvent) => {
      if (ev.type !== 'import-progress' && ev.type !== 'import-done') return;

      const d = ev.data as
        | { instance?: string; phase?: string; index?: number; total?: number }
        | { instance?: string; ok?: boolean; error?: string; already_exists?: boolean }
        | null;

      if (!d || typeof d.instance !== 'string') return;
      if (mrpackResultInstance && d.instance !== mrpackResultInstance) return;

      if (mrpackWatchdog) {
        clearTimeout(mrpackWatchdog);
        mrpackWatchdog = null;
      }

      if (ev.type === 'import-progress') {
        const p = d as { phase?: string; index?: number; total?: number };
        if (p.phase === 'files' || p.phase === 'overrides') {
          mrpackPhase = p.phase;
          if (typeof p.index === 'number') mrpackDone = p.index + 1;
          if (typeof p.total === 'number') mrpackTotal = p.total;
        }
      } else if (ev.type === 'import-done') {
        const done = d as { ok?: boolean; error?: string; already_exists?: boolean };
        if (done.ok) {
          mrpackPhase = 'done';
          mrpackAlreadyExists = done.already_exists === true;
          const instName = d.instance || mrpackResultInstance;
          mrpackResultInstance = instName;
          void instances.refresh();

          // Add to session-local recent imports
          const item: RecentImportItem = {
            id: `mrpack-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
            timestamp: new Date(),
            type: 'mrpack',
            instanceName: instName,
            summary: mrpackAlreadyExists
              ? t('import.mrpack.alreadyExists', { name: instName })
              : t('import.mrpack.phaseDone'),
          };
          recentImports = [item, ...recentImports];
        } else {
          mrpackPhase = 'error';
          mrpackErr = done.error ?? 'Error desconocido durante la instalación';
        }
      }
    });
  });

  /* ==========================================================================
     Mount & Setup
     ========================================================================== */
  onMount(() => {
    void loadSources();
    void instances.refresh();

    // Default target instance to first available instance if not selected
    if (instanceList.length > 0 && !targetInstance) {
      targetInstance = instanceList[0].name;
    }
  });

  // Keep target instance in sync if instance list loads later
  $effect(() => {
    if (instanceList.length > 0 && !targetInstance) {
      targetInstance = instanceList[0].name;
    }
  });

  function formatTime(date: Date): string {
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }
</script>

<svelte:head>
  <title>{t('import.tag')}</title>
</svelte:head>

<div class="import-page">
  <!-- Page Header -->
  <header class="import-header" in:flyY={{ y: -8, duration: 200 }}>
    <div class="import-header__badge-row">
      <Badge variant="accent" pixel size="sm">MIGRATION & PACKS</Badge>
      <span class="import-header__route">#/library/import</span>
    </div>
    <h1 class="import-header__title">
      <GradientText gradient="emerald-gold">{t('import.pageTitle')}</GradientText>
    </h1>
    <p class="import-header__subtitle">
      {t('import.pageSubtitle')}
    </p>
  </header>

  <!-- Two-Lane Hero Section -->
  <div class="import-lanes">
    <!-- ========================================================================
         LANE A: .mrpack Modpack Installer
         ======================================================================== -->
    <section class="lane lane--mrpack" in:flyY={{ y: 12, duration: 220, delay: 40 }}>
      <GlassCard elevation="md" className="lane-card">
        <!-- Lane Header -->
        <div class="lane-header">
          <div class="lane-header__title-group">
            <div class="lane-header__tag-row">
              <Badge variant="accent" size="sm">{t('import.laneA.badge')}</Badge>
              <span class="lane-header__format-tag">.mrpack</span>
            </div>
            <h2 class="lane-header__title">{t('import.laneA.title')}</h2>
          </div>
          <p class="lane-header__desc">{t('import.laneA.description')}</p>
        </div>

        {#if mrpackPhase === 'done'}
          <!-- Success State -->
          <div class="mrpack-success" in:scalePop={{ duration: 200 }}>
            <div class="mrpack-success__icon-box">
              <span class="mrpack-success__icon">✨</span>
            </div>
            <div class="mrpack-success__content">
              <Badge variant="ok" size="md">
                {mrpackAlreadyExists ? 'INSTANCIA VINCULADA' : 'INSTALACIÓN COMPLETADA'}
              </Badge>
              <h3 class="mrpack-success__name">{mrpackResultInstance}</h3>
              <p class="mrpack-success__text">
                {mrpackAlreadyExists
                  ? t('import.mrpack.alreadyExists', { name: mrpackResultInstance })
                  : t('import.mrpack.successDesc', { name: mrpackResultInstance })}
              </p>
            </div>

            <div class="mrpack-success__actions">
              <a
                href="#/instances/{encodeURIComponent(mrpackResultInstance)}"
                class="btn-link"
              >
                <Btn variant="primary" size="md" block>
                  {t('import.mrpack.viewInstance')}
                </Btn>
              </a>
              <Btn variant="ghost" size="md" onclick={resetMrpackFlow} block>
                {t('import.mrpack.importAnother')}
              </Btn>
            </div>
          </div>
        {:else}
          <!-- Standard Form / Dropzone Flow -->
          <div class="lane-body">
            <!-- Hidden File Input Fallback -->
            <input
              type="file"
              accept=".mrpack"
              class="hidden-file-input"
              bind:this={hiddenFileInput}
              onchange={handleFileInputChange}
              tabindex="-1"
              aria-hidden="true"
            />

            <!-- Big Drop-Zone Card -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="dropzone {isDraggingMrpack ? 'dropzone--dragging' : ''} {mrpackPath ? 'dropzone--selected' : ''}"
              ondragover={handleDragOver}
              ondragenter={handleDragEnter}
              ondragleave={handleDragLeave}
              ondrop={handleDrop}
              onclick={handleBrowseMrpack}
              onkeydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  void handleBrowseMrpack();
                }
              }}
              tabindex="0"
              role="button"
              aria-label={t('import.mrpack.browseButton')}
            >
              <div class="dropzone__glow"></div>
              <div class="dropzone__content">
                <div class="dropzone__icon-circle">
                  <span class="dropzone__icon">{isDraggingMrpack ? '📥' : '📦'}</span>
                </div>
                <div class="dropzone__labels">
                  <span class="dropzone__title">
                    {isDraggingMrpack
                      ? t('import.mrpack.dropTitle')
                      : mrpackFileName || t('import.mrpack.dropTitle')}
                  </span>
                  <span class="dropzone__subtitle">
                    {mrpackFileName ? mrpackPath : t('import.mrpack.dropSubtitle')}
                  </span>
                </div>
                <Btn
                  variant={mrpackFileName ? 'ghost' : 'secondary'}
                  size="sm"
                  disabled={mrpackWorking}
                  onclick={(e) => {
                    e.stopPropagation();
                    void handleBrowseMrpack();
                  }}
                >
                  {mrpackFileName ? 'Cambiar archivo' : t('import.mrpack.browseButton')}
                </Btn>
              </div>
            </div>

            <!-- Selected File Chip / Status -->
            {#if mrpackFileName}
              <div class="file-chip" in:flyY={{ y: 6, duration: 160 }}>
                <div class="file-chip__info">
                  <span class="file-chip__icon">📄</span>
                  <span class="file-chip__name" title={mrpackPath}>{mrpackFileName}</span>
                  <Badge variant="neutral" size="sm">.mrpack</Badge>
                </div>
                {#if !mrpackWorking}
                  <button
                    type="button"
                    class="file-chip__clear"
                    onclick={handleClearMrpack}
                    title={t('import.mrpack.clearFile')}
                    aria-label={t('import.mrpack.clearFile')}
                  >
                    ✕
                  </button>
                {/if}
              </div>
            {/if}

            <!-- RAM Memory Allocation -->
            <div class="memory-config">
              <Field
                label={t('import.mrpack.memoryLabel')}
                hint={t('import.mrpack.memoryHint')}
                id="mrpack-ram"
              >
                <div class="memory-input-group">
                  <input
                    type="number"
                    id="mrpack-ram"
                    min="1024"
                    max="16384"
                    step="512"
                    disabled={mrpackWorking}
                    bind:value={mrpackMemoryMb}
                    class="field-input memory-input"
                  />
                  <div class="memory-presets">
                    {#each [2048, 3072, 4096, 6144, 8192] as ram}
                      <button
                        type="button"
                        class="preset-btn {mrpackMemoryMb === ram ? 'preset-btn--active' : ''}"
                        disabled={mrpackWorking}
                        onclick={() => (mrpackMemoryMb = ram)}
                      >
                        {ram / 1024} GB
                      </button>
                    {/each}
                  </div>
                </div>
              </Field>
            </div>

            <!-- Live Progress / Status View -->
            {#if mrpackWorking || mrpackPhase === 'error'}
              <div class="mrpack-progress-block" in:flyY={{ y: 8, duration: 180 }}>
                <div class="mrpack-progress-block__header">
                  <span class="mrpack-progress-block__phase">
                    {#if mrpackPhase === 'post'}
                      {t('import.mrpack.phasePost')}
                    {:else if mrpackPhase === 'files'}
                      {t('import.mrpack.phaseFiles', { done: mrpackDone, total: mrpackTotal || '…' })}
                    {:else if mrpackPhase === 'overrides'}
                      {t('import.mrpack.phaseOverrides', { done: mrpackDone, total: mrpackTotal || '…' })}
                    {:else if mrpackPhase === 'error'}
                      <span class="text-error">{t('import.mrpack.phaseError', { error: mrpackErr })}</span>
                    {:else}
                      {t('import.mrpack.installing')}
                    {/if}
                  </span>
                  {#if mrpackPhase === 'files' || mrpackPhase === 'overrides'}
                    <span class="mrpack-progress-block__counts">
                      {mrpackDone} / {mrpackTotal}
                    </span>
                  {/if}
                </div>

                <ProgressBar
                  value={mrpackTotal > 0 ? mrpackDone : undefined}
                  max={mrpackTotal > 0 ? mrpackTotal : 100}
                  indeterminate={mrpackPhase === 'post'}
                  tone={mrpackPhase === 'error' ? 'danger' : 'accent'}
                  height="md"
                />
              </div>
            {/if}

            <!-- Install Trigger Button -->
            <div class="lane-actions">
              <Btn
                variant="primary"
                size="lg"
                block
                disabled={!mrpackPath || mrpackWorking}
                loading={mrpackWorking}
                onclick={doInstallMrpack}
              >
                {mrpackWorking ? t('import.mrpack.installing') : t('import.mrpack.installButton')}
              </Btn>
            </div>
          </div>
        {/if}
      </GlassCard>
    </section>

    <!-- ========================================================================
         LANE B: Launcher Profile Migrator
         ======================================================================== -->
    <section class="lane lane--profiles" in:flyY={{ y: 12, duration: 220, delay: 80 }}>
      <GlassCard elevation="md" className="lane-card">
        <!-- Lane Header -->
        <div class="lane-header">
          <div class="lane-header__title-group">
            <div class="lane-header__tag-row">
              <Badge variant="purple" size="sm">{t('import.laneB.badge')}</Badge>
              <span class="lane-header__format-tag">Vanilla • FastClient • Lunar</span>
            </div>
            <h2 class="lane-header__title">{t('import.laneB.title')}</h2>
          </div>
          <p class="lane-header__desc">{t('import.laneB.description')}</p>
        </div>

        {#if profileResult}
          <!-- Profile Result State -->
          <div class="profile-result" in:scalePop={{ duration: 200 }}>
            <div class="profile-result__header">
              <Badge variant="ok" size="md">MIGRACIÓN COMPLETADA</Badge>
              <h3 class="profile-result__title">{t('import.result.title')}</h3>
              <p class="profile-result__target">
                Destino: <strong>{targetInstance}</strong>
              </p>
            </div>

            <!-- Stats Grid -->
            <div class="result-stats">
              <div class="result-stat-chip">
                <span class="result-stat-chip__label">{t('import.result.copied', { count: profileResult.copied.length })}</span>
                <span class="result-stat-chip__value text-ok">{profileResult.copied.length}</span>
              </div>
              <div class="result-stat-chip">
                <span class="result-stat-chip__label">{t('import.result.skipped', { count: profileResult.skipped.length })}</span>
                <span class="result-stat-chip__value text-muted">{profileResult.skipped.length}</span>
              </div>
              <div class="result-stat-chip">
                <span class="result-stat-chip__label">{t('import.result.servers', { count: profileResult.servers_parsed.length })}</span>
                <span class="result-stat-chip__value text-cyan">{profileResult.servers_parsed.length}</span>
              </div>
              <div class="result-stat-chip">
                <span class="result-stat-chip__label">Claves de opciones</span>
                <span class="result-stat-chip__value text-gold">{profileResult.options_keys}</span>
              </div>
            </div>

            <!-- Copied / Skipped Lists Accordion -->
            {#if profileResult.copied.length > 0}
              <div class="result-box">
                <h4 class="result-box__title">Archivos copiados con éxito:</h4>
                <div class="copied-tags">
                  {#each profileResult.copied as file}
                    <Badge variant="ok" size="sm">{file}</Badge>
                  {/each}
                </div>
              </div>
            {/if}

            {#if profileResult.skipped.length > 0}
              <div class="result-box result-box--muted">
                <h4 class="result-box__title">Archivos omitidos:</h4>
                <ul class="skipped-list">
                  {#each profileResult.skipped as sk}
                    <li class="skipped-list__item">
                      <code>{sk.file}</code>
                      <span class="skipped-list__reason">({sk.reason})</span>
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}

            <!-- Parsed Servers List -->
            {#if profileResult.servers_parsed.length > 0}
              <div class="result-box">
                <h4 class="result-box__title">Servidores importados:</h4>
                <div class="servers-scroll-list">
                  {#each profileResult.servers_parsed as srv}
                    <div class="server-item">
                      <div class="server-item__meta">
                        <span class="server-item__name">{srv.name || srv.ip}</span>
                        <code class="server-item__ip">{srv.ip}</code>
                      </div>
                      {#if srv.has_icon}
                        <Badge variant="accent" size="sm">{t('import.result.hasIcon')}</Badge>
                      {/if}
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            <div class="profile-result__actions">
              <a
                href="#/instances/{encodeURIComponent(targetInstance)}"
                class="btn-link"
              >
                <Btn variant="primary" size="md" block>
                  Ver instancia {targetInstance}
                </Btn>
              </a>
              <Btn variant="secondary" size="md" onclick={resetProfileResult} block>
                {t('import.result.newImport')}
              </Btn>
            </div>
          </div>
        {:else}
          <!-- Standard Profile Selection & Settings Flow -->
          <div class="lane-body">
            <!-- Target Instance Selection -->
            <div class="target-selection">
              <Field
                label={t('import.profile.targetInstance')}
                hint={t('import.profile.targetHint')}
                id="target-instance-select"
              >
                {#if instanceList.length > 0}
                  <select
                    id="target-instance-select"
                    bind:value={targetInstance}
                    disabled={profileImporting}
                    class="field-select"
                  >
                    {#each instanceList as inst}
                      <option value={inst.name}>{inst.name} ({inst.version} • {inst.loader})</option>
                    {/each}
                  </select>
                {:else}
                  <div class="no-instances-alert">
                    <p class="no-instances-alert__text">{t('import.profile.noInstances')}</p>
                    <Btn variant="secondary" size="sm" onclick={openInstanceWizard}>
                      {t('import.profile.createInstance')}
                    </Btn>
                  </div>
                {/if}
              </Field>
            </div>

            <!-- Source Launcher Cards List -->
            <div class="sources-section">
              <div class="sources-section__header">
                <span class="sources-section__title">{t('import.profile.sourceListTitle')}</span>
                {#if sourcesLoading}
                  <Badge variant="neutral" size="sm">Buscando…</Badge>
                {/if}
              </div>

              {#if sourcesLoading}
                <div class="sources-loading">
                  <div class="spinner"></div>
                  <span>{t('import.profile.loadingSources')}</span>
                </div>
              {:else if sourcesError}
                <div class="sources-error">
                  <p>{t('import.profile.errorSources', { error: sourcesError })}</p>
                  <Btn variant="secondary" size="sm" onclick={loadSources}>
                    {t('import.profile.retrySources')}
                  </Btn>
                </div>
              {:else if sources.length === 0}
                <div class="sources-empty">
                  <span class="sources-empty__icon">🔍</span>
                  <p>{t('import.profile.noSourcesFound')}</p>
                </div>
              {:else}
                <div class="source-cards-grid">
                  {#each sources as src}
                    {@const accent = getKindAccent(src.kind)}
                    <!-- svelte-ignore a11y_no_static_element_interactions -->
                    <div
                      class="source-card {selectedSourceId === src.id ? 'source-card--selected' : ''} source-card--{src.kind}"
                      onclick={() => (selectedSourceId = src.id)}
                      onkeydown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault();
                          selectedSourceId = src.id;
                        }
                      }}
                      tabindex="0"
                      role="radio"
                      aria-checked={selectedSourceId === src.id}
                    >
                      <div class="source-card__top">
                        <div class="source-card__identity">
                          <span class="source-card__icon">{accent.icon}</span>
                          <div>
                            <div class="source-card__title-row">
                              <h4 class="source-card__name">{src.label || src.id}</h4>
                              <Badge variant={accent.variant} size="sm">{src.kind}</Badge>
                            </div>
                            <code class="source-card__path" title={src.path}>{src.path}</code>
                          </div>
                        </div>

                        <div class="source-card__radio-marker">
                          <span class="radio-dot {selectedSourceId === src.id ? 'radio-dot--checked' : ''}"></span>
                        </div>
                      </div>

                      <div class="source-card__meta-pills">
                        <span class="meta-pill">
                          📋 {t('import.profile.optionsCount', { count: src.options_key_count ?? 0 })}
                        </span>
                        <span class="meta-pill">
                          🌐 {t('import.profile.serversCount', { count: src.servers_entry_count ?? 0 })}
                        </span>
                        {#if src.lunar?.allocated_memory}
                          <span class="meta-pill">
                            ⚡ {t('import.profile.lunarRam', { ram: src.lunar.allocated_memory })}
                          </span>
                        {/if}
                      </div>

                      {#if src.kind === 'lunar'}
                        <div class="lunar-token-shield">
                          <span class="shield-icon">🛡️</span>
                          <span class="shield-text">{t('import.profile.lunarTokenShield')}</span>
                        </div>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>

            <!-- Overwrite Policy Configuration -->
            <div class="overwrite-policy-section">
              <span class="policy-label">{t('import.profile.overwriteTitle')}</span>
              <div class="policy-options">
                <label class="policy-card {overwritePolicy === 'never' ? 'policy-card--active' : ''}">
                  <input
                    type="radio"
                    name="overwrite-policy"
                    value="never"
                    checked={overwritePolicy === 'never'}
                    disabled={profileImporting}
                    onchange={() => (overwritePolicy = 'never')}
                    class="policy-radio"
                  />
                  <div class="policy-card__text">
                    <span class="policy-card__title">{t('import.profile.policyNever')}</span>
                    <span class="policy-card__desc">{t('import.profile.policyNeverDesc')}</span>
                  </div>
                </label>

                <label class="policy-card {overwritePolicy === 'if-older' ? 'policy-card--active' : ''}">
                  <input
                    type="radio"
                    name="overwrite-policy"
                    value="if-older"
                    checked={overwritePolicy === 'if-older'}
                    disabled={profileImporting}
                    onchange={() => (overwritePolicy = 'if-older')}
                    class="policy-radio"
                  />
                  <div class="policy-card__text">
                    <span class="policy-card__title">{t('import.profile.policyIfOlder')}</span>
                    <span class="policy-card__desc">{t('import.profile.policyIfOlderDesc')}</span>
                  </div>
                </label>
              </div>
            </div>

            <!-- Error Banner -->
            {#if profileErr}
              <div class="lane-error-banner" in:flyY={{ y: 6, duration: 150 }}>
                <span>⚠️ {profileErr}</span>
              </div>
            {/if}

            <!-- Import Action Button -->
            <div class="lane-actions">
              <Btn
                variant="primary"
                size="lg"
                block
                disabled={!targetInstance || !selectedSource || profileImporting}
                loading={profileImporting}
                onclick={doImportProfile}
              >
                {profileImporting ? t('import.profile.importing') : t('import.profile.importButton')}
              </Btn>
            </div>
          </div>
        {/if}
      </GlassCard>
    </section>
  </div>

  <!-- ==========================================================================
       BOTTOM SECTION: Recent Imports Session Summary
       ========================================================================== -->
  <section class="recent-section" in:flyY={{ y: 16, duration: 240, delay: 120 }}>
    <div class="recent-section__head">
      <div class="recent-section__title-group">
        <h3 class="recent-section__title">{t('import.recent.title')}</h3>
        <Badge variant="accent" pixel size="sm">{t('import.recent.badge')}</Badge>
      </div>
      <span class="recent-section__count">
        {recentImports.length} {recentImports.length === 1 ? 'registro' : 'registros'}
      </span>
    </div>

    {#if recentImports.length === 0}
      <div class="recent-empty">
        <span class="recent-empty__icon">📑</span>
        <p class="recent-empty__text">{t('import.recent.empty')}</p>
      </div>
    {:else}
      <div class="recent-grid">
        {#each recentImports as item (item.id)}
          <div class="recent-item" in:flyY={{ y: 8, duration: 180 }}>
            <div class="recent-item__time-col">
              <span class="recent-item__time">{formatTime(item.timestamp)}</span>
              {#if item.type === 'mrpack'}
                <Badge variant="accent" size="sm">{t('import.recent.typeMrpack')}</Badge>
              {:else}
                <Badge variant="purple" size="sm">
                  {t('import.recent.typeProfile', { kind: item.sourceKind || 'Perfil' })}
                </Badge>
              {/if}
            </div>

            <div class="recent-item__body">
              <div class="recent-item__target-row">
                <span class="recent-item__instance-label">Instancia:</span>
                <a
                  href="#/instances/{encodeURIComponent(item.instanceName)}"
                  class="recent-item__link"
                >
                  {item.instanceName} ↗
                </a>
              </div>
              <p class="recent-item__summary">{item.summary}</p>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  /* ==========================================================================
     Import Center Layout & Glass Styling
     ========================================================================== */
  .import-page {
    width: 100%;
    max-width: var(--content-max, 1280px);
    margin: 0 auto;
    padding: var(--space-6, 24px) var(--space-6, 24px) var(--space-10, 48px);
    display: flex;
    flex-direction: column;
    gap: var(--space-6, 24px);
    color: var(--text, #e8ecf4);
  }

  /* --- Header --- */
  .import-header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .import-header__badge-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .import-header__route {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
    opacity: 0.8;
  }

  .import-header__title {
    font-size: var(--text-3xl, 32px);
    font-weight: 700;
    line-height: var(--leading-tight, 1.2);
    letter-spacing: -0.03em;
  }

  .import-header__subtitle {
    font-size: var(--text-md, 15px);
    color: var(--muted, #8e9eb8);
    max-width: 780px;
    line-height: var(--leading-relaxed, 1.6);
  }

  /* --- Two-Lane Hero Layout --- */
  .import-lanes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-6, 24px);
    align-items: stretch;
  }

  @media (max-width: 1024px) {
    .import-lanes {
      grid-template-columns: 1fr;
    }
  }

  .lane {
    display: flex;
    flex-direction: column;
  }

  :global(.lane-card) {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: var(--space-6, 24px);
    background-color: var(--surface, rgba(16, 22, 42, 0.65)) !important;
  }

  .lane-header {
    margin-bottom: var(--space-5, 20px);
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .lane-header__title-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .lane-header__tag-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .lane-header__format-tag {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
  }

  .lane-header__title {
    font-size: var(--text-xl, 20px);
    font-weight: 700;
    color: var(--text, #e8ecf4);
    letter-spacing: -0.02em;
  }

  .lane-header__desc {
    font-size: var(--text-sm, 13px);
    color: var(--muted, #8e9eb8);
    line-height: var(--leading-body, 1.5);
  }

  .lane-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-5, 20px);
    flex: 1 1 auto;
  }

  /* --- LANE A: Dropzone & Memory --- */
  .hidden-file-input {
    display: none;
  }

  .dropzone {
    position: relative;
    overflow: hidden;
    border: 2px dashed var(--border, rgba(255, 255, 255, 0.12));
    background-color: var(--surface-solid, #0d1222);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-6, 24px) var(--space-4, 16px);
    text-align: center;
    cursor: pointer;
    transition:
      transform var(--transition-fast, 120ms ease),
      border-color var(--transition-fast, 120ms ease),
      background-color var(--transition-fast, 120ms ease);
    outline: none;
  }

  .dropzone:hover {
    border-color: var(--accent, #10b981);
    transform: translateY(-2px);
    background-color: var(--surface-up, rgba(25, 32, 64, 0.55));
  }

  .dropzone:focus-visible {
    border-color: var(--border-focus, #10b981);
    box-shadow: var(--focus-ring, 0 0 0 3px rgba(16, 185, 129, 0.28));
  }

  .dropzone--dragging {
    border-color: var(--accent-gold, #ffd700);
    background-color: var(--surface-up, rgba(25, 32, 64, 0.75));
    transform: scale(1.015);
    box-shadow: 0 0 24px rgba(255, 215, 0, 0.2);
  }

  .dropzone--selected {
    border-style: solid;
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.4);
  }

  .dropzone__glow {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: radial-gradient(circle at 50% 30%, rgba(var(--accent-rgb, 16, 185, 129), 0.08), transparent 70%);
  }

  .dropzone__content {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .dropzone__icon-circle {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-full, 9999px);
    background-color: rgba(var(--accent-rgb, 16, 185, 129), 0.12);
    border: 1px solid rgba(var(--accent-rgb, 16, 185, 129), 0.25);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dropzone__icon {
    font-size: 26px;
  }

  .dropzone__labels {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .dropzone__title {
    font-size: var(--text-md, 15px);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .dropzone__subtitle {
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
    word-break: break-all;
    max-width: 400px;
  }

  /* File Chip */
  .file-chip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2, 8px) var(--space-3, 12px);
    background-color: var(--surface-up-solid, #161e36);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    border-radius: var(--radius-md, 8px);
  }

  .file-chip__info {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    min-width: 0;
  }

  .file-chip__icon {
    font-size: 16px;
  }

  .file-chip__name {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 12px);
    color: var(--text, #e8ecf4);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-chip__clear {
    background: none;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: var(--text-sm, 14px);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--radius-sm, 4px);
    transition: color var(--transition-fast, 120ms ease), background-color var(--transition-fast, 120ms ease);
  }

  .file-chip__clear:hover {
    color: var(--accent-danger, #ef4444);
    background-color: rgba(239, 68, 68, 0.12);
  }

  /* Memory allocation controls */
  .memory-config {
    margin-top: var(--space-1, 4px);
  }

  .memory-input-group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .memory-input {
    width: 100%;
    max-width: 200px;
    font-family: var(--font-mono, monospace);
  }

  .memory-presets {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
  }

  .preset-btn {
    padding: 4px 10px;
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    border-radius: var(--radius-sm, 4px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    color: var(--muted, #8e9eb8);
    transition: all var(--transition-fast, 120ms ease);
  }

  .preset-btn:hover:not(:disabled) {
    background-color: var(--surface-up, rgba(25, 32, 64, 0.55));
    color: var(--text, #e8ecf4);
    border-color: var(--border-focus, #10b981);
  }

  .preset-btn--active {
    background-color: rgba(var(--accent-rgb, 16, 185, 129), 0.2) !important;
    border-color: var(--accent, #10b981) !important;
    color: var(--accent, #10b981) !important;
  }

  /* Mrpack Live Progress */
  .mrpack-progress-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-md, 8px);
  }

  .mrpack-progress-block__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--text-xs, 12px);
  }

  .mrpack-progress-block__phase {
    color: var(--text, #e8ecf4);
    font-weight: 500;
  }

  .mrpack-progress-block__counts {
    font-family: var(--font-mono, monospace);
    color: var(--accent, #10b981);
    font-weight: 600;
  }

  .text-error {
    color: var(--accent-danger, #ef4444);
  }

  /* Success Card Layout */
  .mrpack-success {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: var(--space-8, 32px) var(--space-4, 16px);
    gap: var(--space-4, 16px);
  }

  .mrpack-success__icon-box {
    width: 64px;
    height: 64px;
    border-radius: var(--radius-full, 9999px);
    background: radial-gradient(circle, rgba(16, 185, 129, 0.2), transparent 70%);
    border: 1px solid var(--accent, #10b981);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .mrpack-success__icon {
    font-size: 32px;
  }

  .mrpack-success__name {
    font-size: var(--text-2xl, 24px);
    font-weight: 700;
    color: var(--text, #e8ecf4);
    margin-top: 4px;
  }

  .mrpack-success__text {
    font-size: var(--text-sm, 14px);
    color: var(--muted, #8e9eb8);
    max-width: 360px;
    margin: 0 auto;
  }

  .mrpack-success__actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    width: 100%;
    max-width: 320px;
    margin-top: var(--space-2, 8px);
  }

  .btn-link {
    text-decoration: none;
    display: block;
    width: 100%;
  }

  .lane-actions {
    margin-top: auto;
    padding-top: var(--space-2, 8px);
  }

  /* --- LANE B: Profiles & Sources --- */
  .field-select {
    width: 100%;
    background-color: var(--surface-solid, #0d1222);
    color: var(--text, #e8ecf4);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    border-radius: var(--radius-md, 8px);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-size: var(--text-sm, 14px);
    font-family: var(--font-body, inherit);
  }

  .field-select:focus {
    border-color: var(--border-focus, #10b981);
    outline: none;
    box-shadow: var(--focus-ring, 0 0 0 3px rgba(16, 185, 129, 0.28));
  }

  .no-instances-alert {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    padding: var(--space-3, 12px);
    background-color: rgba(245, 158, 11, 0.1);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: var(--radius-md, 8px);
  }

  .no-instances-alert__text {
    font-size: var(--text-xs, 12px);
    color: var(--accent-alt, #f59e0b);
  }

  /* Sources section */
  .sources-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .sources-section__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .sources-section__title {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
  }

  .sources-loading,
  .sources-error,
  .sources-empty {
    padding: var(--space-6, 24px);
    border-radius: var(--radius-md, 8px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    text-align: center;
    font-size: var(--text-sm, 13px);
    color: var(--muted, #8e9eb8);
  }

  .sources-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3, 12px);
  }

  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, #10b981);
    border-radius: var(--radius-full, 9999px);
    animation: spin 800ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .source-cards-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    max-height: 280px;
    overflow-y: auto;
    padding-right: 4px;
  }

  .source-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-md, 8px);
    cursor: pointer;
    outline: none;
    transition:
      transform var(--transition-fast, 120ms ease),
      border-color var(--transition-fast, 120ms ease),
      background-color var(--transition-fast, 120ms ease);
  }

  .source-card:hover {
    background-color: var(--surface-up, rgba(25, 32, 64, 0.55));
    border-color: var(--hover-border, rgba(255, 255, 255, 0.2));
    transform: translateY(-1px);
  }

  .source-card:focus-visible {
    border-color: var(--border-focus, #10b981);
    box-shadow: var(--focus-ring, 0 0 0 3px rgba(16, 185, 129, 0.28));
  }

  .source-card--selected {
    border-color: var(--accent, #10b981) !important;
    background-color: var(--surface-up, rgba(25, 32, 64, 0.75)) !important;
    box-shadow: 0 0 16px rgba(16, 185, 129, 0.12);
  }

  .source-card--lunar.source-card--selected {
    border-color: var(--accent-purple, #a855f7) !important;
    box-shadow: 0 0 16px rgba(168, 85, 247, 0.15);
  }

  .source-card__top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .source-card__identity {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3, 12px);
    min-width: 0;
  }

  .source-card__icon {
    font-size: 20px;
    flex-shrink: 0;
  }

  .source-card__title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .source-card__name {
    font-size: var(--text-sm, 14px);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .source-card__path {
    display: block;
    font-size: 11px;
    color: var(--muted, #8e9eb8);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
    margin-top: 2px;
  }

  .source-card__radio-marker {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full, 9999px);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.2));
    background-color: var(--surface-solid, #0d1222);
    flex-shrink: 0;
  }

  .radio-dot {
    width: 10px;
    height: 10px;
    border-radius: var(--radius-full, 9999px);
    background-color: transparent;
    transition: background-color var(--transition-fast, 120ms ease);
  }

  .radio-dot--checked {
    background-color: var(--accent, #10b981);
  }

  .source-card--lunar .radio-dot--checked {
    background-color: var(--accent-purple, #a855f7);
  }

  .source-card__meta-pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
  }

  .meta-pill {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: var(--radius-pill, 9999px);
    background-color: rgba(255, 255, 255, 0.05);
    color: var(--muted, #8e9eb8);
  }

  .lunar-token-shield {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: 4px 8px;
    border-radius: var(--radius-sm, 4px);
    background-color: rgba(168, 85, 247, 0.1);
    border: 1px solid rgba(168, 85, 247, 0.25);
  }

  .shield-icon {
    font-size: 12px;
  }

  .shield-text {
    font-size: 11px;
    color: #d8b4fe;
    font-weight: 500;
  }

  /* Overwrite policy */
  .overwrite-policy-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .policy-label {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
  }

  .policy-options {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .policy-card {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3, 12px);
    padding: var(--space-3, 12px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-md, 8px);
    cursor: pointer;
    transition: all var(--transition-fast, 120ms ease);
  }

  .policy-card:hover {
    background-color: var(--surface-up, rgba(25, 32, 64, 0.55));
  }

  .policy-card--active {
    border-color: rgba(var(--accent-rgb, 16, 185, 129), 0.5);
    background-color: rgba(var(--accent-rgb, 16, 185, 129), 0.08);
  }

  .policy-radio {
    margin-top: 3px;
    accent-color: var(--accent, #10b981);
  }

  .policy-card__text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .policy-card__title {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .policy-card__desc {
    font-size: 11px;
    color: var(--muted, #8e9eb8);
    line-height: 1.4;
  }

  .lane-error-banner {
    padding: var(--space-2, 8px) var(--space-3, 12px);
    background-color: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius-sm, 4px);
    font-size: var(--text-xs, 12px);
    color: var(--accent-danger, #ef4444);
  }

  /* Profile Result View */
  .profile-result {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .profile-result__header {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .profile-result__title {
    font-size: var(--text-lg, 18px);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .profile-result__target {
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
  }

  .result-stats {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-2, 8px);
  }

  .result-stat-chip {
    display: flex;
    flex-direction: column;
    padding: var(--space-2, 8px) var(--space-3, 12px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
  }

  .result-stat-chip__label {
    font-size: 11px;
    color: var(--muted, #8e9eb8);
  }

  .result-stat-chip__value {
    font-size: var(--text-lg, 18px);
    font-weight: 700;
    font-family: var(--font-mono, monospace);
  }

  .text-ok {
    color: var(--accent, #10b981);
  }

  .text-muted {
    color: var(--muted, #8e9eb8);
  }

  .text-cyan {
    color: var(--accent-cyan, #06b6d4);
  }

  .text-gold {
    color: var(--accent-gold, #ffd700);
  }

  .result-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
  }

  .result-box__title {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  .copied-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .skipped-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .skipped-list__item {
    font-size: 11px;
    color: var(--muted, #8e9eb8);
  }

  .skipped-list__reason {
    opacity: 0.8;
  }

  .servers-scroll-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 120px;
    overflow-y: auto;
  }

  .server-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    background-color: rgba(255, 255, 255, 0.03);
    border-radius: var(--radius-sm, 4px);
  }

  .server-item__meta {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .server-item__name {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
  }

  .server-item__ip {
    font-size: 11px;
    color: var(--muted, #8e9eb8);
  }

  .profile-result__actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    margin-top: var(--space-2, 8px);
  }

  /* --- Recent Imports History Section --- */
  .recent-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    padding: var(--space-5, 20px);
    background-color: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-lg, 12px);
  }

  .recent-section__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .recent-section__title-group {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .recent-section__title {
    font-size: var(--text-md, 16px);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .recent-section__count {
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
  }

  .recent-empty {
    padding: var(--space-6, 24px);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .recent-empty__icon {
    font-size: 28px;
    opacity: 0.6;
  }

  .recent-empty__text {
    font-size: var(--text-sm, 13px);
    color: var(--muted, #8e9eb8);
  }

  .recent-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .recent-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3, 12px) var(--space-4, 16px);
    background-color: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
    gap: var(--space-4, 16px);
  }

  @media (max-width: 640px) {
    .recent-item {
      flex-direction: column;
      align-items: flex-start;
    }
  }

  .recent-item__time-col {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    flex-shrink: 0;
  }

  .recent-item__time {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
  }

  .recent-item__body {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: 1 1 auto;
    gap: var(--space-3, 12px);
  }

  .recent-item__target-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .recent-item__instance-label {
    font-size: var(--text-xs, 12px);
    color: var(--muted, #8e9eb8);
  }

  .recent-item__link {
    font-size: var(--text-xs, 12px);
    font-weight: 600;
    color: var(--accent, #10b981);
    text-decoration: none;
    transition: color var(--transition-fast, 120ms ease);
  }

  .recent-item__link:hover {
    color: var(--accent-gold, #ffd700);
    text-decoration: underline;
  }

  .recent-item__summary {
    font-size: var(--text-xs, 12px);
    color: var(--text, #e8ecf4);
    text-align: right;
  }
</style>
