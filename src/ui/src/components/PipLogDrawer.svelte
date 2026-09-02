<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { launchLog, liveLaunches } from '../lib/stores';
  import type { LaunchBuffer } from '../lib/stores';
  import type { LiveLaunch } from '../lib/types';
  import { stopInstance } from '../lib/api';
  import { useCopy } from '../lib/useCopy.svelte';
  import { t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';

  const STORAGE_KEY_PINNED = 'espectral_pip_pinned';

  let isExpanded = $state(false);
  let isPinned = $state(false);
  let autoScroll = $state(true);
  let userScrolledUp = $state(false);
  let stoppingKey = $state<string | null>(null);
  let dismissedKeys = $state<Set<string>>(new Set());
  let seenLaunchKeys = $state<Set<string>>(new Set());
  let terminalEl = $state<HTMLDivElement | null>(null);

  const clipboard = useCopy(1600);

  // Buffer state derivations
  const buffersMap = $derived<Record<string, LaunchBuffer>>($launchLog.buffers ?? {});
  const bufferList = $derived<LaunchBuffer[]>(Object.values(buffersMap));
  const activeKey = $derived<string | null>($launchLog.activeKey);

  // Active buffer resolution
  const activeBuffer = $derived.by<LaunchBuffer | null>(() => {
    if (activeKey && buffersMap[activeKey]) {
      return buffersMap[activeKey];
    }
    if (bufferList.length > 0) {
      return bufferList[bufferList.length - 1];
    }
    return null;
  });

  // Live launches derivation
  const liveList = $derived<LiveLaunch[]>($liveLaunches.value ?? []);
  const runningLaunches = $derived<LiveLaunch[]>(liveList.filter((l) => l.running));
  const runningCount = $derived<number>(runningLaunches.length);

  // Active lines and running state
  const lines = $derived<string[]>(activeBuffer?.lines ?? []);
  const isRunning = $derived<boolean>(activeBuffer?.running ?? false);

  // Load pinned state & register hotkeys
  onMount(() => {
    try {
      const savedPin = localStorage.getItem(STORAGE_KEY_PINNED);
      if (savedPin === 'true') {
        isPinned = true;
        isExpanded = true;
      }
    } catch {
      isPinned = false;
    }

    const onCustomToggle = () => {
      toggleExpanded();
    };
    window.addEventListener('horizon:toggle-pip-log', onCustomToggle);

    const onGlobalKeyDown = (e: KeyboardEvent) => {
      // Space key shortcut when not typing in an input
      if (e.code === 'Space' || e.key === ' ') {
        const target = e.target as HTMLElement | null;
        const tag = target?.tagName?.toLowerCase();
        const isEditable = target?.isContentEditable || tag === 'input' || tag === 'textarea' || tag === 'select';
        if (!isEditable) {
          e.preventDefault();
          toggleExpanded();
          return;
        }
      }

      // Escape collapses if not pinned
      if (e.key === 'Escape' && isExpanded && !isPinned) {
        collapseDrawer();
      }
    };

    window.addEventListener('keydown', onGlobalKeyDown);

    return () => {
      window.removeEventListener('horizon:toggle-pip-log', onCustomToggle);
      window.removeEventListener('keydown', onGlobalKeyDown);
    };
  });

  // Auto-expand when a new launch appears (never stealing focus)
  $effect(() => {
    const running = runningLaunches;
    if (running.length === 0) return;

    for (const r of running) {
      if (!seenLaunchKeys.has(r.key)) {
        seenLaunchKeys.add(r.key);
        // Automatically switch active buffer and expand if not dismissed
        if (!dismissedKeys.has(r.key)) {
          launchLog.setActive(r.key);
          isExpanded = true;
        }
      }
    }
  });

  // Auto-scroll effect on new lines
  $effect(() => {
    // depend on lines count
    const count = lines.length;
    if (count > 0 && autoScroll && !userScrolledUp && terminalEl) {
      void tick().then(() => {
        if (terminalEl && autoScroll && !userScrolledUp) {
          terminalEl.scrollTop = terminalEl.scrollHeight;
        }
      });
    }
  });

  function toggleExpanded(): void {
    if (isExpanded) {
      collapseDrawer();
    } else {
      isExpanded = true;
    }
  }

  function collapseDrawer(): void {
    isExpanded = false;
    if (activeBuffer?.key) {
      dismissedKeys.add(activeBuffer.key);
    }
  }

  function togglePin(): void {
    isPinned = !isPinned;
    try {
      localStorage.setItem(STORAGE_KEY_PINNED, isPinned ? 'true' : 'false');
    } catch {
      // ignore storage error
    }
  }

  function toggleAutoScroll(): void {
    autoScroll = !autoScroll;
    if (autoScroll && terminalEl) {
      userScrolledUp = false;
      terminalEl.scrollTop = terminalEl.scrollHeight;
    }
  }

  function onTerminalScroll(): void {
    if (!terminalEl) return;
    const distFromBottom = terminalEl.scrollHeight - terminalEl.scrollTop - terminalEl.clientHeight;
    userScrolledUp = distFromBottom > 28;
  }

  function handleCopy(): void {
    if (lines.length === 0) return;
    const text = lines.join('\n');
    clipboard.copy(text);
    pushToast({ kind: 'ok', text: t('pip.copied') });
  }

  function handleClear(): void {
    launchLog.clear();
  }

  async function handleStop(): Promise<void> {
    const inst = activeBuffer?.instance;
    const key = activeBuffer?.key;
    if (!inst) return;

    stoppingKey = key ?? inst;
    try {
      await stopInstance(inst);
      pushToast({ kind: 'info', text: t('pip.stopSuccess') });
    } catch (err) {
      pushToast({ kind: 'err', text: String(err) });
    } finally {
      stoppingKey = null;
    }
  }

  function cleanAnsi(line: string): string {
    return line.replace(/(\u001b\[[0-9;]*[a-zA-Z]|\u001b\([a-zA-Z])/g, '');
  }

  function isErrorLine(clean: string): boolean {
    return /(^|\s)(ERROR|SEVERE|Exception|Caused by|Failed to|Error:)/i.test(clean);
  }

  function isWarnLine(clean: string): boolean {
    return /(^|\s)(WARN|WARNING)/i.test(clean);
  }
</script>

<div class="pip-drawer-container" class:pip-drawer-container--expanded={isExpanded}>
  {#if !isExpanded}
    <!-- ====================================================================
         COLLAPSED HUD PILL (z20)
         ==================================================================== -->
    <div
      class="pip-pill"
      role="button"
      tabindex="0"
      aria-expanded="false"
      aria-label={t('pip.title')}
      onclick={toggleExpanded}
      onkeydown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          toggleExpanded();
        }
      }}
    >
      <div class="pip-pill__left">
        <!-- Status Indicator Dot -->
        <div
          class="pip-status-dot"
          class:pip-status-dot--running={runningCount > 0}
          class:pip-status-dot--idle={runningCount === 0}
          aria-hidden="true"
        ></div>

        <!-- Running Count / Summary -->
        <span class="pip-pill__title">
          {#if runningCount > 0}
            <span class="pip-pill__count font-pixel">{runningCount}</span>
            <span class="pip-pill__label">
              {runningCount === 1 ? t('pip.runningOne') : t('pip.runningMany', { count: runningCount })}
            </span>
          {:else if activeBuffer}
            <span class="pip-pill__label">{activeBuffer.instance || t('pip.title')}</span>
          {:else}
            <span class="pip-pill__label">{t('pip.noActiveLaunch')}</span>
          {/if}
        </span>
      </div>

      <div class="pip-pill__right">
        {#if activeBuffer && isRunning}
          <button
            type="button"
            class="pip-btn pip-btn--stop-micro font-pixel"
            title={t('pip.kill')}
            disabled={stoppingKey !== null}
            onclick={(e) => {
              e.stopPropagation();
              void handleStop();
            }}
          >
            {stoppingKey ? '…' : '■'}
          </button>
        {/if}

        <span class="pip-pill__shortcut font-pixel">[ESPACIO]</span>
        <span class="pip-pill__chevron" aria-hidden="true">▲</span>
      </div>
    </div>
  {:else}
    <!-- ====================================================================
         EXPANDED TERMINAL DRAWER (z20)
         ==================================================================== -->
    <div class="pip-panel" role="region" aria-label={t('pip.title')}>
      <!-- Top Control Bar / Tab Bar -->
      <div class="pip-panel__header">
        <div class="pip-panel__tabs">
          {#if bufferList.length > 0}
            {#each bufferList as buf (buf.key)}
              {@const isCur = activeKey === buf.key || (!activeKey && activeBuffer?.key === buf.key)}
              <button
                type="button"
                class="pip-tab"
                class:pip-tab--active={isCur}
                onclick={() => launchLog.setActive(buf.key)}
              >
                <span
                  class="pip-tab__dot"
                  class:pip-tab__dot--running={buf.running}
                  aria-hidden="true"
                ></span>
                <span class="pip-tab__name">{buf.instance || buf.key.slice(0, 8)}</span>
                {#if buf.account}
                  <span class="pip-tab__account">({buf.account})</span>
                {/if}
              </button>
            {/each}
          {:else}
            <div class="pip-tab pip-tab--active">
              <span class="pip-tab__dot pip-tab__dot--idle" aria-hidden="true"></span>
              <span class="pip-tab__name">{t('pip.title')}</span>
            </div>
          {/if}
        </div>

        <!-- Terminal Actions Toolbar -->
        <div class="pip-panel__actions">
          <span class="pip-lines-counter font-pixel">
            {t('pip.linesCount', { count: lines.length })}
          </span>

          <!-- Auto-scroll Toggle -->
          <button
            type="button"
            class="pip-tool-btn"
            class:pip-tool-btn--active={autoScroll}
            title={autoScroll ? t('pip.autoscrollOn') : t('pip.autoscrollOff')}
            onclick={toggleAutoScroll}
          >
            ↓
          </button>

          <!-- Copy Button -->
          <button
            type="button"
            class="pip-tool-btn"
            title={t('pip.copy')}
            onclick={handleCopy}
            disabled={lines.length === 0}
          >
            {clipboard.copied ? '✓' : '⧉'}
          </button>

          <!-- Clear Button -->
          <button
            type="button"
            class="pip-tool-btn"
            title={t('pip.clear')}
            onclick={handleClear}
            disabled={lines.length === 0}
          >
            ⊘
          </button>

          <!-- Stop Button -->
          {#if activeBuffer && isRunning}
            <button
              type="button"
              class="pip-btn pip-btn--stop font-pixel"
              title={t('pip.kill')}
              disabled={stoppingKey !== null}
              onclick={() => void handleStop()}
            >
              {stoppingKey ? t('pip.killing') : t('pip.kill')}
            </button>
          {/if}

          <!-- Pin Button -->
          <button
            type="button"
            class="pip-tool-btn"
            class:pip-tool-btn--active={isPinned}
            title={isPinned ? t('pip.unpin') : t('pip.pin')}
            onclick={togglePin}
          >
            📌
          </button>

          <!-- Minimize / Close Button -->
          <button
            type="button"
            class="pip-tool-btn pip-tool-btn--close"
            title={t('pip.collapse')}
            onclick={collapseDrawer}
          >
            ▼
          </button>
        </div>
      </div>

      <!-- Monospace Dark Terminal Body -->
      <div
        class="pip-panel__terminal"
        bind:this={terminalEl}
        onscroll={onTerminalScroll}
      >
        {#if lines.length === 0}
          <div class="pip-terminal__empty">
            <span class="pip-terminal__cursor">_</span>
            <span>{t('pip.waitingLogs')}</span>
          </div>
        {:else}
          {#each lines as rawLine, i (i)}
            {@const clean = cleanAnsi(rawLine)}
            {@const isErr = isErrorLine(clean)}
            {@const isWrn = isWarnLine(clean)}
            <div
              class="pip-line"
              class:pip-line--error={isErr}
              class:pip-line--warn={isWrn}
            >
              <span class="pip-line__num">{i + 1}</span>
              <span class="pip-line__text">{clean}</span>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .pip-drawer-container {
    position: fixed;
    bottom: calc(var(--statusbar-height, 28px) + var(--space-2));
    right: var(--space-4);
    z-index: var(--z-pip, 20);
    pointer-events: auto;
    transform: translateY(0);
    transition: transform var(--dur-med) var(--ease-out-expo),
                opacity var(--dur-med) var(--ease-out-expo);
    will-change: transform, opacity;
  }

  /* ========================================================================
     COLLAPSED PILL STYLES
     ======================================================================== */
  .pip-pill {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    height: 36px;
    padding: 0 var(--space-3) 0 var(--space-3);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    box-shadow: var(--shadow-md);
    cursor: pointer;
    user-select: none;
    transition: background var(--dur-fast) var(--ease-out-expo),
                border-color var(--dur-fast) var(--ease-out-expo),
                transform var(--dur-fast) var(--ease-out-expo);
  }

  .pip-pill:hover {
    background: var(--surface-up);
    border-color: var(--border-focus);
    transform: translateY(-1px);
  }

  .pip-pill__left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pip-status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--muted);
    flex-shrink: 0;
  }

  .pip-status-dot--running {
    background: var(--accent);
    box-shadow: 0 0 10px rgba(var(--accent-rgb), 0.7);
    animation: pipDotPulse 1.8s ease-in-out infinite;
  }

  .pip-status-dot--idle {
    background: var(--muted);
    opacity: 0.5;
  }

  .pip-pill__title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text);
  }

  .pip-pill__count {
    font-size: 0.5625rem;
    color: var(--accent);
    background: rgba(var(--accent-rgb), 0.15);
    padding: 2px 5px;
    border-radius: var(--radius-xs);
    border: 1px solid rgba(var(--accent-rgb), 0.3);
  }

  .pip-pill__label {
    font-weight: 500;
  }

  .pip-pill__right {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .pip-pill__shortcut {
    font-size: 0.5625rem;
    color: var(--muted);
    opacity: 0.7;
  }

  .pip-pill__chevron {
    font-size: 0.625rem;
    color: var(--muted);
  }

  /* ========================================================================
     EXPANDED TERMINAL PANEL STYLES
     ======================================================================== */
  .pip-panel {
    width: clamp(520px, 52vw, 840px);
    height: clamp(240px, 36vh, 40vh);
    max-height: 40vh;
    display: flex;
    flex-direction: column;
    background: var(--log-bg, #080c18);
    border: 1px solid var(--border-solid);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg), 0 0 24px rgba(0, 0, 0, 0.4);
    overflow: hidden;
    animation: pipSlideUp var(--dur-med) var(--ease-out-expo) both;
  }

  .pip-panel__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 38px;
    padding: 0 var(--space-2);
    background: var(--surface-solid);
    border-bottom: 1px solid var(--border);
    user-select: none;
    flex-shrink: 0;
  }

  .pip-panel__tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    overflow-x: auto;
    max-width: 60%;
  }

  .pip-tab {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-sm);
    background: transparent;
    border: 1px solid transparent;
    color: var(--muted);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--dur-fast), color var(--dur-fast);
  }

  .pip-tab:hover {
    background: var(--surface-up);
    color: var(--text);
  }

  .pip-tab--active {
    background: var(--surface-up-solid);
    border-color: var(--border-focus);
    color: var(--text);
  }

  .pip-tab__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--muted);
  }

  .pip-tab__dot--running {
    background: var(--accent);
    box-shadow: 0 0 6px rgba(var(--accent-rgb), 0.6);
  }

  .pip-tab__dot--idle {
    background: var(--muted);
  }

  .pip-tab__name {
    font-weight: 600;
  }

  .pip-tab__account {
    font-size: 0.6875rem;
    color: var(--muted);
  }

  .pip-panel__actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .pip-lines-counter {
    font-size: 0.5625rem;
    color: var(--muted);
    margin-right: var(--space-2);
    opacity: 0.8;
  }

  .pip-tool-btn {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-xs);
    color: var(--muted);
    font-size: 0.8125rem;
    cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }

  .pip-tool-btn:hover {
    background: var(--surface-up);
    color: var(--text);
    border-color: var(--border);
  }

  .pip-tool-btn--active {
    background: rgba(var(--accent-rgb), 0.15);
    color: var(--accent);
    border-color: rgba(var(--accent-rgb), 0.3);
  }

  .pip-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--radius-xs);
    font-size: 0.5625rem;
    letter-spacing: 0.05em;
    cursor: pointer;
    user-select: none;
    transition: background var(--dur-fast), transform var(--dur-fast);
  }

  .pip-btn--stop {
    padding: 3px 8px;
    background: rgba(var(--accent-red-rgb), 0.18);
    border: 1px solid rgba(var(--accent-red-rgb), 0.4);
    color: var(--accent-red);
  }

  .pip-btn--stop:hover:not(:disabled) {
    background: rgba(var(--accent-red-rgb), 0.3);
  }

  .pip-btn--stop-micro {
    padding: 2px 6px;
    background: rgba(var(--accent-red-rgb), 0.2);
    border: 1px solid rgba(var(--accent-red-rgb), 0.4);
    color: var(--accent-red);
  }

  .pip-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Terminal Scroll Container */
  .pip-panel__terminal {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: var(--space-2) var(--space-3);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    line-height: 1.45;
    background: var(--log-bg, #080c18);
    color: var(--text);
  }

  .pip-terminal__empty {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--muted);
    font-style: italic;
    padding: var(--space-4) 0;
  }

  .pip-terminal__cursor {
    color: var(--accent);
    font-weight: 700;
    animation: pipBlink 1s step-start infinite;
  }

  .pip-line {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    word-break: break-all;
    white-space: pre-wrap;
  }

  .pip-line__num {
    font-size: 0.6875rem;
    color: var(--muted);
    user-select: none;
    opacity: 0.4;
    min-width: 32px;
    text-align: right;
    flex-shrink: 0;
  }

  .pip-line__text {
    flex: 1;
  }

  .pip-line--error {
    color: #f87171;
    background: rgba(239, 68, 68, 0.08);
  }

  .pip-line--warn {
    color: #fbbf24;
    background: rgba(245, 158, 11, 0.05);
  }

  /* Animations */
  @keyframes pipSlideUp {
    from {
      opacity: 0;
      transform: translateY(16px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes pipDotPulse {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.5;
      transform: scale(0.85);
    }
  }

  @keyframes pipBlink {
    0%, 50% {
      opacity: 1;
    }
    51%, 100% {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pip-drawer-container,
    .pip-panel,
    .pip-status-dot--running,
    .pip-terminal__cursor {
      animation: none;
      transition: none;
      transform: none;
    }
  }
</style>
