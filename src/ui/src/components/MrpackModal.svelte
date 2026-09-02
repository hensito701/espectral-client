<!--
  ============================================================================
  MrpackModal.svelte — Horizon Glass Modrinth Modpack Importer
  ============================================================================
  z40 modal facilitating .mrpack modpack import with real-time SSE progress
  streaming, dependency download tracking, and instance registration.

  Props:
    - open?: boolean
    - onclose?: () => void
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { pickMrpackFile, importMrpack, getInstance } from '../lib/api';
  import { subscribeEvents } from '../lib/sse';
  import type { SseEvent } from '../lib/sse';
  import { instances as instancesStore } from '../lib/stores';
  import { pushToast } from '../lib/toast.svelte';
  import { scalePop, fade } from '../lib/motion';
  import Btn from './Btn.svelte';
  import Field from './Field.svelte';
  import ProgressBar from './ProgressBar.svelte';
  import { memLabel } from '../lib/format';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let {
    open = $bindable(false),
    onclose,
  }: Props = $props();

  let filePath = $state('');
  let memoryMb = $state(4096);
  let isBrowsing = $state(false);
  let isImporting = $state(false);
  let createdInstanceName = $state('');
  let alreadyExists = $state(false);

  // SSE Progress State
  let importPhase = $state<'idle' | 'post' | 'files' | 'overrides' | 'done' | 'error'>('idle');
  let filesDone = $state(0);
  let filesTotal = $state(0);
  let errorMessage = $state('');
  let watchdogTimer: ReturnType<typeof setTimeout> | null = null;
  let sseUnsub: (() => void) | null = null;

  const isWorking = $derived(
    isImporting || importPhase === 'post' || importPhase === 'files' || importPhase === 'overrides',
  );

  async function handleBrowse(): Promise<void> {
    if (isBrowsing || isWorking) return;
    isBrowsing = true;
    try {
      const res = await pickMrpackFile();
      if (res?.path) {
        filePath = res.path;
      }
    } catch (e) {
      console.error('File pick failed:', e);
    } finally {
      isBrowsing = false;
    }
  }

  async function checkWatchdog(): Promise<void> {
    watchdogTimer = null;
    if (!open || importPhase !== 'post' || !createdInstanceName) return;
    try {
      await getInstance(createdInstanceName);
      importPhase = 'done';
      void $instancesStore.refresh();
    } catch {
      // keep waiting
    }
  }

  async function startImport(): Promise<void> {
    const path = filePath.trim();
    if (!path || isWorking) return;

    isImporting = true;
    errorMessage = '';
    importPhase = 'post';
    filesDone = 0;
    filesTotal = 0;
    createdInstanceName = '';
    alreadyExists = false;

    // Attach SSE listener
    if (sseUnsub) sseUnsub();
    sseUnsub = subscribeEvents((ev: SseEvent) => {
      if (ev.type !== 'import-progress' && ev.type !== 'import-done') return;
      const d = ev.data as
        | { instance?: string; phase?: string; index?: number; total?: number }
        | { instance?: string; ok?: boolean; error?: string; already_exists?: boolean }
        | null;

      if (!d || typeof d.instance !== 'string') return;
      if (createdInstanceName && d.instance !== createdInstanceName) return;

      if (watchdogTimer) {
        clearTimeout(watchdogTimer);
        watchdogTimer = null;
      }

      if (ev.type === 'import-progress') {
        const p = d as { phase?: string; index?: number; total?: number };
        if (p.phase === 'files' || p.phase === 'overrides') {
          importPhase = p.phase;
          if (typeof p.index === 'number') filesDone = p.index + 1;
          if (typeof p.total === 'number') filesTotal = p.total;
        }
      } else {
        const done = d as { ok?: boolean; error?: string; already_exists?: boolean };
        if (done.ok) {
          importPhase = 'done';
          alreadyExists = done.already_exists === true;
          void $instancesStore.refresh();
          pushToast({
            kind: 'ok',
            text: t('home.mrpackPhaseDone'),
          });
        } else {
          importPhase = 'error';
          errorMessage = done.error ?? String(done.ok);
        }
      }
    });

    try {
      const res = await importMrpack(path, memoryMb);
      createdInstanceName = res.summary.name;
      alreadyExists = res.already_exists === true;
      if (watchdogTimer) clearTimeout(watchdogTimer);
      watchdogTimer = setTimeout(checkWatchdog, 15_000);
    } catch (e) {
      importPhase = 'error';
      errorMessage = e instanceof Error ? e.message : String(e);
    } finally {
      isImporting = false;
    }
  }

  function closeModal(): void {
    if (isWorking) return;
    if (watchdogTimer) {
      clearTimeout(watchdogTimer);
      watchdogTimer = null;
    }
    if (sseUnsub) {
      sseUnsub();
      sseUnsub = null;
    }
    open = false;
    if (onclose) onclose();
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape' && !isWorking) {
      e.stopPropagation();
      closeModal();
    }
  }

  function goToCreatedInstance(): void {
    const name = createdInstanceName;
    closeModal();
    if (name && typeof window !== 'undefined') {
      window.location.hash = `#/instances/${encodeURIComponent(name)}`;
    }
  }

  onDestroy(() => {
    if (watchdogTimer) clearTimeout(watchdogTimer);
    if (sseUnsub) sseUnsub();
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label={t('home.mrpackTitle')}
    tabindex="-1"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeModal();
    }}
    onkeydown={handleKeyDown}
    in:fade={{ duration: 150 }}
    out:fade={{ duration: 100 }}
  >
    <div
      class="mrpack-modal"
      in:scalePop={{ start: 0.94, duration: 200 }}
      out:scalePop={{ start: 0.94, duration: 150 }}
    >
      <!-- Modal Header -->
      <header class="mrpack-header">
        <div class="mrpack-header__info">
          <h2 class="mrpack-title">{t('home.mrpackTitle')}</h2>
          <p class="mrpack-subtitle">{t('home.mrpackSubtitle')}</p>
        </div>

        <button
          type="button"
          class="mrpack-close-btn"
          onclick={closeModal}
          disabled={isWorking}
          aria-label={t('home.mrpackClose')}
        >
          ✕
        </button>
      </header>

      <!-- Modal Body -->
      <div class="mrpack-body">
        {#if errorMessage}
          <div class="mrpack-error-banner animate-pop-in">
            <span class="mrpack-error-icon">⚠️</span>
            <span>{t('home.mrpackError', { error: errorMessage })}</span>
          </div>
        {/if}

        {#if importPhase === 'done'}
          <!-- Success State View -->
          <div class="mrpack-success-view animate-fade-up">
            <div class="mrpack-success-icon">✨</div>
            <h3 class="mrpack-success-title">
              {alreadyExists ? t('home.mrpackAlreadyExists') : t('home.mrpackPhaseDone')}
            </h3>
            <p class="mrpack-success-inst">
              Instancia: <strong>{createdInstanceName}</strong>
            </p>
          </div>
        {:else if isWorking}
          <!-- Progress Tracking View -->
          <div class="mrpack-progress-view animate-fade-up">
            <div class="mrpack-phase-indicator">
              <span class="mrpack-spinner"></span>
              <span class="mrpack-phase-text">
                {#if importPhase === 'post'}
                  {t('home.mrpackPhasePost')}
                {:else if importPhase === 'files'}
                  {t('home.mrpackPhaseFiles', { done: filesDone, total: filesTotal })}
                {:else if importPhase === 'overrides'}
                  {t('home.mrpackPhaseOverrides')}
                {:else}
                  {t('home.mrpackImporting')}
                {/if}
              </span>
            </div>

            <ProgressBar
              value={filesTotal > 0 ? filesDone : undefined}
              max={filesTotal > 0 ? filesTotal : 100}
              indeterminate={importPhase === 'post' || filesTotal === 0}
              height="lg"
              tone="accent"
              showValue={filesTotal > 0}
            />
          </div>
        {:else}
          <!-- Input Form View -->
          <div class="mrpack-form-stack">
            <Field label={t('home.mrpackFile')} required>
              <div class="mrpack-file-row">
                <input
                  type="text"
                  class="mrpack-input"
                  placeholder={t('home.mrpackFilePlaceholder')}
                  bind:value={filePath}
                />
                <Btn
                  variant="secondary"
                  loading={isBrowsing}
                  onclick={handleBrowse}
                >
                  📁 {t('home.mrpackBrowse')}
                </Btn>
              </div>
            </Field>

            <Field label={t('home.mrpackMemory')}>
              <div class="mrpack-memory-row">
                <input
                  type="range"
                  class="mrpack-range"
                  min="2048"
                  max="16384"
                  step="512"
                  bind:value={memoryMb}
                />
                <span class="mrpack-memory-badge">{memLabel(memoryMb)}</span>
              </div>
            </Field>
          </div>
        {/if}
      </div>

      <!-- Modal Footer -->
      <footer class="mrpack-footer">
        <div class="mrpack-footer__left">
          {#if importPhase === 'done'}
            <Btn variant="primary" onclick={goToCreatedInstance}>
              🚀 {t('home.mrpackGoToInstance')}
            </Btn>
          {/if}
        </div>

        <div class="mrpack-footer__right">
          {#if importPhase === 'done'}
            <Btn variant="secondary" onclick={closeModal}>
              {t('home.mrpackClose')}
            </Btn>
          {:else}
            <Btn variant="ghost" disabled={isWorking} onclick={closeModal}>
              {t('home.wizardCancelBtn')}
            </Btn>
            <Btn
              variant="primary"
              loading={isWorking}
              disabled={!filePath.trim() || isWorking}
              onclick={startImport}
            >
              📥 {t('home.mrpackImportBtn')}
            </Btn>
          {/if}
        </div>
      </footer>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(3, 6, 14, 0.78);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    z-index: var(--z-modal, 40);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4, 16px);
  }

  .mrpack-modal {
    width: 100%;
    max-width: 600px;
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-xl, 1.25rem);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.8), 0 0 32px rgba(6, 182, 212, 0.1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .mrpack-header {
    padding: var(--space-5, 20px) var(--space-6, 24px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: rgba(16, 22, 42, 0.4);
  }

  .mrpack-title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-xl, 1.375rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .mrpack-subtitle {
    margin: 2px 0 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .mrpack-close-btn {
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 16px;
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    transition: color var(--dur-fast, 120ms) ease;
  }

  .mrpack-close-btn:hover:not(:disabled) {
    color: var(--text, #e8ecf4);
  }

  .mrpack-body {
    padding: var(--space-6, 24px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .mrpack-error-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: var(--radius-md, 0.625rem);
    color: #fca5a5;
    font-size: var(--text-sm, 0.875rem);
  }

  .mrpack-form-stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .mrpack-file-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .mrpack-input {
    flex: 1;
    font-size: var(--text-sm, 0.875rem) !important;
  }

  .mrpack-memory-row {
    display: flex;
    align-items: center;
    gap: var(--space-4, 16px);
  }

  .mrpack-range {
    flex: 1;
    height: 6px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 3px;
    outline: none;
  }

  .mrpack-memory-badge {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    color: var(--accent-cyan, #06b6d4);
    min-width: 90px;
    text-align: right;
  }

  /* Progress Tracking View */
  .mrpack-progress-view {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    padding: var(--space-4, 16px) 0;
  }

  .mrpack-phase-indicator {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .mrpack-spinner {
    width: 18px;
    height: 18px;
    border: 2px solid rgba(6, 182, 212, 0.2);
    border-top-color: var(--accent-cyan, #06b6d4);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .mrpack-phase-text {
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #e8ecf4);
  }

  /* Success View */
  .mrpack-success-view {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: var(--space-6, 24px) 0;
  }

  .mrpack-success-icon {
    font-size: 48px;
    margin-bottom: var(--space-2, 8px);
  }

  .mrpack-success-title {
    margin: 0 0 var(--space-2, 8px) 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-xl, 1.375rem);
    font-weight: 700;
    color: #22c55e;
  }

  .mrpack-success-inst {
    margin: 0;
    font-size: var(--text-base, 1rem);
    color: var(--muted-strong, #b3c5e3);
  }

  /* Footer */
  .mrpack-footer {
    padding: var(--space-4, 16px) var(--space-6, 24px);
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(16, 22, 42, 0.4);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .mrpack-footer__right {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }
</style>
