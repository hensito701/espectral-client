<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.svelte';

  const STORAGE_KEY_HIDE = 'espectral_hide_hotkeys';

  let hidden = $state(false);

  onMount(() => {
    try {
      hidden = localStorage.getItem(STORAGE_KEY_HIDE) === 'true';
    } catch {
      hidden = false;
    }
  });

  const shortcuts = $derived([
    { key: 'CTRL+K', label: t('hotkey.command') },
    { key: '/', label: t('hotkey.search') },
    { key: 'ESPACIO', label: t('hotkey.logs') },
    { key: 'ESC', label: t('hotkey.back') },
  ]);
</script>

{#if !hidden}
  <aside class="hotkey-legend" aria-label="Atajos de teclado">
    {#each shortcuts as sc (sc.key)}
      <div class="hotkey-legend__item">
        <kbd class="hotkey-legend__key font-pixel">{sc.key}</kbd>
        <span class="hotkey-legend__label font-pixel">{sc.label}</span>
      </div>
    {/each}
  </aside>
{/if}

<style>
  .hotkey-legend {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    user-select: none;
    pointer-events: none;
  }

  .hotkey-legend__item {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    opacity: 0.75;
    transition: opacity var(--dur-fast);
  }

  .hotkey-legend__key {
    font-size: 0.5625rem;
    color: var(--accent);
    background: rgba(var(--accent-rgb), 0.1);
    border: 1px solid rgba(var(--accent-rgb), 0.25);
    border-radius: var(--radius-xs);
    padding: 2px 5px;
    line-height: 1;
    letter-spacing: 0.05em;
  }

  .hotkey-legend__label {
    font-size: 0.5625rem;
    color: var(--muted);
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  @media (max-width: 960px) {
    .hotkey-legend {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .hotkey-legend__item {
      transition: none;
    }
  }
</style>
