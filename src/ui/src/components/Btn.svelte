<!--
  ============================================================================
  Btn.svelte — Horizon Glass Button Primitive
  ============================================================================
  Interactive button component featuring tactile spring motion, magnetic hover
  elevation, loading state spinner, multiple sizes, and semantic variants.

  Props:
    - variant?: 'primary' | 'secondary' | 'ghost' | 'danger' (default: 'primary')
    - size?: 'sm' | 'md' | 'lg' (default: 'md')
    - type?: 'button' | 'submit' | 'reset' (default: 'button')
    - disabled?: boolean (default: false)
    - loading?: boolean (default: false)
    - block?: boolean (default: false) — full width
    - title?: string
    - ariaLabel?: string
    - onclick?: (event: MouseEvent) => void
    - icon?: Snippet — leading icon snippet
    - children?: Snippet — button text/content
    - class?: string (or className)
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md' | 'lg';
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    loading?: boolean;
    block?: boolean;
    title?: string;
    ariaLabel?: string;
    class?: string;
    className?: string;
    onclick?: (e: MouseEvent) => void;
    icon?: Snippet;
    children?: Snippet;
    [key: string]: unknown;
  }

  let {
    variant = 'primary',
    size = 'md',
    type = 'button',
    disabled = false,
    loading = false,
    block = false,
    title,
    ariaLabel,
    class: extraClass = '',
    className = '',
    onclick,
    icon,
    children,
    ...rest
  }: Props = $props();

  const isDisabled = $derived(disabled || loading);
  const combinedClass = $derived([
    'btn',
    `btn--${variant}`,
    `btn--${size}`,
    block ? 'btn--block' : '',
    loading ? 'btn--loading' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<button
  {type}
  class={combinedClass}
  disabled={isDisabled}
  {title}
  aria-label={ariaLabel || title}
  aria-busy={loading ? 'true' : undefined}
  {onclick}
  {...rest}
>
  {#if loading}
    <span class="btn__spinner" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        <circle class="btn__spinner-track" cx="12" cy="12" r="9" stroke="currentColor" stroke-width="3" />
        <path class="btn__spinner-head" d="M12 3a9 9 0 0 1 9 9" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
      </svg>
    </span>
  {/if}

  {#if icon && !loading}
    <span class="btn__icon" aria-hidden="true">
      {@render icon()}
    </span>
  {/if}

  {#if children}
    <span class="btn__label" class:btn__label--hidden={loading && !children}>
      {@render children()}
    </span>
  {/if}
</button>

<style>
  .btn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2, 8px);
    border: 1px solid transparent;
    font-family: var(--font-body, inherit);
    font-weight: 600;
    line-height: var(--leading-tight, 1.2);
    text-decoration: none;
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
    overflow: hidden;
    vertical-align: middle;
    outline: none;
    transform: translateY(0) scale(1);
    transition:
      transform var(--dur-fast, 120ms) var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
      box-shadow var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      background-color var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      border-color var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      filter var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      opacity var(--dur-fast, 120ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
    will-change: transform;
  }

  .btn--block {
    display: flex;
    width: 100%;
  }

  /* Size Variants */
  .btn--sm {
    height: 28px;
    padding: 0 var(--space-3, 12px);
    font-size: var(--text-xs, 0.75rem);
    border-radius: var(--radius-sm, 0.375rem);
  }

  .btn--md {
    height: 38px;
    padding: 0 var(--space-4, 16px);
    font-size: var(--text-sm, 0.875rem);
    border-radius: var(--radius-md, 0.625rem);
  }

  .btn--lg {
    height: 48px;
    padding: 0 var(--space-6, 24px);
    font-size: var(--text-base, 1rem);
    border-radius: var(--radius-lg, 0.875rem);
  }

  /* Variant: Primary (Emerald Glow & High-Contrast) */
  .btn--primary {
    background: linear-gradient(135deg, var(--accent, #10b981), var(--accent-ink, #059669));
    color: var(--text-inverse, #060a14);
    border-color: rgba(255, 255, 255, 0.2);
    box-shadow:
      0 4px 16px rgba(var(--accent-rgb, 16, 185, 129), 0.28),
      inset 0 1px 0 rgba(255, 255, 255, 0.35);
  }

  .btn--primary:hover:not(:disabled) {
    transform: translateY(-2px) scale(1.01);
    box-shadow:
      0 6px 24px rgba(var(--accent-rgb, 16, 185, 129), 0.42),
      inset 0 1px 0 rgba(255, 255, 255, 0.5);
    filter: brightness(1.08);
  }

  .btn--primary:active:not(:disabled) {
    transform: translateY(0) scale(0.97);
    box-shadow:
      0 2px 8px rgba(var(--accent-rgb, 16, 185, 129), 0.3),
      inset 0 1px 0 rgba(255, 255, 255, 0.2);
  }

  /* Variant: Secondary (Translucent Glass) */
  .btn--secondary {
    background: var(--surface, rgba(16, 22, 42, 0.65));
    color: var(--text, #e8ecf4);
    border-color: var(--border, rgba(40, 58, 96, 0.45));
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .btn--secondary:hover:not(:disabled) {
    background: var(--surface-up, rgba(25, 32, 64, 0.55));
    border-color: var(--hover-border, rgba(16, 185, 129, 0.4));
    color: var(--text, #e8ecf4);
    transform: translateY(-2px);
    box-shadow: var(--shadow-md, 0 8px 28px rgba(0, 0, 0, 0.35));
  }

  .btn--secondary:active:not(:disabled) {
    transform: translateY(0) scale(0.97);
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  /* Variant: Ghost (Minimal) */
  .btn--ghost {
    background: transparent;
    color: var(--muted-strong, #b3c5e3);
    border-color: transparent;
  }

  .btn--ghost:hover:not(:disabled) {
    background: var(--surface, rgba(16, 22, 42, 0.65));
    color: var(--text, #e8ecf4);
    border-color: var(--glass-border, rgba(255, 255, 255, 0.08));
    transform: translateY(-1px);
  }

  .btn--ghost:active:not(:disabled) {
    transform: translateY(0) scale(0.97);
  }

  /* Variant: Danger (Crimson Glow) */
  .btn--danger {
    background: linear-gradient(135deg, var(--accent-red, #ef4444), #b91c1c);
    color: #ffffff;
    border-color: rgba(239, 68, 68, 0.4);
    box-shadow:
      0 4px 16px rgba(var(--accent-red-rgb, 239, 68, 68), 0.3),
      inset 0 1px 0 rgba(255, 255, 255, 0.25);
  }

  .btn--danger:hover:not(:disabled) {
    transform: translateY(-2px) scale(1.01);
    box-shadow:
      0 6px 24px rgba(var(--accent-red-rgb, 239, 68, 68), 0.45),
      inset 0 1px 0 rgba(255, 255, 255, 0.35);
    filter: brightness(1.08);
  }

  .btn--danger:active:not(:disabled) {
    transform: translateY(0) scale(0.97);
    box-shadow: 0 2px 8px rgba(var(--accent-red-rgb, 239, 68, 68), 0.25);
  }

  /* Focus and Disabled */
  .btn:focus-visible {
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  .btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
    transform: none !important;
    box-shadow: none !important;
    filter: none !important;
  }

  /* Spinner and Elements */
  .btn__icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .btn__label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1, 4px);
  }

  .btn__spinner {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1em;
    height: 1em;
    animation: btn-spin 0.75s linear infinite;
  }

  .btn__spinner svg {
    width: 100%;
    height: 100%;
  }

  .btn__spinner-track {
    opacity: 0.25;
  }

  .btn__spinner-head {
    opacity: 0.95;
  }

  @keyframes btn-spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .btn {
      transition: none !important;
      transform: none !important;
    }
    .btn__spinner {
      animation-duration: 1.5s;
    }
  }
</style>
