<!--
  ============================================================================
  GradientText.svelte — Horizon Glass Metallic Gradient Text Utility
  ============================================================================
  Typography utility rendering metallic gradient-clipped text with emerald,
  gold, cyan, and cosmic shimmer presets.

  Props:
    - as?: string (default: 'span') — HTML element to render ('span', 'h1', 'p', etc.)
    - gradient?: 'emerald-gold' | 'cyan-emerald' | 'gold-amber' | 'purple-pink' (default: 'emerald-gold')
    - class?: string (or className)
    - children?: Snippet — text content to clip
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    as?: string;
    gradient?: 'emerald-gold' | 'cyan-emerald' | 'gold-amber' | 'purple-pink';
    class?: string;
    className?: string;
    children?: Snippet;
    [key: string]: unknown;
  }

  let {
    as = 'span',
    gradient = 'emerald-gold',
    class: extraClass = '',
    className = '',
    children,
    ...rest
  }: Props = $props();

  const combinedClass = $derived([
    'gradient-text',
    `gradient-text--${gradient}`,
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<svelte:element this={as} class={combinedClass} {...rest}>
  {@render children?.()}
</svelte:element>

<style>
  .gradient-text {
    display: inline-block;
    background-clip: text;
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    color: transparent;
  }

  .gradient-text--emerald-gold {
    background-image: linear-gradient(
      135deg,
      var(--accent-green) 0%,
      var(--accent-gold) 100%
    );
  }

  .gradient-text--cyan-emerald {
    background-image: linear-gradient(
      135deg,
      var(--accent-cyan) 0%,
      var(--accent-green) 100%
    );
  }

  .gradient-text--gold-amber {
    background-image: linear-gradient(
      135deg,
      var(--accent-gold) 0%,
      var(--accent-alt) 100%
    );
  }

  .gradient-text--purple-pink {
    background-image: linear-gradient(
      135deg,
      var(--accent-purple) 0%,
      #ec4899 100%
    );
  }
</style>
