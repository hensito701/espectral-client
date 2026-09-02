<!--
  ============================================================================
  LanguageToggle.svelte — Horizon Glass Language Switcher
  ============================================================================
  Compact ES/EN locale switcher button with instant visual feedback and
  tactile spring response.

  Props:
    - compact?: boolean (default: false) — ultra-compact mode for statusbar
    - variant?: 'pill' | 'segmented' (default: 'pill')
    - class?: string (or className)
-->
<script lang="ts">
  import { lang, t } from '../lib/i18n.svelte';

  interface Props {
    compact?: boolean;
    variant?: 'pill' | 'segmented';
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    compact = false,
    variant = 'pill',
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  const isEs = $derived(lang.value === 'es');
  const target = $derived(isEs ? 'EN' : 'ES');
  const label = $derived(isEs ? t('lang.toEn') : t('lang.toEs'));

  function toggle(): void {
    lang.set(isEs ? 'en' : 'es');
  }

  const combinedClass = $derived([
    'lang-toggle',
    `lang-toggle--${variant}`,
    compact ? 'lang-toggle--compact' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

{#if variant === 'segmented'}
  <div class="lang-segmented" class:lang-segmented--compact={compact}>
    <button
      type="button"
      class="lang-seg__btn"
      class:lang-seg__btn--active={isEs}
      onclick={() => lang.set('es')}
      aria-label="Español"
      aria-pressed={isEs}
    >
      ES
    </button>
    <button
      type="button"
      class="lang-seg__btn"
      class:lang-seg__btn--active={!isEs}
      onclick={() => lang.set('en')}
      aria-label="English"
      aria-pressed={!isEs}
    >
      EN
    </button>
  </div>
{:else}
  <button
    type="button"
    class={combinedClass}
    onclick={toggle}
    title={label}
    aria-label={label}
    aria-pressed={!isEs}
    {...rest}
  >
    <span class="lang-toggle__code">{target}</span>
  </button>
{/if}

<style>
  .lang-toggle {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border-radius: var(--radius-md, 0.625rem);
    background: var(--surface, rgba(16, 22, 42, 0.65));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    color: var(--muted-strong, #b3c5e3);
    cursor: pointer;
    padding: 0;
    user-select: none;
    font-family: var(--font-body, inherit);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    letter-spacing: 0.04em;
    transform: translateY(0) scale(1);
    transition:
      transform var(--dur-fast, 120ms) var(--ease-spring),
      background-color var(--dur-fast, 120ms) var(--ease-out-expo),
      border-color var(--dur-fast, 120ms) var(--ease-out-expo),
      color var(--dur-fast, 120ms) var(--ease-out-expo),
      box-shadow var(--dur-fast, 120ms) var(--ease-out-expo);
  }

  .lang-toggle--compact {
    width: 24px;
    height: 24px;
    font-size: 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
  }

  .lang-toggle:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border-color: var(--hover-border, rgba(16, 185, 129, 0.4));
    color: var(--accent, #10b981);
    transform: translateY(-1px);
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .lang-toggle:active {
    transform: translateY(0) scale(0.94);
  }

  .lang-toggle:focus-visible {
    outline: none;
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  .lang-toggle__code {
    line-height: 1;
  }

  /* Segmented variant */
  .lang-segmented {
    display: inline-flex;
    align-items: center;
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-sm, 0.375rem);
    padding: 2px;
    gap: 2px;
  }

  .lang-segmented--compact {
    padding: 1px;
  }

  .lang-seg__btn {
    padding: 2px 6px;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--muted, #8e9eb8);
    background: transparent;
    border: none;
    border-radius: var(--radius-xs, 0.25rem);
    cursor: pointer;
    transition: all var(--dur-fast) var(--ease-out-expo);
  }

  .lang-segmented--compact .lang-seg__btn {
    padding: 1px 4px;
    font-size: 0.65rem;
  }

  .lang-seg__btn:hover {
    color: var(--text, #e8ecf4);
  }

  .lang-seg__btn--active {
    background: var(--surface-up-solid, #161e36);
    color: var(--accent, #10b981);
    box-shadow: var(--shadow-sm);
  }

  @media (prefers-reduced-motion: reduce) {
    .lang-toggle,
    .lang-seg__btn {
      transition: none !important;
      transform: none !important;
    }
  }
</style>
