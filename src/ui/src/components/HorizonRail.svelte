<!--
  ============================================================================
  HorizonRail.svelte — Horizon Glass Instance Carousel Rail
  ============================================================================
  Full-bleed horizontal instance tile rail (PS4/PS5 profile selector vibe)
  featuring snap-x scrolling, wheel-to-horizontal translation, drag scrolling,
  keyboard navigation, edge fade masks, and dynamic ambient lighting sync.

  Props:
    - instances?: InstanceSummary[] (defaults to $instances.value)
    - selectedName?: string
    - onselect?: (name: string) => void
    - onopenwizard?: () => void
    - runningNames?: string[]
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { InstanceSummary } from '../lib/types';
  import { instances as instancesStore, liveLaunches } from '../lib/stores';
  import HorizonTile from './HorizonTile.svelte';
  import Btn from './Btn.svelte';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    instances?: InstanceSummary[];
    selectedName?: string;
    onselect?: (name: string) => void;
    onopenwizard?: () => void;
    runningNames?: string[];
    [key: string]: unknown;
  }

  let {
    instances: propInstances,
    selectedName = $bindable(''),
    onselect,
    onopenwizard,
    runningNames = [],
  }: Props = $props();

  let railEl: HTMLElement | null = $state(null);
  let isDragging = $state(false);
  let startX = $state(0);
  let scrollLeft = $state(0);
  let hasDragged = $state(false);
  let canScrollLeft = $state(false);
  let canScrollRight = $state(false);
  let isOverflowing = $state(false);
  const instanceList = $derived(propInstances ?? $instancesStore.value);
  const isLoading = $derived($instancesStore.loading && !instanceList.length);

  // Derive running set
  const runningSet = $derived.by(() => {
    const set = new Set<string>(runningNames);
    for (const l of $liveLaunches.value) {
      if (l.running) set.add(l.instance);
    }
    return set;
  });

  // Ensure an instance is selected by default if available
  $effect(() => {
    if (!selectedName && instanceList.length > 0) {
      selectedName = instanceList[0].name;
      if (onselect) onselect(selectedName);
    }
  });

  // Update dynamic ambient lighting on documentElement based on selected tile
  $effect(() => {
    const current = instanceList.find((i) => i.name === selectedName);
    if (typeof document !== 'undefined') {
      const loader = (current?.loader || 'vanilla').toLowerCase();
      let hue = '160';
      if (loader === 'fabric') hue = '190';
      else if (loader === 'neoforge' || loader === 'forge') hue = '24';
      document.documentElement.style.setProperty('--ambient-hue', hue);
    }
  });

  function updateScrollState(): void {
    if (!railEl) return;
    isOverflowing = railEl.scrollWidth > railEl.clientWidth + 8;
    canScrollLeft = railEl.scrollLeft > 10;
    canScrollRight = railEl.scrollLeft < railEl.scrollWidth - railEl.clientWidth - 10;
  }

  function handleWheel(e: WheelEvent): void {
    if (!railEl) return;
    // Intercept vertical scroll and map to horizontal carousel
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
      e.preventDefault();
      railEl.scrollBy({ left: e.deltaY * 1.5, behavior: 'auto' });
      updateScrollState();
    }
  }

  function handleMouseDown(e: MouseEvent): void {
    if (!railEl) return;
    isDragging = true;
    hasDragged = false;
    startX = e.pageX - railEl.offsetLeft;
    scrollLeft = railEl.scrollLeft;
  }

  function handleMouseMove(e: MouseEvent): void {
    if (!isDragging || !railEl) return;
    const x = e.pageX - railEl.offsetLeft;
    const walk = (x - startX) * 1.5;
    if (Math.abs(walk) > 5) hasDragged = true;
    railEl.scrollLeft = scrollLeft - walk;
    updateScrollState();
  }

  function handleMouseUp(): void {
    isDragging = false;
  }

  function handleSelect(name: string): void {
    if (hasDragged) return;
    selectedName = name;
    if (onselect) onselect(name);
    scrollToInstance(name);
  }

  function scrollToInstance(name: string): void {
    if (!railEl) return;
    const idx = instanceList.findIndex((i) => i.name === name);
    if (idx === -1) return;
    const tiles = railEl.querySelectorAll('.horizon-tile');
    const target = tiles[idx] as HTMLElement | undefined;
    if (target) {
      target.scrollIntoView({ behavior: 'smooth', inline: 'center', block: 'nearest' });
    }
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (instanceList.length === 0) return;
    const currentIndex = instanceList.findIndex((i) => i.name === selectedName);

    if (e.key === 'ArrowRight') {
      e.preventDefault();
      const next = Math.min(instanceList.length - 1, (currentIndex === -1 ? 0 : currentIndex) + 1);
      handleSelect(instanceList[next].name);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      const prev = Math.max(0, (currentIndex === -1 ? 0 : currentIndex) - 1);
      handleSelect(instanceList[prev].name);
    } else if (e.key === 'Home') {
      e.preventDefault();
      handleSelect(instanceList[0].name);
    } else if (e.key === 'End') {
      e.preventDefault();
      handleSelect(instanceList[instanceList.length - 1].name);
    }
  }

  function triggerWizard(): void {
    if (onopenwizard) {
      onopenwizard();
    } else if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent('horizon:open-wizard'));
    }
  }

  onMount(() => {
    updateScrollState();
    window.addEventListener('resize', updateScrollState);
    return () => window.removeEventListener('resize', updateScrollState);
  });
