<!--
  ============================================================================
  ThemeToggle.svelte — Horizon Glass Theme Switcher
  ============================================================================
  Compact theme switcher button with instant visual feedback and smooth icon
  transitions between Dark and Light color schemes.

  Props:
    - theme?: Theme — optional theme override (defaults to theme store)
    - ontoggle?: () => void — optional toggle handler override
    - compact?: boolean (default: false) — ultra-compact mode for statusbar
    - class?: string (or className)
-->
<script lang="ts">
  import type { Theme } from '../lib/types';
  import { theme as themeStore, resolveTheme } from '../lib/theme.svelte';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    theme?: Theme;
    ontoggle?: () => void;
    compact?: boolean;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    theme: propTheme,
    ontoggle,
    compact = false,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  const currentTheme = $derived(propTheme ?? themeStore.value);
  const resolved = $derived(resolveTheme(currentTheme));
  const isLight = $derived(resolved === 'light');
  const label = $derived(isLight ? t('themeToggle.toDark') : t('themeToggle.toLight'));

  function handleToggle(): void {
    if (ontoggle) {
      ontoggle();
    } else {
      themeStore.set(isLight ? 'dark' : 'light');
    }
  }

  const combinedClass = $derived([
    'theme-toggle',
    compact ? 'theme-toggle--compact' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<button
  type="button"
  class={combinedClass}
  onclick={handleToggle}
  title={label}
  aria-label={label}
  aria-pressed={isLight}
  {...rest}
>
  <!-- Moon Icon (Visible in Dark Mode) -->
  <svg
    class="theme-toggle__icon theme-toggle__icon--moon"
    class:theme-toggle__icon--active={!isLight}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
  </svg>

  <!-- Sun Icon (Visible in Light Mode) -->
  <svg
    class="theme-toggle__icon theme-toggle__icon--sun"
    class:theme-toggle__icon--active={isLight}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <circle cx="12" cy="12" r="4" />
    <line x1="12" y1="2" x2="12" y2="4" />
    <line x1="12" y1="20" x2="12" y2="22" />
    <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
    <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
    <line x1="1" y1="12" x2="3" y2="12" />
    <line x1="21" y1="12" x2="23" y2="12" />
    <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
    <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
  </svg>
</button>

<style>
  .theme-toggle {
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
    overflow: hidden;
    user-select: none;
    transform: translateY(0) scale(1);
    transition:
      transform var(--dur-fast, 120ms) var(--ease-spring),
      background-color var(--dur-fast, 120ms) var(--ease-out-expo),
      border-color var(--dur-fast, 120ms) var(--ease-out-expo),
      color var(--dur-fast, 120ms) var(--ease-out-expo),
      box-shadow var(--dur-fast, 120ms) var(--ease-out-expo);
  }

  .theme-toggle--compact {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm, 0.375rem);
  }

  .theme-toggle:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border-color: var(--hover-border, rgba(16, 185, 129, 0.4));
    color: var(--accent, #10b981);
    transform: translateY(-1px);
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .theme-toggle:active {
    transform: translateY(0) scale(0.94);
  }

  .theme-toggle:focus-visible {
    outline: none;
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  .theme-toggle__icon {
    position: absolute;
    width: 16px;
    height: 16px;
    transform: scale(0.6) rotate(-45deg);
    opacity: 0;
    transition:
      transform var(--dur-med, 260ms) var(--ease-spring),
      opacity var(--dur-med, 260ms) var(--ease-out-expo);
    pointer-events: none;
  }

  .theme-toggle--compact .theme-toggle__icon {
    width: 13px;
    height: 13px;
  }

  .theme-toggle__icon--active {
    transform: scale(1) rotate(0deg);
    opacity: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .theme-toggle,
    .theme-toggle__icon {
      transition: none !important;
      transform: none !important;
    }
  }
</style>
