<!--
  ============================================================================
  DensityToggle.svelte — Horizon Glass Density Switcher
  ============================================================================
  Segmented compact/spacious control persisting to localStorage 'horizon:density'
  and driving the data-density attribute on documentElement ('compact' | 'spacious').

  Props:
    - size?: 'sm' | 'md' (default: 'sm')
    - compact?: boolean (default: false)
    - class?: string (or className)
-->
<script lang="ts">
  import { t } from '../lib/i18n.svelte';

  type Density = 'compact' | 'spacious';

  interface Props {
    size?: 'sm' | 'md';
    compact?: boolean;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    size = 'sm',
    compact = false,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  function getInitialDensity(): Density {
    if (typeof window === 'undefined') return 'spacious';
    try {
      const stored = localStorage.getItem('horizon:density');
      if (stored === 'compact' || stored === 'spacious') return stored;
    } catch {
      // storage unavailable
    }
    return 'spacious';
  }

  let density = $state<Density>(getInitialDensity());

  function applyDensity(val: Density): void {
    density = val;
    if (typeof document !== 'undefined') {
      document.documentElement.setAttribute('data-density', val);
    }
    if (typeof window !== 'undefined') {
      try {
        localStorage.setItem('horizon:density', val);
      } catch {
        // storage unavailable
      }
      window.dispatchEvent(new CustomEvent('horizon:density-changed', { detail: { density: val } }));
    }
  }

  $effect(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.setAttribute('data-density', density);
    }
  });

  const combinedClass = $derived([
    'density-toggle',
    `density-toggle--${size}`,
    compact ? 'density-toggle--compact' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div
  class={combinedClass}
  role="group"
  aria-label={t('home.densityToggleAria')}
  {...rest}
>
  <button
    type="button"
    class="density-toggle__btn"
    class:density-toggle__btn--active={density === 'compact'}
    onclick={() => applyDensity('compact')}
    title={t('home.densityCompact')}
    aria-pressed={density === 'compact'}
  >
    <svg class="density-toggle__icon" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <rect x="2" y="2" width="6" height="6" rx="1.5" />
      <rect x="12" y="2" width="6" height="6" rx="1.5" />
      <rect x="2" y="12" width="6" height="6" rx="1.5" />
      <rect x="12" y="12" width="6" height="6" rx="1.5" />
    </svg>
    {#if !compact}
      <span class="density-toggle__label">{t('home.densityCompact')}</span>
    {/if}
  </button>

  <button
    type="button"
    class="density-toggle__btn"
    class:density-toggle__btn--active={density === 'spacious'}
    onclick={() => applyDensity('spacious')}
    title={t('home.densitySpacious')}
    aria-pressed={density === 'spacious'}
  >
    <svg class="density-toggle__icon" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <rect x="2" y="3" width="16" height="6" rx="1.5" />
      <rect x="2" y="11" width="16" height="6" rx="1.5" />
    </svg>
    {#if !compact}
      <span class="density-toggle__label">{t('home.densitySpacious')}</span>
    {/if}
  </button>
</div>

<style>
  .density-toggle {
    display: inline-flex;
    align-items: center;
    padding: 2px;
    background: rgba(10, 15, 30, 0.6);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25);
    user-select: none;
  }

  .density-toggle--sm {
    height: 30px;
  }

  .density-toggle--md {
    height: 36px;
    padding: 3px;
  }

  .density-toggle__btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1, 4px);
    height: 100%;
    padding: 0 var(--space-2, 8px);
    background: transparent;
    border: none;
    border-radius: var(--radius-pill, 9999px);
    color: var(--muted, #8e9eb8);
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
    outline: none;
  }

  .density-toggle--md .density-toggle__btn {
    padding: 0 var(--space-3, 12px);
    font-size: var(--text-sm, 0.875rem);
  }

  .density-toggle__btn:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.05);
  }

  .density-toggle__btn:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--accent, #10b981);
  }

  .density-toggle__btn--active {
    background: var(--surface-up-solid, #161e36);
    color: var(--text, #e8ecf4);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3), inset 0 1px 0 rgba(255, 255, 255, 0.12);
  }

  [data-theme='dark'] .density-toggle__btn--active {
    background: rgba(25, 32, 64, 0.85);
    color: #ffffff;
    border: 1px solid rgba(255, 255, 255, 0.1);
  }

  [data-theme='light'] .density-toggle {
    background: rgba(235, 240, 248, 0.7);
    border-color: rgba(200, 215, 235, 0.7);
  }

  [data-theme='light'] .density-toggle__btn--active {
    background: #ffffff;
    color: #0b1526;
    border: 1px solid rgba(0, 0, 0, 0.08);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.08);
  }

  .density-toggle__icon {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
  }

  .density-toggle--md .density-toggle__icon {
    width: 16px;
    height: 16px;
  }

  .density-toggle__label {
    white-space: nowrap;
  }
</style>
