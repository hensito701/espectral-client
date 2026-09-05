<!--
  ============================================================================
  TelemetryCapsule.svelte — Horizon Glass Launch Flight Deck
  ============================================================================
  Docked glass capsule offering full launch orchestration: account selector,
  Normal/AOT mode switch, tactile JUGAR primary action, dry-run inspector,
  live boot-phase telemetry readout, and active instance termination.

  Props (Pinned Contract):
    - instanceName: string | null
    - running?: boolean
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Account, LaunchMode, LaunchReply, LaunchExitEvent } from '../lib/types';
  import {
    getAccounts,
    setActiveAccount,
    launchInstance,
    stopInstance,
    avatarUrl,
  } from '../lib/api';
  import { launchLog, liveLaunches, instances as instancesStore } from '../lib/stores';
  import { subscribeEvents } from '../lib/sse';
  import { pushToast } from '../lib/toast.svelte';
  import Btn from './Btn.svelte';
  import MonogramTile from './MonogramTile.svelte';
  import Badge from './Badge.svelte';
  import { t } from '../lib/i18n.svelte';
  interface Props {
    instanceName: string | null;
    running?: boolean;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    instanceName,
    running: propRunning = false,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  let accounts = $state<Account[]>([]);
  let activeUsername = $state('');
  // Cache-buster (AccountVault pattern): avatar bytes can change under the
  // same URL, so bump on every account change to force a refetch.
  let avatarBust = $state(0);
  let accountMenuOpen = $state(false);
  let launchMode = $state<LaunchMode>('aot');
  let isLaunching = $state(false);
  let isStopping = $state(false);

  // Initial launch mode from localStorage
  if (typeof window !== 'undefined') {
    try {
      const storedMode = localStorage.getItem('horizon:launch-mode');
      if (storedMode === 'normal' || storedMode === 'aot') {
        launchMode = storedMode;
      }
    } catch {
      // storage unavailable
    }
  }

  function setMode(mode: LaunchMode): void {
    launchMode = mode;
    try {
      localStorage.setItem('horizon:launch-mode', mode);
    } catch {
      // storage unavailable
    }
  }

  // Derive active account object
  const activeAccount = $derived(
    accounts.find((a) => a.username === activeUsername) || accounts[0] || null,
  );

  // Derive running state for current instance
  const isCurrentlyRunning = $derived.by(() => {
    if (propRunning) return true;
    if (!instanceName) return false;
    return $liveLaunches.value.some((l) => l.instance === instanceName && l.running);
  });

  // Telemetry buffer inspection
  const activeBuffer = $derived.by(() => {
    if (!instanceName) return null;
    const buffers = Object.values($launchLog.buffers);
    // Find latest buffer matching this instance
    return buffers.filter((b) => b.instance === instanceName).pop() || null;
  });

  const latestTelemetryLine = $derived.by(() => {
    if (!activeBuffer || !activeBuffer.lines.length) return '';
    const last = activeBuffer.lines[activeBuffer.lines.length - 1];
    return last.length > 70 ? last.slice(0, 67) + '...' : last;
  });

  // C12: compact timing from launch-exit (graceful when fields missing — works
  // before engine-side B lands). Stored per-instance so switching instances
  // does not leak timing.
  let timingByInstance = $state<Record<string, { spawn_ms?: number | null; boot_ms?: number | null }>>({});

  const capsuleTiming = $derived.by(() => {
    if (!instanceName) return null;
    const rec = timingByInstance[instanceName];
    if (!rec) return null;
    const s = rec.spawn_ms;
    const b = rec.boot_ms;
    if (s == null && b == null) return null;
    const fmt = (v: number | null | undefined) => (typeof v === 'number' && Number.isFinite(v) ? (v / 1000).toFixed(1) : null);
    const prep = fmt(s);
    const jvm = fmt(b);
    // Render only when at least one is numeric; missing side shows em dash
    // so the template stays stable. If both missing we already returned null.
    const p = prep ?? '—';
    const j = jvm ?? '—';
    // If both were missing we would have returned; if one is dash we still
    // show the combined line gracefully.
    // When engine fields are absent (pre-B), we hide the line.
    if (prep === null && jvm === null) return null;
    return t('home.capsuleTiming', { prep: p, jvm: j });
  });
  async function loadAccountsList(): Promise<void> {
    try {
      const list = await getAccounts();
      accounts = list;
      
      // Check last chosen account in localStorage
      let preferred = '';
      try {
        preferred = localStorage.getItem('horizon:last-account') || '';
      } catch {
        // storage unavailable
      }

      if (preferred && list.some((a) => a.username === preferred)) {
        activeUsername = preferred;
      } else {
        const sorted = [...list].sort(
          (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
        );
        if (sorted.length > 0) {
          activeUsername = sorted[0].username;
        }
      }
    } catch (e) {
      console.error('Failed to fetch accounts:', e);
    }
  }

  async function selectAccount(username: string): Promise<void> {
    try {
      await setActiveAccount(username);
      activeUsername = username;
      try {
        localStorage.setItem('horizon:last-account', username);
      } catch {
        // storage unavailable
      }
      accountMenuOpen = false;
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
    } catch (e) {
      console.error('Failed to set active account:', e);
    }
  }

  async function handleLaunch(): Promise<void> {
    if (!instanceName || isLaunching || isCurrentlyRunning) return;
    isLaunching = true;

    try {
      const res = await launchInstance(instanceName, {
        mode: launchMode,
        dry_run: false,
        account: activeUsername || undefined,
      });

      const reply = res as LaunchReply;
      if (reply?.key) {
        $launchLog.start(reply.key, { instance: instanceName, account: activeUsername });
      }

      void $liveLaunches.refresh();
      void $instancesStore.refresh();

      pushToast({
        kind: 'ok',
        text: `Lanzando ${instanceName}...`,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({
        kind: 'err',
        text: msg,
      });
    } finally {
      isLaunching = false;
    }
  }

  async function handleStop(): Promise<void> {
    if (!instanceName || isStopping) return;
    isStopping = true;

    try {
      await stopInstance(instanceName);
      void $liveLaunches.refresh();
      void $instancesStore.refresh();
      pushToast({
        kind: 'info',
        text: `Instancia ${instanceName} detenida.`,
      });
    } catch (e) {
      pushToast({
        kind: 'err',
        text: e instanceof Error ? e.message : String(e),
      });
    } finally {
      isStopping = false;
    }
  }

  function handleDryRun(): void {
    if (!instanceName) return;
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('horizon:dryrun', { detail: { name: instanceName } }),
      );
    }
  }

  function openInstanceDetail(): void {
    if (instanceName && typeof window !== 'undefined') {
      window.location.hash = `#/instances/${encodeURIComponent(instanceName)}`;
    }
  }

  onMount(() => {
    void loadAccountsList();

    const onAccountChanged = (e: Event) => {
      const detail = (e as CustomEvent<{ username: string }>).detail;
      if (detail?.username) {
        activeUsername = detail.username;
      }
      avatarBust += 1;
      void loadAccountsList();
    };

    window.addEventListener('horizon:account-changed', onAccountChanged);

    // C12: capture timing from launch-exit for this capsule's instance.
    // Graceful if engine hasn't yet shipped spawn_ms/boot_ms (pre-B).
    const unsubExit = subscribeEvents((ev) => {
      if (ev.type !== 'launch-exit') return;
      const d = ev.data as LaunchExitEvent | null;
      if (!d || typeof d.key !== 'string') return;
      const inst = d.instance ?? '';
      if (!inst) return;
      // Graceful: hide when engine hasn't yet sent timing (pre-B) — both
      // spawn_ms and boot_ms absent/null means nothing to show.
      if (d.spawn_ms == null && d.boot_ms == null) return;
      timingByInstance[inst] = { spawn_ms: d.spawn_ms ?? null, boot_ms: d.boot_ms ?? null };
      // Trigger reactivity (assign new object ref)
      timingByInstance = { ...timingByInstance };
    });

    return () => {
      window.removeEventListener('horizon:account-changed', onAccountChanged);
      unsubExit();
    };
  });
</script>

<div
  class="telemetry-capsule-dock"
  class:telemetry-capsule-dock--running={isCurrentlyRunning}
  {...rest}
>
  <div class="telemetry-capsule">
    <!-- Left Section: Account Picker -->
    <div class="capsule-section capsule-account">
      <button
        type="button"
        class="capsule-account__btn"
        onclick={() => (accountMenuOpen = !accountMenuOpen)}
        title={t('home.capsuleAccount')}
        aria-haspopup="listbox"
        aria-expanded={accountMenuOpen}
      >
        <MonogramTile
          name={activeAccount?.username || activeUsername || 'Steve'}
          hue={activeAccount?.avatar_color ?? undefined}
          avatarUrl={activeAccount?.has_avatar ? `${avatarUrl(activeAccount.username)}${avatarBust ? `?v=${avatarBust}` : ''}` : undefined}
          size={32}
          shape="rounded"
        />
        <div class="capsule-account__info">
          <span
            class="capsule-account__name"
            title={activeUsername || t('home.noActiveAccount')}
          >
            {activeUsername || t('home.noActiveAccount')}
          </span>
          <span class="capsule-account__tag">
            {activeAccount?.token_kind === 'msa' ? 'Microsoft' : 'Offline'}
          </span>
        </div>
        <svg class="capsule-chevron" viewBox="0 0 20 20" fill="currentColor">
          <path fill-rule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clip-rule="evenodd" />
        </svg>
      </button>

      <!-- Account Popover Menu -->
      {#if accountMenuOpen}
        <div class="capsule-account-dropdown" role="listbox">
          <div class="capsule-dropdown-header">
            {t('home.capsuleSelectAccount')}
          </div>
          {#each accounts as acc (acc.username)}
            <button
              type="button"
              class="capsule-dropdown-item"
              class:capsule-dropdown-item--active={acc.username === activeUsername}
              onclick={() => selectAccount(acc.username)}
              role="option"
              aria-selected={acc.username === activeUsername}
            >
              <MonogramTile
                name={acc.username}
                hue={acc.avatar_color ?? undefined}
                avatarUrl={acc.has_avatar ? `${avatarUrl(acc.username)}${avatarBust ? `?v=${avatarBust}` : ''}` : undefined}
                size={24}
                shape="rounded"
              />
              <span class="capsule-dropdown-name">{acc.username}</span>
              {#if acc.token_kind === 'msa'}
                <Badge variant="accent" size="sm">MSA</Badge>
              {/if}
            </button>
          {/each}
          <div class="capsule-dropdown-footer">
            <a href="#/account" class="capsule-dropdown-link" onclick={() => (accountMenuOpen = false)}>
              + Administrar Cuentas
            </a>
          </div>
        </div>
      {/if}
    </div>

    <div class="capsule-divider"></div>

    <!-- Center Left Section: Launch Mode & Instance Telemetry -->
    <div class="capsule-section capsule-telemetry">
      <div class="capsule-telemetry__header">
        {#if instanceName}
          <button
            type="button"
            class="capsule-instance-link"
            onclick={openInstanceDetail}
            title={t('home.capsuleManageInstance')}
          >
            <span class="capsule-instance-name">{instanceName}</span>
            <svg class="capsule-link-icon" viewBox="0 0 20 20" fill="currentColor">
              <path d="M10 3a1.5 1.5 0 110 3 1.5 1.5 0 010-3zM10 8.5a1.5 1.5 0 110 3 1.5 1.5 0 010-3zM11.5 15.5a1.5 1.5 0 10-3 0 1.5 1.5 0 003 0z" />
            </svg>
          </button>
        {:else}
          <span class="capsule-instance-empty">{t('home.capsuleSelectInstance')}</span>
        {/if}

        <!-- Mode Segmented Control -->
        <div class="capsule-mode-toggle" role="group" aria-label={t('home.capsuleMode')}>
          <button
            type="button"
            class="capsule-mode-btn"
            class:capsule-mode-btn--active={launchMode === 'normal'}
            onclick={() => setMode('normal')}
            title="Lanzamiento estándar JVM"
          >
            Normal
          </button>
          <button
            type="button"
            class="capsule-mode-btn capsule-mode-btn--aot"
            class:capsule-mode-btn--active={launchMode === 'aot'}
            onclick={() => setMode('aot')}
            title="AOT AppCDS FastBoot optimizado"
          >
            ⚡ AOT
          </button>
        </div>
      </div>

      <!-- Live Log / Phase Readout -->
      <div class="capsule-phase-line">
        {#if isCurrentlyRunning}
          <span class="capsule-phase-status capsule-phase-status--running">
            <span class="capsule-live-dot"></span>
            {t('home.capsuleStatusRunning')}
          </span>
          {#if latestTelemetryLine}
            <span class="capsule-phase-tail">{latestTelemetryLine}</span>
          {/if}
        {:else if isLaunching}
          <span class="capsule-phase-status capsule-phase-status--starting">
            {t('home.capsuleStatusStarting')}
          </span>
        {:else}
          <span class="capsule-phase-status">
            {t('home.capsuleStatusIdle')}
          </span>
        {/if}
      </div>
      {#if capsuleTiming}
        <div class="capsule-timing-line">{capsuleTiming}</div>
      {/if}
    </div>

    <!-- Right Section: Action Controls -->
    <div class="capsule-section capsule-actions">
      <!-- Dry Run Button -->
      <button
        type="button"
        class="capsule-dryrun-btn"
        onclick={handleDryRun}
        disabled={!instanceName}
        title="Verificar classpath y comando JVM sin lanzar"
      >
        {t('home.capsuleDryRun')}
      </button>

      <!-- Main Action: Play / Stop Button -->
      {#if isCurrentlyRunning}
        <Btn
          variant="danger"
          size="lg"
          loading={isStopping}
          onclick={handleStop}
          class="capsule-play-btn capsule-stop-btn"
        >
          ■ {t('home.capsuleStop')}
        </Btn>
      {:else}
        <Btn
          variant="primary"
          size="lg"
          loading={isLaunching}
          disabled={!instanceName}
          onclick={handleLaunch}
          class="capsule-play-btn u-glow-play"
        >
          ▶ {isLaunching ? t('home.capsuleLaunching') : t('home.capsuleLaunch')}
        </Btn>
      {/if}
    </div>
  </div>
</div>

<style>
  .telemetry-capsule-dock {
    width: 100%;
    display: flex;
    justify-content: center;
    padding: 0 var(--space-4, 16px) var(--space-4, 16px);
    z-index: 25;
  }

  .telemetry-capsule {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-4, 16px);
    padding: var(--space-2, 8px) var(--space-4, 16px);
    background: var(--chrome-bg, rgba(10, 15, 30, 0.85));
    backdrop-filter: blur(24px);
    -webkit-backdrop-filter: blur(24px);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    box-shadow: var(--shadow-lg, 0 16px 40px rgba(0, 0, 0, 0.6)), var(--glass-glow, 0 0 24px rgba(16, 185, 129, 0.15));
    max-width: 960px;
    width: 100%;
    transition: all var(--dur-med, 260ms) ease;
  }

  [data-theme='light'] .telemetry-capsule {
    background: var(--surface, rgba(255, 255, 255, 0.9));
    border-color: var(--border, rgba(190, 204, 226, 0.65));
    box-shadow: var(--shadow-lg, 0 16px 40px rgba(18, 24, 41, 0.12)), 0 0 20px rgba(5, 150, 105, 0.1);
  }

  .telemetry-capsule-dock--running .telemetry-capsule {
    border-color: rgba(var(--accent-green-rgb, 34, 197, 94), 0.5);
    box-shadow: var(--shadow-lg, 0 16px 40px rgba(0, 0, 0, 0.6)), 0 0 28px rgba(var(--accent-green-rgb, 34, 197, 94), 0.25);
  }

  .capsule-section {
    display: flex;
    align-items: center;
  }

  .capsule-divider {
    width: 1px;
    height: 36px;
    background: var(--border, rgba(255, 255, 255, 0.1));
    flex-shrink: 0;
  }

  /* --- Account Picker --- */
  .capsule-account {
    position: relative;
    flex-shrink: 0;
    min-width: fit-content;
  }

  .capsule-account__btn {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    background: transparent;
    border: none;
    border-radius: var(--radius-pill, 9999px);
    padding: 4px 8px;
    color: var(--text, #e8ecf4);
    cursor: pointer;
    transition: background var(--dur-fast, 120ms) ease;
    outline: none;
  }

  .capsule-account__btn:hover {
    background: var(--surface-up, rgba(255, 255, 255, 0.06));
  }

  .capsule-account__btn:focus-visible {
    box-shadow: 0 0 0 2px var(--accent, #10b981);
  }

  .capsule-account__info {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    text-align: left;
  }

  .capsule-account__name {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    line-height: 1.1;
    color: var(--text, #e8ecf4);
    min-width: 11ch;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .capsule-account__tag {
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--muted-strong, #8e9eb8);
  }

  .capsule-chevron {
    width: 16px;
    height: 16px;
    color: var(--muted-strong, #8e9eb8);
  }

  /* Account Popover */
  .capsule-account-dropdown {
    position: absolute;
    bottom: calc(100% + 12px);
    left: 0;
    width: 240px;
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-lg, 0.875rem);
    box-shadow: var(--shadow-lg, 0 16px 36px rgba(0, 0, 0, 0.6));
    padding: 6px;
    z-index: 50;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .capsule-dropdown-header {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--muted-strong, #8e9eb8);
    padding: 6px 8px 4px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .capsule-dropdown-item {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: 6px 8px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm, 0.375rem);
    color: var(--text, #e8ecf4);
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    cursor: pointer;
    width: 100%;
    text-align: left;
    transition: background var(--dur-fast, 120ms) ease;
  }

  .capsule-dropdown-item:hover {
    background: var(--surface-up, rgba(255, 255, 255, 0.06));
  }

  .capsule-dropdown-item--active {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.15);
    color: var(--accent, #10b981);
    font-weight: 600;
  }

  .capsule-dropdown-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .capsule-dropdown-footer {
    border-top: 1px solid var(--border, rgba(255, 255, 255, 0.06));
    padding-top: 4px;
    margin-top: 2px;
  }

  .capsule-dropdown-link {
    display: block;
    padding: 6px 8px;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--accent, #10b981);
    text-decoration: none;
    border-radius: var(--radius-sm, 0.375rem);
  }

  .capsule-dropdown-link:hover {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.1);
  }
  /* --- Telemetry Center --- */
  .capsule-telemetry {
    flex: 1;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 2px;
    min-width: 0;
  }

  .capsule-telemetry__header {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    width: 100%;
  }

  .capsule-instance-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: transparent;
    border: none;
    padding: 0;
    color: var(--text, #e8ecf4);
    cursor: pointer;
    max-width: 220px;
    outline: none;
  }

  .capsule-instance-link:hover .capsule-instance-name {
    color: var(--accent, #10b981);
  }

  .capsule-instance-name {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .capsule-link-icon {
    width: 14px;
    height: 14px;
    color: var(--muted-strong, #8e9eb8);
  }

  .capsule-instance-empty {
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #8e9eb8);
    font-style: italic;
  }

  /* Mode Segmented */
  .capsule-mode-toggle {
    display: inline-flex;
    padding: 2px;
    background: var(--surface-up, rgba(255, 255, 255, 0.05));
    border-radius: var(--radius-pill, 9999px);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
  }

  .capsule-mode-btn {
    padding: 2px 8px;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: 0.6875rem;
    font-weight: 600;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-pill, 9999px);
    color: var(--muted-strong, #8e9eb8);
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .capsule-mode-btn--active {
    background: var(--surface-solid, #161e36);
    border-color: var(--border, transparent);
    color: var(--text, #ffffff);
    box-shadow: var(--shadow-sm, 0 1px 4px rgba(0, 0, 0, 0.3));
  }

  .capsule-mode-btn--aot.capsule-mode-btn--active {
    color: var(--accent-gold, #ffd700);
  }

  /* Telemetry Status Line */
  .capsule-phase-line {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted-strong, #8e9eb8);
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .capsule-phase-status--running {
    color: var(--accent-green, #22c55e);
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .capsule-live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-green, #22c55e);
    box-shadow: 0 0 6px var(--accent-green, #22c55e);
    animation: pulseGlow 1.5s infinite;
  }

  .capsule-phase-status--starting {
    color: var(--accent-alt, #f59e0b);
    font-weight: 600;
  }

  .capsule-phase-tail {
    color: var(--muted-strong, #b3c5e3);
    font-family: var(--font-mono, monospace);
    font-size: 0.7rem;
    opacity: 1;
  }

  .capsule-timing-line {
    font-size: var(--text-xs, 0.7rem);
    color: var(--muted-strong, #8e9eb8);
    font-family: var(--font-mono, monospace);
    opacity: 0.9;
    white-space: nowrap;
  }
  /* --- Actions Section --- */
  .capsule-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    flex-shrink: 0;
  }

  .capsule-dryrun-btn {
    padding: 0 var(--space-3, 12px);
    height: 40px;
    background: var(--surface-up, rgba(255, 255, 255, 0.05));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    color: var(--text, #b3c5e3);
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .capsule-dryrun-btn:hover:not(:disabled) {
    background: var(--surface-solid, rgba(255, 255, 255, 0.1));
    color: var(--text, #e8ecf4);
    border-color: var(--accent, rgba(255, 255, 255, 0.2));
  }

  .capsule-dryrun-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  :global(.capsule-play-btn) {
    min-width: 140px;
    height: 44px !important;
    border-radius: var(--radius-pill, 9999px) !important;
    font-size: var(--text-md, 0.9375rem) !important;
    font-weight: 700 !important;
    letter-spacing: 0.03em;
  }

  :global(.capsule-stop-btn) {
    background: var(--accent-red, #ef4444) !important;
  }
</style>
