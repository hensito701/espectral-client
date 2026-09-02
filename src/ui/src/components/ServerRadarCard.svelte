<!--
  ============================================================================
  ServerRadarCard.svelte — Horizon Glass Multiplayer Server Beacon Card
  ============================================================================
  Interactive multiplayer server beacon card featuring live player count,
  MOTD typography with Minecraft color-code parsing, software & version tags,
  animated SVG PingWaveform sparkline, and tactile one-click host copying
  with advancement-style toast feedback.

  Exported API (safe for consumption by HomeScreen, ServerRadar, etc.):
  Props:
    - server: ServerStatus — authoritative server state from $servers store
    - compact?: boolean (default: false) — condensed layout for horizontal strips
    - interactive?: boolean (default: true) — enables hover sheen and click-to-copy
    - featured?: boolean (default: false) — emerald highlight border
    - showIp?: boolean (default: true) — show direct numeric IP badge
    - class?: string (or className)
    - onclick?: (event: MouseEvent) => void — optional override for card click

  Events / Actions:
    - Card click copies server.host to clipboard (unless onclick prop provided)
    - Triggers pushToast feedback on successful copy
  ============================================================================
-->
<script lang="ts">
  import type { ServerStatus } from '../lib/types';
  import GlassCard from './GlassCard.svelte';
  import Badge from './Badge.svelte';
  import MonogramTile from './MonogramTile.svelte';
  import { renderMotdLines } from '../lib/motd';
  import { useCopy } from '../lib/useCopy.svelte';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    server: ServerStatus;
    compact?: boolean;
    interactive?: boolean;
    featured?: boolean;
    showIp?: boolean;
    class?: string;
    className?: string;
    onclick?: (e: MouseEvent) => void;
    [key: string]: unknown;
  }

  let {
    server,
    compact = false,
    interactive = true,
    featured = false,
    showIp = true,
    class: extraClass = '',
    className = '',
    onclick,
    ...rest
  }: Props = $props();

  const copier = useCopy(1800);

  const motdHtml = $derived.by(() => {
    if (!server) return '';
    if (server.motd_raw && server.motd_raw.length > 0) {
      return renderMotdLines(server.motd_raw);
    }
    if (server.motd_clean && server.motd_clean.length > 0) {
      return server.motd_clean.map((l) => `<span class="motd-clean-line">${l}</span>`).join('<br>');
    }
    return `<span class="motd-fallback">${server.hostname || server.host}</span>`;
  });

  const playerCounter = $derived.by(() => {
    if (!server || !server.online) return '0/0';
    return `${server.players_online ?? 0}/${server.players_max ?? 0}`;
  });

  const serverAddress = $derived.by(() => {
    if (!server) return '';
    return server.host || server.hostname || '';
  });

  const directAddress = $derived.by(() => {
    if (!server || !server.ip) return null;
    return `${server.ip}:${server.port ?? 25565}`;
  });

  function handleCardClick(e: MouseEvent) {
    if (onclick) {
      onclick(e);
      return;
    }
    if (!serverAddress) return;
    copier.copy(serverAddress);
  }

  function handleCopyDirect(e: MouseEvent) {
    e.stopPropagation();
    if (!directAddress) return;
    copier.copy(directAddress);
  }

  const combinedClass = $derived([
    'server-radar-card',
    compact ? 'server-radar-card--compact' : '',
    server?.online ? 'server-radar-card--online' : 'server-radar-card--offline',
    copier.copied ? 'server-radar-card--copied' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div
  class={combinedClass}
  role="button"
  tabindex="0"
  onclick={handleCardClick}
  onkeydown={(e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      handleCardClick(e as unknown as MouseEvent);
    }
  }}
  aria-label={`${server?.hostname || server?.host} - ${server?.online ? `${server.players_online} players online` : 'Offline'}`}
  {...rest}
>
  <GlassCard
    elevation="md"
    interactive={interactive}
    featured={featured || (server?.online && (server?.players_online ?? 0) > 0)}
    class="radar-glass-card"
  >
    <div class="radar-card-layout" class:radar-card-layout--compact={compact}>
      <!-- Top / Left Beacon Head -->
      <div class="beacon-header">
        <div class="beacon-avatar-wrap">
          {#if server?.icon}
            <img
              src={server.icon}
              alt={server.hostname || server.host}
              class="server-icon-img"
              loading="lazy"
            />
          {:else}
            <MonogramTile
              name={server?.hostname || server?.host || 'ES'}
              size={compact ? 40 : 48}
              shape="rounded"
            />
          {/if}

          <!-- Live Beacon Pulse Orb -->
          <span
            class="beacon-orb"
            class:beacon-orb--online={server?.online}
            class:beacon-orb--offline={!server?.online}
            title={server?.online ? t('server.online') : t('server.offline')}
          ></span>
        </div>

        <div class="server-identity">
          <div class="server-title-row">
            <h3 class="server-name">{server?.hostname || server?.host}</h3>
            {#if server?.online}
              <Badge variant="ok" size="sm" dot={true}>
                {t('common.online')}
              </Badge>
            {:else}
              <Badge variant="err" size="sm">
                {t('server.offline')}
              </Badge>
            {/if}
          </div>

          <div class="server-meta-tags">
            {#if server?.software}
              <span class="meta-chip meta-chip--software font-pixel">
                {server.software}
              </span>
            {/if}
            {#if server?.version}
              <span class="meta-chip meta-chip--version">
                {server.version}
              </span>
            {/if}
          </div>
        </div>

        <!-- Players counter -->
        <div class="telemetry-hud">
          <div class="players-counter-block">
            <span class="players-label">{t('server.players')}</span>
            <div class="players-badge">
              <Badge
                variant={server?.online && (server?.players_online ?? 0) > 0 ? 'accent' : 'neutral'}
                size={compact ? 'sm' : 'md'}
                pixel={true}
              >
                {playerCounter}
              </Badge>
            </div>
          </div>
        </div>
      </div>

      <!-- MOTD Stage (Hidden in compact view) -->
      {#if !compact}
        <div class="motd-stage">
          <div class="motd-content">
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            {@html motdHtml}
          </div>
        </div>
      {/if}

      <!-- Bottom Connect Action Bar -->
      <div class="radar-card-footer">
        <div class="address-preview">
          <span class="host-text">{serverAddress}</span>
          {#if showIp && directAddress}
            <button
              type="button"
              class="direct-ip-btn"
              title={t('server.copyIp')}
              onclick={handleCopyDirect}
            >
              IP: {directAddress}
            </button>
          {/if}
        </div>

        <div class="action-dock">
          <div class="copy-hint" class:copy-hint--copied={copier.copied}>
            {#if copier.copied}
              <span class="copy-hint-text font-pixel">✓ {t('server.copied')}</span>
            {:else}
              <span class="copy-hint-text font-pixel">CLICK ↵ {t('server.copy')}</span>
            {/if}
          </div>
        </div>
      </div>
    </div>
  </GlassCard>
</div>

<style>
  .server-radar-card {
    display: block;
    width: 100%;
    cursor: pointer;
    text-align: left;
    outline: none;
    transition: transform var(--dur-fast, 120ms) var(--ease-out-expo, ease-out);
  }

  .server-radar-card:focus-visible :global(.glass-card) {
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  .radar-card-layout {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 0.75rem);
    padding: var(--space-4, 1rem);
  }

  .radar-card-layout--compact {
    padding: var(--space-3, 0.75rem);
    gap: var(--space-2, 0.5rem);
  }

  /* Beacon Head */
  .beacon-header {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    width: 100%;
  }

  .beacon-avatar-wrap {
    position: relative;
    flex-shrink: 0;
    width: 48px;
    height: 48px;
  }

  .radar-card-layout--compact .beacon-avatar-wrap {
    width: 40px;
    height: 40px;
  }

  .server-icon-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: var(--radius-md, 0.625rem);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
    image-rendering: pixelated;
  }

  .beacon-orb {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--surface-solid, #0d1222);
  }

  .beacon-orb--online {
    background: var(--accent-green, #22c55e);
    box-shadow: 0 0 10px var(--accent-green, #22c55e);
    animation: orbPulse 2.6s infinite;
  }

  .beacon-orb--offline {
    background: var(--accent-red, #ef4444);
    box-shadow: 0 0 6px rgba(239, 68, 68, 0.6);
  }

  @keyframes orbPulse {
    0%, 100% {
      transform: scale(1);
      box-shadow: 0 0 8px var(--accent-green, #22c55e);
    }
    50% {
      transform: scale(1.18);
      box-shadow: 0 0 16px var(--accent-green, #22c55e);
    }
  }

  .server-identity {
    flex: 1 1 0%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 4px;
  }

  .server-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-width: 0;
  }

  .server-name {
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: -0.01em;
    max-width: 100%;
    line-height: 1.2;
  }

  .radar-card-layout--compact .server-name {
    font-size: var(--text-sm, 0.875rem);
  }

  .server-meta-tags {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    min-width: 0;
  }

  .meta-chip {
    font-size: 0.6875rem;
    padding: 1px 6px;
    border-radius: var(--radius-xs, 0.25rem);
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    color: var(--muted, #8e9eb8);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    line-height: 1.3;
  }

  .meta-chip--software {
    color: var(--accent-cyan, #06b6d4);
    border-color: rgba(6, 182, 212, 0.3);
    font-size: 0.625rem;
  }

  .telemetry-hud {
    display: flex;
    align-items: center;
    gap: var(--space-3, 0.75rem);
    flex-shrink: 0;
  }

  .players-counter-block {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }

  .players-label {
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* MOTD Stage */
  .motd-stage {
    padding: var(--space-2, 0.5rem) var(--space-3, 0.75rem);
    background: var(--surface-solid, #0d1222);
    border-radius: var(--radius-sm, 0.375rem);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    min-height: 48px;
    display: flex;
    align-items: center;
  }

  .motd-content {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 0.75rem);
    line-height: 1.4;
    color: var(--text, #e8ecf4);
    word-break: break-word;
    user-select: text;
  }

  /* Card Footer */
  .radar-card-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-top: var(--space-2, 0.5rem);
    border-top: 1px solid var(--border, rgba(40, 58, 96, 0.3));
    font-size: var(--text-xs, 0.75rem);
  }

  .address-preview {
    display: flex;
    align-items: center;
    gap: var(--space-2, 0.5rem);
    min-width: 0;
  }

  .host-text {
    font-family: var(--font-mono, monospace);
    font-weight: 600;
    color: var(--muted-strong, #b3c5e3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .direct-ip-btn {
    all: unset;
    font-family: var(--font-mono, monospace);
    font-size: 0.6875rem;
    color: var(--muted, #8e9eb8);
    background: var(--surface, rgba(16, 22, 42, 0.65));
    padding: 2px 6px;
    border-radius: var(--radius-xs, 0.25rem);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.4));
    cursor: pointer;
    transition: color var(--dur-fast, 120ms), border-color var(--dur-fast, 120ms);
  }

  .direct-ip-btn:hover {
    color: var(--accent-alt, #f59e0b);
    border-color: var(--accent-alt, #f59e0b);
  }

  .copy-hint {
    padding: 3px 8px;
    border-radius: var(--radius-xs, 0.25rem);
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    transition: all var(--dur-fast, 120ms);
  }

  .copy-hint-text {
    font-size: 0.625rem;
    color: var(--accent, #10b981);
    letter-spacing: 0.02em;
  }

  .copy-hint--copied {
    background: rgba(16, 185, 129, 0.2);
    border-color: var(--accent, #10b981);
  }

  @media (prefers-reduced-motion: reduce) {
    .beacon-orb--online {
      animation: none;
    }
  }
</style>
