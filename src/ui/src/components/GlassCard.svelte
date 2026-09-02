<!--
  ============================================================================
  GlassCard.svelte — Horizon Glass Container Primitive
  ============================================================================
  Translucent glass container featuring specular top-border highlighting,
  hardware-accelerated hover sheen sweep, configurable elevation levels,
  and optional interactive raise.

  Props:
    - title?: string
    - subtitle?: string
    - footer?: string
    - elevation?: 'none' | 'sm' | 'md' | 'lg' (default: 'md')
    - interactive?: boolean (default: false) — enables hover lift & sheen
    - featured?: boolean (default: false) — emerald glow accent border
    - backdrop?: boolean (default: true) — enables blur (disable if nested)
    - class?: string (or className)
    - onclick?: (event: MouseEvent) => void
    - header?: Snippet — custom header area
    - children?: Snippet — body content
    - footerSnippet?: Snippet — custom footer area
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title?: string;
    subtitle?: string;
    footer?: string;
    elevation?: 'none' | 'sm' | 'md' | 'lg';
    interactive?: boolean;
    featured?: boolean;
    backdrop?: boolean;
    class?: string;
    className?: string;
    onclick?: (e: MouseEvent) => void;
    header?: Snippet;
    children?: Snippet;
    footerSnippet?: Snippet;
    [key: string]: unknown;
  }

  let {
    title,
    subtitle,
    footer,
    elevation = 'md',
    interactive = false,
    featured = false,
    backdrop = true,
    class: extraClass = '',
    className = '',
    onclick,
    header,
    children,
    footerSnippet,
    ...rest
  }: Props = $props();

  const combinedClass = $derived([
    'glass-card',
    `glass-card--elevation-${elevation}`,
    interactive ? 'glass-card--interactive' : '',
    featured ? 'glass-card--featured' : '',
    backdrop ? 'glass-card--backdrop' : 'glass-card--flat-surface',
    onclick ? 'glass-card--clickable' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class={combinedClass}
  role={onclick ? 'button' : undefined}
  tabindex={onclick ? 0 : undefined}
  {onclick}
  onkeydown={onclick ? (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); (onclick as Function)(e); } } : undefined}
  {...rest}
>
  {#if header}
    <div class="glass-card__header">
      {@render header()}
    </div>
  {:else if title || subtitle}
    <div class="glass-card__header">
      {#if title}
        <h3 class="glass-card__title">{title}</h3>
      {/if}
      {#if subtitle}
        <p class="glass-card__subtitle">{subtitle}</p>
      {/if}
    </div>
  {/if}

  {#if children}
    <div class="glass-card__body">
      {@render children()}
    </div>
  {/if}

  {#if footerSnippet}
    <div class="glass-card__footer">
      {@render footerSnippet()}
    </div>
  {:else if footer}
    <div class="glass-card__footer">{footer}</div>
  {/if}
</div>

<style>
  .glass-card {
    position: relative;
    display: flex;
    flex-direction: column;
    background: var(--card-bg, rgba(13, 18, 34, 0.75));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-lg, 0.875rem);
    padding: var(--space-5, 20px);
    overflow: hidden;
    color: var(--text, #e8ecf4);
    box-shadow: var(--shadow-md, 0 8px 28px rgba(0, 0, 0, 0.35));
    transition:
      transform var(--dur-med, 260ms) var(--ease-spring-soft, cubic-bezier(0.2, 0.8, 0.2, 1)),
      box-shadow var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      border-color var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1)),
      background-color var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
  }

  /* Specular Highlight along the top edge */
  .glass-card::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent 0%,
      var(--glass-border, rgba(255, 255, 255, 0.15)) 25%,
      rgba(255, 255, 255, 0.25) 50%,
      var(--glass-border, rgba(255, 255, 255, 0.15)) 75%,
      transparent 100%
    );
    pointer-events: none;
    z-index: 1;
  }

  /* Hover Sheen Sweep (Transform-Only GPU acceleration) */
  .glass-card::after {
    content: '';
    position: absolute;
    top: -50%;
    left: -50%;
    width: 200%;
    height: 200%;
    background: linear-gradient(
      60deg,
      transparent 35%,
      rgba(255, 255, 255, 0.04) 45%,
      rgba(255, 255, 255, 0.08) 50%,
      rgba(255, 255, 255, 0.04) 55%,
      transparent 65%
    );
    transform: translateX(-150%) rotate(25deg);
    pointer-events: none;
    z-index: 2;
    transition: transform var(--dur-slow, 450ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
    will-change: transform;
  }

  .glass-card--interactive:hover::after,
  .glass-card--clickable:hover::after {
    transform: translateX(100%) rotate(25deg);
  }

  /* Backdrop Blur Discipline */
  .glass-card--backdrop {
    -webkit-backdrop-filter: blur(var(--glass-blur, 16px));
    backdrop-filter: blur(var(--glass-blur, 16px));
  }

  .glass-card--flat-surface {
    background: var(--surface-solid, #0d1222);
    -webkit-backdrop-filter: none;
    backdrop-filter: none;
  }

  /* Elevation mapping */
  .glass-card--elevation-none {
    box-shadow: none;
  }

  .glass-card--elevation-sm {
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
  }

  .glass-card--elevation-md {
    box-shadow: var(--shadow-md, 0 8px 28px rgba(0, 0, 0, 0.35));
  }

  .glass-card--elevation-lg {
    box-shadow: var(--shadow-lg, 0 16px 48px rgba(0, 0, 0, 0.5));
  }

  /* Featured Emerald Accent */
  .glass-card--featured {
    border-color: var(--border-focus, rgba(16, 185, 129, 0.6));
    box-shadow: var(--glass-glow, 0 8px 32px rgba(16, 185, 129, 0.15));
  }

  .glass-card--featured::before {
    background: linear-gradient(
      90deg,
      transparent 0%,
      var(--accent, #10b981) 50%,
      transparent 100%
    );
    height: 2px;
  }

  /* Interactive State */
  .glass-card--interactive:hover,
  .glass-card--clickable:hover {
    transform: translateY(-4px);
    border-color: var(--hover-border, rgba(16, 185, 129, 0.4));
    box-shadow: var(--shadow-lg, 0 16px 48px rgba(0, 0, 0, 0.5));
  }

  .glass-card--clickable {
    cursor: pointer;
    user-select: none;
  }

  .glass-card--clickable:active {
    transform: translateY(-1px);
  }

  .glass-card--clickable:focus-visible {
    outline: none;
    box-shadow: var(--shadow-focus, 0 0 0 2px var(--bg), 0 0 0 4px var(--accent));
  }

  /* Internal Sections */
  .glass-card__header {
    margin-bottom: var(--space-3, 12px);
  }

  .glass-card__title {
    margin: 0;
    font-size: var(--text-lg, 1.125rem);
    font-weight: 600;
    line-height: var(--leading-snug, 1.35);
    color: var(--text, #e8ecf4);
  }

  .glass-card__subtitle {
    margin: var(--space-1, 4px) 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .glass-card__body {
    flex: 1;
    min-width: 0;
  }

  .glass-card__footer {
    margin-top: var(--space-4, 16px);
    padding-top: var(--space-3, 12px);
    border-top: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  @media (prefers-reduced-motion: reduce) {
    .glass-card {
      transition: none !important;
      transform: none !important;
    }
    .glass-card::after {
      display: none !important;
    }
  }
</style>