</script>

<section
  class="horizon-rail-wrapper"
  aria-label={t('home.instancesRailTitle')}
  onkeydown={handleKeyDown}
>

  <!-- Horizontal Scroll Track -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    bind:this={railEl}
    class="horizon-rail"
    class:horizon-rail--centered={!isOverflowing && instanceList.length > 0}
    class:horizon-rail--dragging={isDragging}
    class:horizon-rail--fade-left={canScrollLeft}
    class:horizon-rail--fade-right={canScrollRight}
    onwheel={handleWheel}
    onmousedown={handleMouseDown}
    onmousemove={handleMouseMove}
    onmouseup={handleMouseUp}
    onmouseleave={handleMouseUp}
    onscroll={updateScrollState}
    role="region"
    aria-label="Instance Rail"
    tabindex="0"
  >
    {#if isLoading}
      <!-- Loading Skeleton Rail -->
      {#each [1, 2, 3, 4] as _}
        <div class="horizon-tile-skeleton animate-shimmer">
          <div class="horizon-tile-skeleton__inner"></div>
        </div>
      {/each}
    {:else if instanceList.length === 0}
      <!-- Empty State Card -->
      <div class="horizon-rail__empty">
        <div class="horizon-rail__empty-icon">🎮</div>
        <h3 class="horizon-rail__empty-title">{t('home.noInstancesTitle')}</h3>
        <p class="horizon-rail__empty-subtitle">{t('home.noInstancesSubtitle')}</p>
        <div class="horizon-rail__empty-actions">
          <Btn variant="primary" size="lg" onclick={triggerWizard}>
            + {t('home.createFirstInstance')}
          </Btn>
        </div>
      </div>
    {:else}
      <!-- Render Instance Tiles -->
      {#each instanceList as inst (inst.name)}
        <HorizonTile
          instance={inst}
          selected={inst.name === selectedName}
          running={runningSet.has(inst.name)}
          onselect={() => handleSelect(inst.name)}
        />
      {/each}

      <!-- Quick Add Instance End Tile -->
      <button
        type="button"
        class="horizon-tile-add"
        onclick={triggerWizard}
        aria-label={t('home.tileCreateNew')}
      >
        <div class="horizon-tile-add__inner">
          <div class="horizon-tile-add__icon">+</div>
          <h4 class="horizon-tile-add__title">{t('home.tileCreateNew')}</h4>
          <p class="horizon-tile-add__hint">{t('home.tileCreateNewHint')}</p>
        </div>
      </button>
    {/if}
  </div>
</section>

<style>
  .horizon-rail-wrapper {
    position: relative;
    width: 100%;
    overflow: hidden;
    padding: var(--space-4, 16px) 0;
    background: transparent;
    border: none;
    box-shadow: none;
  }

  /* --- Scroll Container --- */
  .horizon-rail {
    display: flex;
    align-items: center;
    gap: var(--space-6, 24px);
    padding: var(--space-4, 16px) calc(var(--space-8, 32px) + 8px);
    overflow-x: auto;
    overflow-y: hidden;
    scroll-snap-type: x mandatory;
    scroll-behavior: smooth;
    scrollbar-width: none;
    -ms-overflow-style: none;
    cursor: grab;
    outline: none;
    width: 100%;
    background: transparent;
    -webkit-mask-image: linear-gradient(to right, black 0%, black 100%);
    mask-image: linear-gradient(to right, black 0%, black 100%);
    transition: -webkit-mask-image var(--dur-med, 260ms) ease, mask-image var(--dur-med, 260ms) ease;
  }

  .horizon-rail--fade-left:not(.horizon-rail--fade-right) {
    -webkit-mask-image: linear-gradient(to right, transparent 0%, black 48px, black 100%);
    mask-image: linear-gradient(to right, transparent 0%, black 48px, black 100%);
  }

  .horizon-rail--fade-right:not(.horizon-rail--fade-left) {
    -webkit-mask-image: linear-gradient(to right, black 0%, black calc(100% - 48px), transparent 100%);
    mask-image: linear-gradient(to right, black 0%, black calc(100% - 48px), transparent 100%);
  }

  .horizon-rail--fade-left.horizon-rail--fade-right {
    -webkit-mask-image: linear-gradient(to right, transparent 0%, black 48px, black calc(100% - 48px), transparent 100%);
    mask-image: linear-gradient(to right, transparent 0%, black 48px, black calc(100% - 48px), transparent 100%);
  }

  .horizon-rail--centered {
    justify-content: center;
  }

  .horizon-rail::-webkit-scrollbar {
    display: none;
  }

  .horizon-rail--dragging {
    cursor: grabbing;
    scroll-snap-type: none;
    scroll-behavior: auto;
    user-select: none;
  }

  /* --- Skeletons --- */
  .horizon-tile-skeleton {
    flex: 0 0 360px;
    width: 360px;
    height: 320px;
    border-radius: var(--radius-tile, 1rem);
    background: var(--card-bg, rgba(16, 22, 42, 0.4));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    overflow: hidden;
  }

  :global([data-density='compact']) .horizon-tile-skeleton {
    flex: 0 0 280px;
    width: 280px;
    height: 260px;
  }

  /* --- Empty State --- */
  .horizon-rail__empty {
    width: 100%;
    min-height: 280px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: var(--space-8, 32px);
    background: var(--card-bg, rgba(13, 18, 34, 0.75));
    border: 1px dashed var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-xl, 1.25rem);
    margin: 0 var(--space-6, 24px);
  }

  .horizon-rail__empty-icon {
    font-size: 48px;
    margin-bottom: var(--space-3, 12px);
  }

  .horizon-rail__empty-title {
    margin: 0 0 var(--space-2, 8px) 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-2xl, 1.75rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .horizon-rail__empty-subtitle {
    margin: 0 0 var(--space-6, 24px) 0;
    max-width: 480px;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-md, 0.9375rem);
    color: var(--muted-strong, #8e9eb8);
    line-height: var(--leading-body, 1.6);
  }
  .horizon-tile-add {
    flex: 0 0 360px;
    width: 360px;
    height: 320px;
    border-radius: var(--radius-tile, 1rem);
    background: var(--surface, rgba(16, 22, 42, 0.25));
    border: 1px dashed var(--border, rgba(40, 58, 96, 0.45));
    cursor: pointer;
    scroll-snap-align: start;
    outline: none;
    transition: all var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: var(--space-4, 16px);
  }

  :global([data-density='compact']) .horizon-tile-add {
    flex: 0 0 280px;
    width: 280px;
    height: 260px;
  }

  .horizon-tile-add:hover {
    background: rgba(var(--accent-rgb, 16, 185, 129), 0.08);
    border-color: var(--accent, #10b981);
    transform: translateY(-4px);
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.4), 0 0 20px rgba(var(--accent-rgb, 16, 185, 129), 0.2);
  }

  .horizon-tile-add:focus-visible {
    box-shadow: 0 0 0 2px var(--bg, #060a14), 0 0 0 4px var(--accent, #10b981);
  }

  .horizon-tile-add__inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .horizon-tile-add__icon {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 24px;
    font-weight: 300;
    color: var(--muted-strong, #b3c5e3);
    transition: all var(--dur-fast, 120ms) ease;
  }

  .horizon-tile-add:hover .horizon-tile-add__icon {
    background: var(--accent, #10b981);
    color: #ffffff;
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 16px var(--accent, #10b981);
  }

  .horizon-tile-add__title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-base, 1rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .horizon-tile-add__hint {
    margin: 0;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted-strong, #8e9eb8);
  }
</style>
