<!--
  ============================================================================
  HorizonTile.svelte — Horizon Glass Instance Showcase Tile
  ============================================================================
  PS4/PS5 profile-selector inspired hero tile with deterministic loader art
  banner, subtle parallax depth on hover, status badges, running indicator,
  and dynamic ambient lighting glow.

  Props:
    - instance: InstanceSummary
    - selected?: boolean (default: false)
    - focused?: boolean (default: false)
    - running?: boolean (default: false)
    - onselect?: () => void
    - onlaunch?: () => void
    - onclick?: () => void
    - ondblclick?: () => void
-->
<script lang="ts">
  import type { InstanceSummary } from '../lib/types';
  import LoaderBadge from './LoaderBadge.svelte';
  import Badge from './Badge.svelte';
  import { memLabel } from '../lib/format';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    instance: InstanceSummary;
    selected?: boolean;
    focused?: boolean;
    running?: boolean;
    onselect?: () => void;
    onlaunch?: () => void;
    onclick?: () => void;
    ondblclick?: () => void;
    [key: string]: unknown;
  }

  let {
    instance,
    selected = false,
    focused = false,
    running = false,
    onselect,
    onlaunch,
    onclick,
    ondblclick,
    ...rest
  }: Props = $props();

  let tileEl: HTMLElement | null = $state(null);
  let mouseX = $state(0);
  let mouseY = $state(0);
  let isHovered = $state(false);

  const normalizedLoader = $derived(
    ((instance.loader || 'vanilla') as string).toLowerCase().trim(),
  );

  // Deterministic loader color palette & ambient hue
  const loaderMeta = $derived.by(() => {
    switch (normalizedLoader) {
      case 'fabric':
        return {
          hue: 190,
          accent: '#06b6d4',
          accentRgb: '6, 182, 212',
          gradient: 'linear-gradient(135deg, #042f2e 0%, #0891b2 45%, #06b6d4 80%, #38bdf8 100%)',
          mesh: 'radial-gradient(circle at 80% 20%, rgba(6, 182, 212, 0.6), transparent 70%)',
        };
      case 'neoforge':
      case 'forge':
        return {
          hue: 24,
          accent: '#f97316',
          accentRgb: '249, 115, 22',
          gradient: 'linear-gradient(135deg, #431407 0%, #ea580c 45%, #f97316 80%, #fb923c 100%)',
          mesh: 'radial-gradient(circle at 80% 20%, rgba(249, 115, 22, 0.6), transparent 70%)',
        };
      case 'quilt':
        return {
          hue: 280,
          accent: '#a855f7',
          accentRgb: '168, 85, 247',
          gradient: 'linear-gradient(135deg, #3b0764 0%, #9333ea 45%, #a855f7 80%, #c084fc 100%)',
          mesh: 'radial-gradient(circle at 80% 20%, rgba(168, 85, 247, 0.6), transparent 70%)',
        };
      default:
        return {
          hue: 160,
          accent: '#10b981',
          accentRgb: '16, 185, 129',
          gradient: 'linear-gradient(135deg, #022c22 0%, #059669 45%, #10b981 80%, #34d399 100%)',
          mesh: 'radial-gradient(circle at 80% 20%, rgba(16, 185, 129, 0.6), transparent 70%)',
        };
    }
  });

  const memoryDisplay = $derived(memLabel(instance.memory_mb || 3072));
  const monogramLetter = $derived(
    (instance.name || 'M').trim().charAt(0).toUpperCase(),
  );

  function handleMouseMove(e: MouseEvent): void {
    if (!tileEl) return;
    const rect = tileEl.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width - 0.5;
    const y = (e.clientY - rect.top) / rect.height - 0.5;
    mouseX = Math.max(-0.5, Math.min(0.5, x));
    mouseY = Math.max(-0.5, Math.min(0.5, y));
  }

  function handleMouseEnter(): void {
    isHovered = true;
  }

  function handleMouseLeave(): void {
    isHovered = false;
    mouseX = 0;
    mouseY = 0;
  }

  function handleClick(): void {
    if (onselect) onselect();
    if (onclick) onclick();
  }

  function handleDblClick(): void {
    if (ondblclick) {
      ondblclick();
    } else if (typeof window !== 'undefined') {
      window.location.hash = `#/instances/${encodeURIComponent(instance.name)}`;
    }
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      handleClick();
    }
  }

  const parallaxTransform = $derived(
    isHovered
      ? `perspective(1000px) rotateY(${mouseX * 10}deg) rotateX(${-mouseY * 10}deg) translateZ(8px)`
      : 'perspective(1000px) rotateY(0deg) rotateX(0deg) translateZ(0px)',
  );

  const bgParallax = $derived(
    isHovered ? `translate3d(${mouseX * -16}px, ${mouseY * -16}px, 0)` : 'translate3d(0, 0, 0)',
  );

  const monogramParallax = $derived(
    isHovered ? `translate3d(${mouseX * 24}px, ${mouseY * 24}px, 0)` : 'translate3d(0, 0, 0)',
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<article
  bind:this={tileEl}
  class="horizon-tile"
  class:horizon-tile--selected={selected}
  class:horizon-tile--focused={focused}
  class:horizon-tile--running={running}
  style="--tile-hue: {loaderMeta.hue}; --tile-accent: {loaderMeta.accent}; --tile-accent-rgb: {loaderMeta.accentRgb};"
  tabindex="0"
  role="button"
  aria-pressed={selected}
  aria-label="{instance.name} ({instance.version}, {instance.loader})"
  onclick={handleClick}
  ondblclick={handleDblClick}
  onkeydown={handleKeyDown}
  onmousemove={handleMouseMove}
  onmouseenter={handleMouseEnter}
  onmouseleave={handleMouseLeave}
  {...rest}
>
  <!-- Card Transformation Wrapper for 3D depth -->
  <div class="horizon-tile__inner" style="transform: {parallaxTransform};">
    <!-- Background Art Canvas -->
    <div class="horizon-tile__banner">
      <div class="horizon-tile__gradient" style="background: {loaderMeta.gradient};"></div>
      <div class="horizon-tile__mesh" style="background: {loaderMeta.mesh};"></div>
      
      <!-- Subtle Decorative Grid Lines -->
      <div class="horizon-tile__grid" style="transform: {bgParallax};"></div>

      <!-- Large Deterministic Monogram Watermark -->
      <div class="horizon-tile__watermark" style="transform: {monogramParallax};">
        <span class="horizon-tile__watermark-char">{monogramLetter}</span>
      </div>

      <!-- Huge Version Typography in Background -->
      <div class="horizon-tile__version-bg" style="transform: {bgParallax};">
        {instance.version}
      </div>

      <!-- Specular Top Highlight -->
      <div class="horizon-tile__sheen"></div>
    </div>

    <!-- Top Status Bar overlay -->
    <header class="horizon-tile__header">
      <div class="horizon-tile__tags">
        <LoaderBadge loader={instance.loader} version={instance.version} size="sm" />
        {#if instance.aot_cache_exists}
          <Badge variant="ok" size="sm" title={t('home.tileAotTrained')}>
            ⚡ AOT
          </Badge>
        {/if}
      </div>

      {#if running}
        <div class="horizon-tile__running-dot" title={t('home.tileRunning')}>
          <span class="horizon-tile__pulse-core"></span>
          <span class="horizon-tile__pulse-wave"></span>
        </div>
      {/if}
    </header>

    <!-- Center / Bottom Content Area -->
    <div class="horizon-tile__body">
      <div class="horizon-tile__meta-row">
        <span class="horizon-tile__chip">{memoryDisplay}</span>
        {#if instance.mod_count > 0}
          <span class="horizon-tile__chip">
            {t('home.tileModsActive', {
              enabled: instance.enabled_mod_count,
              total: instance.mod_count,
            })}
          </span>
        {/if}
      </div>

      <h3 class="horizon-tile__title" title={instance.name}>
        {instance.name}
      </h3>

      <p class="horizon-tile__hint">
        {t('home.tileDoubleClickHint')}
      </p>
    </div>

    <!-- Selection indicator bar -->
    <div class="horizon-tile__select-bar"></div>
  </div>
</article>

<style>
  .horizon-tile {
    position: relative;
    flex: 0 0 360px;
    width: 360px;
    height: 320px;
    border-radius: var(--radius-tile, 1rem);
    background: transparent;
    cursor: pointer;
    user-select: none;
    scroll-snap-align: start;
    outline: none;
    transition: transform var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      filter var(--dur-med, 260ms) ease;
  }

  :global([data-density='compact']) .horizon-tile {
    flex: 0 0 280px;
    width: 280px;
    height: 260px;
  }

  .horizon-tile__inner {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: var(--radius-tile, 1rem);
    background: var(--card-bg, rgba(13, 18, 34, 0.75));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    box-shadow: var(--shadow-md, 0 10px 30px rgba(0, 0, 0, 0.35));
    overflow: hidden;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    transition: border-color var(--dur-med, 260ms) ease,
      box-shadow var(--dur-med, 260ms) ease,
      filter var(--dur-fast, 120ms) ease;
    will-change: transform;
    transform-style: preserve-3d;
  }

  .horizon-tile:hover .horizon-tile__inner {
    border-color: rgba(var(--tile-accent-rgb, 16, 185, 129), 0.7);
    box-shadow: var(--shadow-lg, 0 16px 48px rgba(0, 0, 0, 0.5)), 0 0 28px rgba(var(--tile-accent-rgb, 16, 185, 129), 0.3);
  }
  .horizon-tile:focus-visible .horizon-tile__inner {
    box-shadow: 0 0 0 2px var(--bg, #060a14), 0 0 0 4px var(--tile-accent, #10b981);
  }

  .horizon-tile--selected .horizon-tile__inner {
    border-color: var(--tile-accent, #10b981);
    box-shadow: var(--shadow-lg, 0 20px 50px rgba(0, 0, 0, 0.55)), 0 0 36px rgba(var(--tile-accent-rgb, 16, 185, 129), 0.45);
  }

  .horizon-tile--selected {
    transform: translateY(-6px) scale(1.02);
  }

  .horizon-tile--running .horizon-tile__inner {
    border-color: #22c55e;
  }

  /* --- Art Banner Area --- */
  .horizon-tile__banner {
    position: absolute;
    inset: 0;
    overflow: hidden;
    border-radius: inherit;
    pointer-events: none;
    z-index: 1;
  }

  .horizon-tile__gradient {
    position: absolute;
    inset: 0;
    opacity: 0.85;
    mix-blend-mode: multiply;
  }

  .horizon-tile__mesh {
    position: absolute;
    inset: 0;
    opacity: 0.9;
  }

  .horizon-tile__grid {
    position: absolute;
    inset: -20px;
    background-image: linear-gradient(rgba(255, 255, 255, 0.04) 1px, transparent 1px),
      linear-gradient(90deg, rgba(255, 255, 255, 0.04) 1px, transparent 1px);
    background-size: 24px 24px;
    transition: transform 0.15s ease-out;
  }

  .horizon-tile__watermark {
    position: absolute;
    right: -8px;
    top: 12px;
    font-family: var(--font-mono-retro, 'Press Start 2P', monospace);
    font-size: 110px;
    font-weight: 900;
    line-height: 1;
    color: rgba(255, 255, 255, 0.16);
    pointer-events: none;
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
    text-shadow: 0 0 30px rgba(var(--tile-accent-rgb, 16, 185, 129), 0.25);
  }

  :global([data-density='compact']) .horizon-tile__watermark {
    font-size: 80px;
    top: 8px;
  }

  .horizon-tile__version-bg {
    position: absolute;
    left: 16px;
    bottom: 74px;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: 42px;
    font-weight: 800;
    letter-spacing: -0.04em;
    color: rgba(255, 255, 255, 0.18);
    pointer-events: none;
    white-space: nowrap;
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
    text-shadow: 0 0 24px rgba(var(--tile-accent-rgb, 16, 185, 129), 0.3);
  }

  :global([data-density='compact']) .horizon-tile__version-bg {
    font-size: 32px;
    bottom: 64px;
  }
  .horizon-tile__sheen {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      180deg,
      rgba(255, 255, 255, 0.15) 0%,
      rgba(255, 255, 255, 0.02) 40%,
      transparent 100%
    );
  }

  /* --- Overlay Header --- */
  .horizon-tile__header {
    position: relative;
    z-index: 2;
    padding: var(--space-3, 12px) var(--space-4, 16px);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .horizon-tile__tags {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    flex-wrap: wrap;
  }

  /* Running Indicator Pulse */
  .horizon-tile__running-dot {
    position: relative;
    width: 14px;
    height: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .horizon-tile__pulse-core {
    width: 8px;
    height: 8px;
    background: #22c55e;
    border-radius: 50%;
    box-shadow: 0 0 8px #22c55e;
  }

  .horizon-tile__pulse-wave {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: rgba(34, 197, 94, 0.6);
    animation: pulseGlow 1.8s ease-in-out infinite;
  }

  /* --- Overlay Body --- */
  .horizon-tile__body {
    position: relative;
    z-index: 2;
    padding: var(--space-4, 16px) var(--space-4, 16px) var(--space-3, 12px);
    background: linear-gradient(180deg, transparent 0%, rgba(var(--bg-rgb, 6, 10, 20), 0.82) 40%, rgba(var(--bg-rgb, 6, 10, 20), 0.96) 100%);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
  }

  .horizon-tile__meta-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .horizon-tile__chip {
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: #ffffff;
    background: rgba(0, 0, 0, 0.4);
    padding: 2px 6px;
    border-radius: var(--radius-xs, 0.25rem);
    border: 1px solid rgba(255, 255, 255, 0.15);
  }
  .horizon-tile__title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-xl, 1.375rem);
    font-weight: 700;
    line-height: var(--leading-tight, 1.2);
    color: #ffffff;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.6);
  }

  :global([data-density='compact']) .horizon-tile__title {
    font-size: var(--text-lg, 1.125rem);
  }

  .horizon-tile__hint {
    margin: 0;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: 0.7rem;
    font-weight: 500;
    color: var(--muted-strong, #b3c5e3);
    opacity: 1;
    transition: opacity var(--dur-fast, 120ms) ease, color var(--dur-fast, 120ms) ease;
  }

  .horizon-tile:hover .horizon-tile__hint {
    opacity: 1;
    color: var(--tile-accent, #10b981);
  }

  /* --- Bottom Select Accent Bar --- */
  .horizon-tile__select-bar {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: var(--tile-accent, #10b981);
    opacity: 0;
    transition: opacity var(--dur-fast, 120ms) ease, height var(--dur-fast, 120ms) ease;
    box-shadow: 0 0 12px var(--tile-accent, #10b981);
  }

  .horizon-tile--selected .horizon-tile__select-bar {
    opacity: 1;
    height: 4px;
  }
</style>
