<!--
  ============================================================================
  ProgressBar.svelte — Horizon Glass Linear Progress Primitive
  ============================================================================
  Linear progress bar supporting determinate and indeterminate states,
  shimmer keyframe animations, multiple height tiers, and semantic color tones.
  Zero store coupling: consumes progress values strictly via props.

  Props:
    - value?: number — current progress (0 to max). If undefined and indeterminate not specified, acts as indeterminate.
    - max?: number (default: 100) — total progress capacity
    - indeterminate?: boolean (default: false) — forces infinite looping pulse
    - height?: 'sm' | 'md' | 'lg' (default: 'md')
    - tone?: 'accent' | 'cyan' | 'gold' | 'purple' | 'danger' (default: 'accent')
    - label?: string — text label above progress bar
    - showValue?: boolean (default: false) — displays percentage/number on right
    - labelSnippet?: Snippet — custom label header
    - class?: string (or className)
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    value?: number;
    max?: number;
    indeterminate?: boolean;
    height?: 'sm' | 'md' | 'lg';
    tone?: 'accent' | 'cyan' | 'gold' | 'purple' | 'danger';
    label?: string;
    showValue?: boolean;
    labelSnippet?: Snippet;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    value,
    max = 100,
    indeterminate = false,
    height = 'md',
    tone = 'accent',
    label,
    showValue = false,
    labelSnippet,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  const isIndeterminate = $derived(indeterminate || value === undefined || value === null);
  const safeMax = $derived(max > 0 ? max : 100);
  const percentage = $derived(
    value !== undefined && value !== null
      ? Math.min(100, Math.max(0, (value / safeMax) * 100))
      : 0
  );

  const combinedClass = $derived([
    'progress-container',
    `progress--height-${height}`,
    `progress--tone-${tone}`,
    isIndeterminate ? 'progress--indeterminate' : '',
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div
  class={combinedClass}
  role="progressbar"
  aria-valuemin={0}
  aria-valuemax={safeMax}
  aria-valuenow={isIndeterminate ? undefined : Math.round(percentage)}
  aria-label={label || 'Progreso'}
  {...rest}
>
  {#if labelSnippet || label || showValue}
    <div class="progress__meta">
      {#if labelSnippet}
        <div class="progress__label">
          {@render labelSnippet()}
        </div>
      {:else if label}
        <span class="progress__label">{label}</span>
      {/if}

      {#if showValue && !isIndeterminate}
        <span class="progress__value">{Math.round(percentage)}%</span>
      {/if}
    </div>
  {/if}

  <div class="progress__track">
    {#if isIndeterminate}
      <div class="progress__fill progress__fill--indeterminate"></div>
    {:else}
      <div class="progress__fill" style:width="{percentage}%">
        <div class="progress__shimmer" aria-hidden="true"></div>
      </div>
    {/if}
  </div>
</div>

<style>
  .progress-container {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
    width: 100%;
  }

  .progress__meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs, 0.75rem);
    line-height: var(--leading-tight, 1.2);
  }

  .progress__label {
    font-weight: 500;
    color: var(--muted-strong, #b3c5e3);
  }

  .progress__value {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.65rem;
    color: var(--text, #e8ecf4);
  }

  .progress__track {
    position: relative;
    width: 100%;
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    overflow: hidden;
  }

  /* Height Variants */
  .progress--height-sm .progress__track {
    height: 4px;
  }

  .progress--height-md .progress__track {
    height: 8px;
  }

  .progress--height-lg .progress__track {
    height: 12px;
  }

  /* Progress Fill */
  .progress__fill {
    position: relative;
    height: 100%;
    border-radius: inherit;
    transition: width var(--dur-med, 260ms) var(--ease-out-expo, cubic-bezier(0.16, 1, 0.3, 1));
    overflow: hidden;
  }

  /* Tone Variants */
  .progress--tone-accent .progress__fill {
    background: linear-gradient(90deg, var(--accent-ink, #059669), var(--accent, #10b981));
    box-shadow: 0 0 10px rgba(16, 185, 129, 0.4);
  }

  .progress--tone-cyan .progress__fill {
    background: linear-gradient(90deg, #0284c7, var(--accent-cyan, #06b6d4));
    box-shadow: 0 0 10px rgba(6, 182, 212, 0.4);
  }

  .progress--tone-gold .progress__fill {
    background: linear-gradient(90deg, var(--accent-alt, #f59e0b), var(--accent-gold, #ffd700));
    box-shadow: 0 0 10px rgba(255, 215, 0, 0.4);
  }

  .progress--tone-purple .progress__fill {
    background: linear-gradient(90deg, #7e22ce, var(--accent-purple, #a855f7));
    box-shadow: 0 0 10px rgba(168, 85, 247, 0.4);
  }

  .progress--tone-danger .progress__fill {
    background: linear-gradient(90deg, #b91c1c, var(--accent-red, #ef4444));
    box-shadow: 0 0 10px rgba(239, 68, 68, 0.4);
  }

  /* Shimmer effect for active determinate fill */
  .progress__shimmer {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: linear-gradient(
      90deg,
      transparent 0%,
      rgba(255, 255, 255, 0.25) 50%,
      transparent 100%
    );
    animation: progress-shimmer 1.8s infinite;
    transform: translateX(-100%);
  }

  @keyframes progress-shimmer {
    100% {
      transform: translateX(100%);
    }
  }

  /* Indeterminate Animation */
  .progress__fill--indeterminate {
    width: 40% !important;
    animation: progress-indeterminate 1.4s cubic-bezier(0.65, 0.815, 0.735, 0.395) infinite;
  }

  @keyframes progress-indeterminate {
    0% {
      transform: translateX(-120%) scaleX(0.2);
    }
    50% {
      transform: translateX(50%) scaleX(0.8);
    }
    100% {
      transform: translateX(260%) scaleX(0.2);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .progress__shimmer {
      display: none;
    }
    .progress__fill--indeterminate {
      width: 100% !important;
      animation: none;
      opacity: 0.75;
    }
  }
</style>
