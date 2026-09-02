<!--
  ============================================================================
  LoaderBadge.svelte — Horizon Glass Deterministic Mod Loader Chip
  ============================================================================
  Deterministic mod loader badge (Vanilla, Fabric, NeoForge, Forge, Quilt)
  with version tag, gradient backdrop, specular border, and size tiers.

  Props:
    - loader?: 'vanilla' | 'fabric' | 'neoforge' | 'forge' | 'quilt' | string (default: 'vanilla')
    - version?: string — game or loader version string (e.g. '1.21.1')
    - size?: 'sm' | 'md' | 'lg' (default: 'md')
    - showIcon?: boolean (default: true)
    - class?: string (or className)
-->
<script lang="ts">
  type LoaderType = 'vanilla' | 'fabric' | 'neoforge' | 'forge' | 'quilt' | string;

  interface Props {
    loader?: LoaderType;
    version?: string;
    size?: 'sm' | 'md' | 'lg';
    showIcon?: boolean;
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    loader = 'vanilla',
    version,
    size = 'md',
    showIcon = true,
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  const normalizedLoader = $derived((loader || 'vanilla').toLowerCase().trim());

  const displayName = $derived.by(() => {
    switch (normalizedLoader) {
      case 'vanilla':
        return 'Vanilla';
      case 'fabric':
        return 'Fabric';
      case 'neoforge':
        return 'NeoForge';
      case 'forge':
        return 'Forge';
      case 'quilt':
        return 'Quilt';
      default:
        return loader ? loader.charAt(0).toUpperCase() + loader.slice(1) : 'Custom';
    }
  });

  const combinedClass = $derived([
    'loader-badge',
    `loader-badge--${normalizedLoader}`,
    `loader-badge--${size}`,
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div class={combinedClass} {...rest}>
  <div class="loader-badge__chip">
    {#if showIcon}
      <span class="loader-badge__icon" aria-hidden="true">
        {#if normalizedLoader === 'vanilla'}
          <!-- Grass/Pickaxe Cube Icon -->
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="M12 2L2 7v10l10 5 10-5V7L12 2zm0 2.8l6.9 3.45L12 11.7 5.1 8.25 12 4.8zM4 9.8l7 3.5v6.9l-7-3.5V9.8zm9 10.4v-6.9l7-3.5v6.9l-7 3.5z" />
          </svg>
        {:else if normalizedLoader === 'fabric'}
          <!-- Thread/Fabric Woven Icon -->
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm1 17.93V13h4.93A8 8 0 0 1 13 19.93zM6.07 13H11v6.93A8 8 0 0 1 6.07 13zm4.93-2H6.07A8 8 0 0 1 11 4.07zm2-6.93A8 8 0 0 1 17.93 11H13z" />
          </svg>
        {:else if normalizedLoader === 'neoforge' || normalizedLoader === 'forge'}
          <!-- Anvil/Hammer Flame Icon -->
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="M19.48 12.35c-1.57-.56-2.67-1.42-3.08-2.61a5.6 5.6 0 0 1-.22-1.84c.14-.94.55-1.9 1.1-2.73-1.63.47-3.07 1.4-4.04 2.76A6.9 6.9 0 0 0 12 12c-1.39 0-2.67-.54-3.64-1.43a6.83 6.83 0 0 0-4.04-2.76c.55.83.96 1.79 1.1 2.73.08.62.01 1.25-.22 1.84-.41 1.19-1.51 2.05-3.08 2.61C1.19 12.69 0 13.9 0 15.35 0 19.02 2.98 22 6.65 22h10.7c3.67 0 6.65-2.98 6.65-6.65 0-1.45-1.19-2.66-2.12-3z" />
          </svg>
        {:else if normalizedLoader === 'quilt'}
          <!-- Quilt Patchwork Icon -->
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="M4 4h7v7H4V4zm9 0h7v7h-7V4zm-9 9h7v7H4v-7zm9 0h7v7h-7v-7z" />
          </svg>
        {:else}
          <!-- Generic Gear Icon -->
          <svg viewBox="0 0 24 24" fill="currentColor">
            <circle cx="12" cy="12" r="8" />
          </svg>
        {/if}
      </span>
    {/if}
    <span class="loader-badge__name">{displayName}</span>
  </div>

  {#if version}
    <span class="loader-badge__version">{version}</span>
  {/if}
</div>

<style>
  .loader-badge {
    display: inline-flex;
    align-items: center;
    border-radius: var(--radius-pill, 9999px);
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    overflow: hidden;
    user-select: none;
    line-height: var(--leading-none, 1);
    white-space: nowrap;
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
    transition: transform var(--dur-fast), border-color var(--dur-fast);
  }

  .loader-badge__chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1, 4px);
    font-weight: 700;
    color: #ffffff;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .loader-badge__icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .loader-badge__icon svg {
    width: 100%;
    height: 100%;
  }

  .loader-badge__name {
    letter-spacing: 0.02em;
  }

  .loader-badge__version {
    font-family: var(--font-mono, monospace);
    font-weight: 500;
    color: var(--muted-strong, #b3c5e3);
    letter-spacing: -0.01em;
  }

  /* Size Variants */
  .loader-badge--sm {
    padding: 2px 2px 2px 2px;
    font-size: 0.68rem;
  }

  .loader-badge--sm .loader-badge__chip {
    padding: 2px 6px;
    border-radius: var(--radius-pill, 9999px);
  }

  .loader-badge--sm .loader-badge__icon {
    width: 10px;
    height: 10px;
  }

  .loader-badge--sm .loader-badge__version {
    padding: 0 6px 0 4px;
    font-size: 0.65rem;
  }

  .loader-badge--md {
    padding: 3px 3px 3px 3px;
    font-size: var(--text-xs, 0.75rem);
  }

  .loader-badge--md .loader-badge__chip {
    padding: 3px 8px;
    border-radius: var(--radius-pill, 9999px);
  }

  .loader-badge--md .loader-badge__icon {
    width: 12px;
    height: 12px;
  }

  .loader-badge--md .loader-badge__version {
    padding: 0 8px 0 6px;
    font-size: 0.72rem;
  }

  .loader-badge--lg {
    padding: 4px 4px 4px 4px;
    font-size: var(--text-sm, 0.875rem);
  }

  .loader-badge--lg .loader-badge__chip {
    padding: 4px 10px;
    border-radius: var(--radius-pill, 9999px);
  }

  .loader-badge--lg .loader-badge__icon {
    width: 14px;
    height: 14px;
  }

  .loader-badge--lg .loader-badge__version {
    padding: 0 10px 0 8px;
    font-size: 0.82rem;
  }

  /* Deterministic Loader Gradients */
  .loader-badge--vanilla .loader-badge__chip {
    background: var(--loader-vanilla, linear-gradient(135deg, #10b981, #059669));
  }

  .loader-badge--fabric .loader-badge__chip {
    background: var(--loader-fabric, linear-gradient(135deg, #06b6d4, #3b82f6));
  }

  .loader-badge--neoforge .loader-badge__chip {
    background: var(--loader-neoforge, linear-gradient(135deg, #f97316, #ef4444));
  }

  .loader-badge--forge .loader-badge__chip {
    background: linear-gradient(135deg, #f59e0b, #d97706);
  }

  .loader-badge--quilt .loader-badge__chip {
    background: var(--loader-quilt, linear-gradient(135deg, #a855f7, #ec4899));
  }
</style>
