<!--
  ============================================================================
  Badge.svelte — Horizon Glass Status & Meta Chip
  ============================================================================
  Compact badge primitive for status indicators, counts, and metadata tags.
  Supports semantic variants, loader tints, status dot indicator, and retro
  pixel-font counter mode.

  Props:
    - variant?: 'ok' | 'warn' | 'error' | 'err' | 'info' | 'neutral' | 'muted' | 'accent' | 'gold' | 'purple' | 'vanilla' | 'fabric' | 'neoforge' | 'quilt' (default: 'neutral')
    - tone?: string — backward-compatible alias for variant
    - dot?: boolean (default: false) — renders a status dot indicator
    - pixel?: boolean (default: false) — renders text in 'Press Start 2P'
    - mono?: boolean (default: false) — alias for pixel
    - size?: 'sm' | 'md' (default: 'sm')
    - title?: string
    - class?: string (or className)
    - children?: Snippet
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  type BadgeVariant =
    | 'ok'
    | 'warn'
    | 'error'
    | 'err'
    | 'info'
    | 'neutral'
    | 'muted'
    | 'accent'
    | 'gold'
    | 'purple'
    | 'vanilla'
    | 'fabric'
    | 'neoforge'
    | 'quilt';

  interface Props {
    variant?: BadgeVariant;
    tone?: BadgeVariant;
    dot?: boolean;
    pixel?: boolean;
    mono?: boolean;
    size?: 'sm' | 'md';
    title?: string;
    class?: string;
    className?: string;
    children?: Snippet;
    [key: string]: unknown;
  }

  let {
    variant,
    tone,
    dot = false,
    pixel = false,
    mono = false,
    size = 'sm',
    title,
    class: extraClass = '',
    className = '',
    children,
    ...rest
  }: Props = $props();

  const resolvedVariant = $derived(variant || tone || 'neutral');
  const isPixel = $derived(pixel || mono);

  const combinedClass = $derived([
    'badge',
    `badge--${resolvedVariant}`,
    `badge--${size}`,
    isPixel ? 'badge--pixel' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<span class={combinedClass} {title} {...rest}>
  {#if dot}
    <span class="badge__dot" aria-hidden="true"></span>
  {/if}
  {#if children}
    <span class="badge__content">
      {@render children()}
    </span>
  {/if}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1, 4px);
    border-radius: var(--radius-pill, 9999px);
    font-family: var(--font-body, inherit);
    font-weight: 600;
    line-height: var(--leading-none, 1);
    white-space: nowrap;
    vertical-align: middle;
    user-select: none;
    border: 1px solid transparent;
    transition:
      background-color var(--dur-fast, 120ms) var(--ease-out-expo),
      border-color var(--dur-fast, 120ms) var(--ease-out-expo),
      color var(--dur-fast, 120ms) var(--ease-out-expo);
  }

  /* Sizes */
  .badge--sm {
    padding: 3px 8px;
    font-size: var(--text-xs, 0.75rem);
    letter-spacing: 0.02em;
  }

  .badge--md {
    padding: 5px 12px;
    font-size: var(--text-sm, 0.875rem);
    letter-spacing: 0.02em;
  }

  /* Retro Pixel / Counter font */
  .badge--pixel {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.62rem;
    letter-spacing: 0.06em;
  }

  /* Status Dot */
  .badge__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 6px currentColor;
    flex-shrink: 0;
  }

  .badge__content {
    display: inline-flex;
    align-items: center;
  }

  /* Semantic Variants */
  .badge--ok {
    background: rgba(16, 185, 129, 0.14);
    color: var(--accent, #10b981);
    border-color: rgba(16, 185, 129, 0.35);
  }

  .badge--warn {
    background: rgba(245, 158, 11, 0.14);
    color: var(--accent-alt, #f59e0b);
    border-color: rgba(245, 158, 11, 0.35);
  }

  .badge--error,
  .badge--err {
    background: rgba(239, 68, 68, 0.14);
    color: var(--accent-red, #ef4444);
    border-color: rgba(239, 68, 68, 0.35);
  }

  .badge--info {
    background: rgba(6, 182, 212, 0.14);
    color: var(--accent-cyan, #06b6d4);
    border-color: rgba(6, 182, 212, 0.35);
  }

  .badge--neutral,
  .badge--muted {
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    color: var(--muted, #8e9eb8);
    border-color: var(--glass-border, rgba(255, 255, 255, 0.08));
  }

  .badge--accent {
    background: rgba(16, 185, 129, 0.2);
    color: var(--accent, #10b981);
    border-color: rgba(16, 185, 129, 0.45);
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.15);
  }

  .badge--gold {
    background: rgba(255, 215, 0, 0.15);
    color: var(--accent-gold, #ffd700);
    border-color: rgba(255, 215, 0, 0.35);
  }

  .badge--purple {
    background: rgba(168, 85, 247, 0.15);
    color: var(--accent-purple, #a855f7);
    border-color: rgba(168, 85, 247, 0.35);
  }

  /* Loader Tints */
  .badge--vanilla {
    background: rgba(16, 185, 129, 0.15);
    color: var(--accent, #10b981);
    border-color: rgba(16, 185, 129, 0.35);
  }

  .badge--fabric {
    background: rgba(6, 182, 212, 0.15);
    color: var(--accent-cyan, #06b6d4);
    border-color: rgba(6, 182, 212, 0.35);
  }

  .badge--neoforge {
    background: rgba(249, 115, 22, 0.15);
    color: var(--accent-alt, #f97316);
    border-color: rgba(249, 115, 22, 0.35);
  }

  .badge--quilt {
    background: rgba(168, 85, 247, 0.15);
    color: var(--accent-purple, #a855f7);
    border-color: rgba(168, 85, 247, 0.35);
  }

  /* Light Theme Overrides */
  :global([data-theme='light']) .badge--ok {
    color: var(--accent-ink, #047857);
  }

  :global([data-theme='light']) .badge--warn {
    color: var(--accent-alt, #d97706);
  }

  :global([data-theme='light']) .badge--error,
  :global([data-theme='light']) .badge--err {
    color: var(--accent-red, #dc2626);
  }

  :global([data-theme='light']) .badge--info {
    color: var(--accent-cyan, #0891b2);
  }

  :global([data-theme='light']) .badge--gold {
    color: var(--accent-gold, #b45309);
  }
</style>
