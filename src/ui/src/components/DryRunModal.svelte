<!--
  ============================================================================
  DryRunModal.svelte — Horizon Glass Launch Dry-Run Inspector
  ============================================================================
  z40 modal inspecting JVM arguments, execution command, classpath entries,
  natives directory, assets validity, and warnings before actual game boot.

  Listens globally to 'horizon:dryrun' events with { detail: { name, result? } }.

  Props:
    - open?: boolean
    - instanceName?: string
    - result?: DryRunResult | null
    - onclose?: () => void
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { DryRunResult } from '../lib/types';
  import { launchInstance } from '../lib/api';
  import { useCopy } from '../lib/useCopy.svelte';
  import { scalePop, fade } from '../lib/motion';
  import Btn from './Btn.svelte';
  import Badge from './Badge.svelte';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    open?: boolean;
    instanceName?: string;
    result?: DryRunResult | null;
    onclose?: () => void;
  }

  let {
    open = $bindable(false),
    instanceName = $bindable(''),
    result = $bindable<DryRunResult | null>(null),
    onclose,
  }: Props = $props();

  let activeTab = $state<'command' | 'classpath' | 'env' | 'warnings'>('command');
  let isLoading = $state(false);
  let errorMsg = $state('');

  const cmdCopier = useCopy(1800);
  const cpCopier = useCopy(1800);

  const fullCommandLine = $derived.by(() => {
    if (!result) return '';
    const java = result.java_path || 'javaw.exe';
    const args = (result.argv || []).map((a) => (a.includes(' ') ? `"${a}"` : a)).join(' ');
    return `${java} ${args}`;
  });

  const fullClasspath = $derived.by(() => {
    if (!result || !result.classpath) return '';
    return result.classpath.join(';');
  });

  async function fetchDryRun(name: string): Promise<void> {
    if (!name) return;
    isLoading = true;
    errorMsg = '';
    result = null;

    try {
      const res = (await launchInstance(name, {
        mode: 'normal',
        dry_run: true,
      })) as unknown as DryRunResult;
      result = res;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      isLoading = false;
    }
  }

  function closeModal(): void {
    open = false;
    if (onclose) onclose();
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.stopPropagation();
      closeModal();
    }
  }

  onMount(() => {
    const onDryRunEvent = (e: Event) => {
      const detail = (e as CustomEvent<{ name: string; result?: DryRunResult }>).detail;
      if (detail?.name) {
        instanceName = detail.name;
        open = true;
        if (detail.result) {
          result = detail.result;
          isLoading = false;
          errorMsg = '';
        } else {
          void fetchDryRun(detail.name);
        }
      }
    };

    window.addEventListener('horizon:dryrun', onDryRunEvent);
    return () => window.removeEventListener('horizon:dryrun', onDryRunEvent);
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label={t('home.dryRunTitle')}
    tabindex="-1"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeModal();
    }}
    onkeydown={handleKeyDown}
    in:fade={{ duration: 150 }}
    out:fade={{ duration: 100 }}
  >
    <div
      class="dryrun-modal"
      in:scalePop={{ start: 0.94, duration: 200 }}
      out:scalePop={{ start: 0.94, duration: 150 }}
    >
      <!-- Modal Header -->
      <header class="dryrun-header">
        <div class="dryrun-header__info">
          <div class="dryrun-header__title-row">
            <h2 class="dryrun-title">{t('home.dryRunTitle')}</h2>
            {#if instanceName}
              <Badge variant="accent" size="sm">{instanceName}</Badge>
            {/if}
          </div>
          <p class="dryrun-subtitle">{t('home.dryRunSubtitle')}</p>
        </div>

        <button
          type="button"
          class="dryrun-close-btn"
          onclick={closeModal}
          aria-label={t('home.dryRunClose')}
        >
          ✕
        </button>
      </header>

      <!-- Navigation Tabs -->
      <nav class="dryrun-tabs">
        <button
          type="button"
          class="dryrun-tab"
          class:dryrun-tab--active={activeTab === 'command'}
          onclick={() => (activeTab = 'command')}
        >
          💻 {t('home.dryRunCommand')}
        </button>
        <button
          type="button"
          class="dryrun-tab"
          class:dryrun-tab--active={activeTab === 'classpath'}
          onclick={() => (activeTab = 'classpath')}
        >
          📦 {t('home.dryRunClasspath', { count: result?.classpath?.length || 0 })}
        </button>
        <button
          type="button"
          class="dryrun-tab"
          class:dryrun-tab--active={activeTab === 'env'}
          onclick={() => (activeTab = 'env')}
        >
          ⚙️ Entorno JVM
        </button>
        <button
          type="button"
          class="dryrun-tab"
          class:dryrun-tab--active={activeTab === 'warnings'}
          onclick={() => (activeTab = 'warnings')}
        >
          ⚠️ {t('home.dryRunWarnings', { count: result?.warnings?.length || 0 })}
        </button>
      </nav>

      <!-- Modal Body -->
      <div class="dryrun-body">
        {#if isLoading}
          <div class="dryrun-loading-state animate-fade-up">
            <span class="dryrun-spinner"></span>
            <p>{t('home.dryRunLoading')}</p>
          </div>
        {:else if errorMsg}
          <div class="dryrun-error-banner animate-pop-in">
            <span class="dryrun-error-icon">⚠️</span>
            <span>{errorMsg}</span>
          </div>
        {:else if result}
          <!-- Tab 1: Command & Argv -->
          {#if activeTab === 'command'}
            <div class="dryrun-tab-content animate-fade-up">
              <div class="dryrun-box-header">
                <span class="dryrun-box-label">{t('home.dryRunCommand')}</span>
                <Btn
                  variant="secondary"
                  size="sm"
                  onclick={() => cmdCopier.copy(fullCommandLine)}
                >
                  {cmdCopier.copied ? t('home.dryRunCopied') : t('home.dryRunCopyCmd')}
                </Btn>
              </div>
              <pre class="dryrun-code-block font-mono"><code>{fullCommandLine}</code></pre>

              <div class="dryrun-box-header" style="margin-top: var(--space-4, 16px);">
                <span class="dryrun-box-label">Argumentos Detallados ({result.argv?.length || 0})</span>
              </div>
              <div class="dryrun-argv-list">
                {#each result.argv || [] as arg, i}
                  <div class="dryrun-argv-item">
                    <span class="dryrun-argv-idx">{i}</span>
                    <code class="dryrun-argv-val font-mono">{arg}</code>
                  </div>
                {/each}
              </div>
            </div>

          <!-- Tab 2: Classpath Inspector -->
          {:else if activeTab === 'classpath'}
            <div class="dryrun-tab-content animate-fade-up">
              <div class="dryrun-box-header">
                <span class="dryrun-box-label">{t('home.dryRunClasspath', { count: result.classpath?.length || 0 })}</span>
                <Btn
                  variant="secondary"
                  size="sm"
                  onclick={() => cpCopier.copy(fullClasspath)}
                >
                  {cpCopier.copied ? t('home.dryRunCopied') : t('home.dryRunCopyClasspath')}
                </Btn>
              </div>

              {#if result.classpath_missing && result.classpath_missing.length > 0}
                <div class="dryrun-missing-box">
                  <span class="dryrun-missing-title">{t('home.dryRunMissingCp', { count: result.classpath_missing.length })}:</span>
                  <ul class="dryrun-missing-list">
                    {#each result.classpath_missing as m}
                      <li><code>{m}</code></li>
                    {/each}
                  </ul>
                </div>
              {/if}

              <div class="dryrun-cp-list">
                {#each result.classpath || [] as cp}
                  <div class="dryrun-cp-item font-mono">
                    <span class="dryrun-cp-bullet">▸</span>
                    <span class="dryrun-cp-path">{cp}</span>
                  </div>
                {/each}
              </div>
            </div>

          <!-- Tab 3: Environment & Natives -->
          {:else if activeTab === 'env'}
            <div class="dryrun-tab-content animate-fade-up">
              <div class="dryrun-meta-grid">
                <div class="dryrun-meta-card">
                  <span class="dryrun-meta-lbl">{t('home.dryRunJava')}</span>
                  <code class="dryrun-meta-val font-mono">{result.java_path}</code>
                </div>

                <div class="dryrun-meta-card">
                  <span class="dryrun-meta-lbl">{t('home.dryRunNatives')}</span>
                  <code class="dryrun-meta-val font-mono">{result.natives_dir}</code>
                </div>

                <div class="dryrun-meta-card">
                  <span class="dryrun-meta-lbl">{t('home.dryRunAssets')}</span>
                  <div class="dryrun-meta-val">
                    {#if result.assets_ok}
                      <Badge variant="ok" size="sm">✓ {t('home.dryRunAssetsOk')}</Badge>
                    {:else}
                      <Badge variant="error" size="sm">✗ {t('home.dryRunAssetsMissing')}</Badge>
                    {/if}
                  </div>
                </div>
              </div>
            </div>

          <!-- Tab 4: Warnings -->
          {:else if activeTab === 'warnings'}
            <div class="dryrun-tab-content animate-fade-up">
              {#if result.warnings && result.warnings.length > 0}
                <div class="dryrun-warnings-list">
                  {#each result.warnings as warn}
                    <div class="dryrun-warning-item">
                      <span class="dryrun-warning-icon">⚠️</span>
                      <span class="dryrun-warning-text">{warn}</span>
                    </div>
                  {/each}
                </div>
              {:else}
                <div class="dryrun-no-warnings">
                  <span class="dryrun-ok-icon">✓</span>
                  <p>{t('home.dryRunNoWarnings')}</p>
                </div>
              {/if}
            </div>
          {/if}
        {/if}
      </div>

      <!-- Modal Footer -->
      <footer class="dryrun-footer">
        <div class="dryrun-footer__info">
          {#if result?.assets_ok}
            <span class="dryrun-status-badge">● Entorno Verificado</span>
          {/if}
        </div>
        <Btn variant="primary" onclick={closeModal}>
          {t('home.dryRunClose')}
        </Btn>
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

  .dryrun-modal {
    width: 100%;
    max-width: 760px;
    max-height: 85vh;
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-xl, 1.25rem);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.8), 0 0 32px rgba(16, 185, 129, 0.1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dryrun-header {
    padding: var(--space-4, 16px) var(--space-6, 24px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    justify-content: space-between;
    background: rgba(16, 22, 42, 0.4);
  }

  .dryrun-header__title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .dryrun-title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .dryrun-subtitle {
    margin: 2px 0 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .dryrun-close-btn {
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 16px;
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    transition: color var(--dur-fast, 120ms) ease;
  }

  .dryrun-close-btn:hover {
    color: var(--text, #e8ecf4);
  }

  /* Navigation Tabs */
  .dryrun-tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1, 4px);
    padding: 0 var(--space-6, 24px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    background: rgba(10, 15, 30, 0.4);
    overflow-x: auto;
  }

  .dryrun-tab {
    padding: var(--space-3, 12px) var(--space-3, 12px);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--muted, #8e9eb8);
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
    white-space: nowrap;
  }

  .dryrun-tab:hover {
    color: var(--text, #e8ecf4);
  }

  .dryrun-tab--active {
    color: var(--accent, #10b981);
    border-bottom-color: var(--accent, #10b981);
  }

  /* Body */
  .dryrun-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-6, 24px);
  }

  .dryrun-loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-8, 32px);
    gap: var(--space-3, 12px);
    color: var(--muted, #8e9eb8);
  }

  .dryrun-spinner {
    width: 24px;
    height: 24px;
    border: 2px solid rgba(16, 185, 129, 0.2);
    border-top-color: var(--accent, #10b981);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .dryrun-error-banner {
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

  .dryrun-tab-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .dryrun-box-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .dryrun-box-label {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    color: var(--muted-strong, #b3c5e3);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .dryrun-code-block {
    margin: 0;
    padding: var(--space-3, 12px);
    background: var(--log-bg, #080c18);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: var(--radius-md, 0.625rem);
    color: #a7f3d0;
    font-size: var(--text-xs, 0.75rem);
    line-height: 1.4;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 180px;
  }

  .dryrun-argv-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: var(--radius-md, 0.625rem);
    background: rgba(8, 12, 24, 0.5);
    padding: 6px;
  }

  .dryrun-argv-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2, 8px);
    padding: 4px 6px;
    border-radius: var(--radius-xs, 0.25rem);
    font-size: var(--text-xs, 0.75rem);
  }

  .dryrun-argv-item:hover {
    background: rgba(255, 255, 255, 0.04);
  }

  .dryrun-argv-idx {
    color: var(--muted, #8e9eb8);
    min-width: 20px;
    user-select: none;
  }

  .dryrun-argv-val {
    color: var(--text, #e8ecf4);
    word-break: break-all;
  }

  /* Classpath */
  .dryrun-missing-box {
    padding: var(--space-3, 12px);
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius-md, 0.625rem);
    margin-bottom: var(--space-3, 12px);
  }

  .dryrun-missing-title {
    font-weight: 700;
    color: #ef4444;
    font-size: var(--text-xs, 0.75rem);
  }

  .dryrun-missing-list {
    margin: 4px 0 0 0;
    padding-left: 18px;
    font-size: var(--text-xs, 0.75rem);
    color: #fca5a5;
  }

  .dryrun-cp-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 320px;
    overflow-y: auto;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: var(--radius-md, 0.625rem);
    background: rgba(8, 12, 24, 0.5);
    padding: 6px;
  }

  .dryrun-cp-item {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 6px;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted-strong, #b3c5e3);
    border-radius: var(--radius-xs, 0.25rem);
  }

  .dryrun-cp-item:hover {
    background: rgba(255, 255, 255, 0.04);
    color: var(--text, #e8ecf4);
  }

  .dryrun-cp-bullet {
    color: var(--accent, #10b981);
    user-select: none;
  }

  .dryrun-cp-path {
    word-break: break-all;
  }

  /* Environment */
  .dryrun-meta-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-3, 12px);
  }

  .dryrun-meta-card {
    padding: var(--space-4, 16px);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .dryrun-meta-lbl {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    color: var(--muted, #8e9eb8);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .dryrun-meta-val {
    color: var(--text, #e8ecf4);
    font-size: var(--text-sm, 0.875rem);
    word-break: break-all;
  }

  /* Warnings */
  .dryrun-warnings-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .dryrun-warning-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    background: rgba(245, 158, 11, 0.12);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: var(--radius-md, 0.625rem);
    color: #fde68a;
    font-size: var(--text-sm, 0.875rem);
  }

  .dryrun-no-warnings {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-8, 32px);
    text-align: center;
    color: #22c55e;
  }

  .dryrun-ok-icon {
    font-size: 36px;
    margin-bottom: var(--space-2, 8px);
  }

  /* Footer */
  .dryrun-footer {
    padding: var(--space-4, 16px) var(--space-6, 24px);
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(16, 22, 42, 0.4);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .dryrun-status-badge {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: #22c55e;
  }
</style>
