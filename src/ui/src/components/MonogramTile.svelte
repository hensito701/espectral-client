<!--
  ============================================================================
  MonogramTile.svelte — Horizon Glass Monogram Avatar Tile
  ============================================================================
  Deterministic avatar tile displaying either a player skin head or a
  retro pixel-font monogram initial with deterministic hue derived from the name.

  Props:
    - name: string — entity or player name for initial & hash derivation
    - src?: string — optional skin head / avatar image url
    - avatarUrl?: string — alias for src
    - hue?: number | null — overrides the name-hash hue when set
    - size?: number (default: 48) — tile width & height in pixels
    - shape?: 'tile' | 'rounded' | 'circle' (default: 'tile')
    - class?: string (or className)
-->
<script lang="ts">
  interface Props {
    name: string;
    src?: string;
    avatarUrl?: string;
    hue?: number | null;
    size?: number;
    shape?: 'tile' | 'rounded' | 'circle';
    class?: string;
    className?: string;
    [key: string]: unknown;
  }

  let {
    name,
    src,
    avatarUrl,
    hue = null,
    size = 48,
    shape = 'tile',
    class: extraClass = '',
    className = '',
    ...rest
  }: Props = $props();

  let imgFailed = $state(false);

  const imgSrc = $derived(src || avatarUrl);
  // Retry the image when its source changes (e.g. avatar uploaded after a
  // failed load, or replaced bytes under the same URL slot). Only imgSrc is
  // read, so the error handler below cannot retrigger this effect.
  $effect(() => {
    void imgSrc;
    imgFailed = false;
  });
  const letter = $derived((name || '?').trim().charAt(0).toUpperCase() || '?');

  // Deterministic hue generator based on string hash (overridable via `hue`)
  const gradientStyles = $derived.by(() => {
    let hue1: number;
    if (typeof hue === 'number' && Number.isFinite(hue)) {
      hue1 = ((Math.round(hue) % 360) + 360) % 360;
    } else {
      let hash = 0;
      const cleanName = name || 'espectral';
      for (let i = 0; i < cleanName.length; i++) {
        hash = (hash << 5) - hash + cleanName.charCodeAt(i);
        hash |= 0;
      }
      hue1 = Math.abs(hash % 360);
    }
    const hue2 = (hue1 + 40) % 360;

    return {
      bg: `linear-gradient(145deg, hsla(${hue1}, 45%, 16%, 0.9), hsla(${hue2}, 55%, 10%, 0.95))`,
      border: `hsla(${hue1}, 60%, 45%, 0.35)`,
      textGlow: `linear-gradient(135deg, hsl(${hue1}, 85%, 70%), hsl(${hue2}, 95%, 60%))`,
    };
  });

  const combinedClass = $derived([
    'monogram-tile',
    `monogram-tile--${shape}`,
    extraClass,
    className,
  ].filter(Boolean).join(' '));
</script>

<div
  class={combinedClass}
  style:width="{size}px"
  style:height="{size}px"
  style:background={gradientStyles.bg}
  style:border-color={gradientStyles.border}
  role="img"
  aria-label={name}
  {...rest}
>
  {#if imgSrc && !imgFailed}
    <img
      class="monogram-tile__img"
      src={imgSrc}
      alt={name}
      loading="lazy"
      onerror={() => { imgFailed = true; }}
    />
  {:else}
    <span
      class="monogram-tile__letter"
      style:font-size="{Math.max(10, Math.round(size * 0.45))}px"
      style:background-image={gradientStyles.textGlow}
    >
      {letter}
    </span>
  {/if}
</div>

<style>
  .monogram-tile {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    overflow: hidden;
    user-select: none;
    box-shadow: var(--shadow-sm, 0 2px 10px rgba(0, 0, 0, 0.25));
    transition: transform var(--dur-fast) var(--ease-spring), box-shadow var(--dur-fast);
  }

  /* Shape Variants */
  .monogram-tile--tile {
    border-radius: var(--radius-tile, 1rem);
  }

  .monogram-tile--rounded {
    border-radius: var(--radius-md, 0.625rem);
  }

  .monogram-tile--circle {
    border-radius: 50%;
  }

  /* Pixelated skin avatar */
  .monogram-tile__img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    image-rendering: pixelated;
  }

  /* Retro Monogram Letter */
  .monogram-tile__letter {
    font-family: var(--font-mono-retro, monospace);
    font-weight: 700;
    line-height: 1;
    background-clip: text;
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    color: transparent;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
  }
</style>
