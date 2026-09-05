<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { SkinVariant } from '../lib/types';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    skinUrl: string | undefined;
    variant: SkinVariant;
    height?: number;
    /** Initial spin state; frozen gallery cards pass false. */
    autoplay?: boolean;
    /** Boot the WebGL viewer only once scrolled into view (gallery grid). */
    lazy?: boolean;
  }

  let { skinUrl, variant, height = 320, autoplay = true, lazy = false }: Props = $props();

  let wrapEl: HTMLDivElement | null = $state(null);
  let canvasEl: HTMLCanvasElement | null = $state(null);
  // Non-reactive handle: the three.js viewer instance (typed loosely because
  // skinview3d is an optional lazy dependency, never a static import).
  let viewer: { dispose(): void } | null = null;
  let booting = false;
  let viewerReady = $state(false);
  let viewerFailed = $state(false);
  // User-controlled spin; the square overlay button flips it. Initial value
  // follows `autoplay` (gallery cards boot frozen).
  let rotating = $state(autoplay);
  // `lazy` cards (gallery grid) stay dormant until scrolled into view: one
  // WebGL context per booted card, browsers cap ~16, so hidden cards must
  // never boot. Non-reactive observer handle, like `viewer` below.
  let inView = $state(!lazy);
  let observer: IntersectionObserver | null = null;

  function webglAvailable(): boolean {
    try {
      const probe = document.createElement('canvas');
      return !!(probe.getContext('webgl2') ?? probe.getContext('webgl'));
    } catch {
      return false;
    }
  }

  async function boot(url: string, model: 'default' | 'slim'): Promise<void> {
    if (viewer || booting || !canvasEl) return;
    if (!webglAvailable()) {
      viewerFailed = true;
      return;
    }
    booting = true;
    try {
      const mod = await import('skinview3d');
      const ViewerCtor = (mod as Record<string, unknown>).SkinViewer as
        | (new (opts: Record<string, unknown>) => {
            dispose(): void;
            loadSkin(u: string, o?: Record<string, unknown>): Promise<void> | void;
            autoRotate?: boolean;
          })
        | undefined;
      if (!canvasEl || typeof ViewerCtor !== 'function') {
        viewerFailed = true;
        return;
      }
      const width = Math.min(wrapEl?.clientWidth || 280, 300);
      const v = new ViewerCtor({ canvas: canvasEl, width, height });
      v.autoRotate = rotating;
      // Idle pose when the build exports one; purely decorative — never fatal.
      try {
        const Idle = (mod as Record<string, unknown>).IdleAnimation as
          | (new () => unknown)
          | undefined;
        if (typeof Idle === 'function') {
          const anim = new Idle();
          const anyV = v as unknown as Record<string, unknown>;
          const animations = anyV.animations as { add?: (a: unknown) => void } | undefined;
          if (typeof animations?.add === 'function') animations.add(anim);
          else anyV.animation = anim;
        }
      } catch {
        /* idle animation is optional */
      }
      await v.loadSkin(url, { model });
      if (canvasEl) {
        // Lock the rendered aspect: CSS width scaling must never stretch the
        // figure (a wide boot canvas squeezed by max-width CSS made skins
        // look thin and tall).
        canvasEl.style.aspectRatio = `${width} / ${height}`;
        viewer = v;
        viewerReady = true;
      } else {
        try {
          v.dispose();
        } catch {
          /* already unmounted */
        }
      }
    } catch {
      viewerFailed = true;
    } finally {
      booting = false;
    }
  }

  function disposeViewer(): void {
    if (viewer) {
      try {
        viewer.dispose();
      } catch {
        /* dispose is best-effort */
      }
      viewer = null;
    }
    viewerReady = false;
  }

  function toggleSpin(): void {
    rotating = !rotating;
    const v = viewer as unknown as { autoRotate?: boolean } | null;
    if (v) {
      try {
        v.autoRotate = rotating;
      } catch {
        /* toggle is best-effort */
      }
    }
  }

  // Reload the texture (or boot lazily) whenever the skin URL / model changes.
  // Lazy cards wait for the IntersectionObserver below: hidden cards must
  // never boot a WebGL context.
  $effect(() => {
    const visible = inView;
    const url = skinUrl;
    const model: 'default' | 'slim' = variant === 'slim' ? 'slim' : 'default';
    if (!visible || !url || viewerFailed) return;
    if (!viewer) {
      void boot(url, model);
      return;
    }
    const v = viewer as unknown as {
      loadSkin(u: string, o?: Record<string, unknown>): Promise<void> | void;
    };
    // NOTE: no setSize here on purpose — resizing the canvas to the (wide)
    // container and letting CSS squeeze it back is what stretched figures.
    // The aspect-ratio lock set at boot keeps proportions exact.
    void Promise.resolve(v.loadSkin(url, { model })).catch(() => {
      viewerFailed = true;
      disposeViewer();
    });
  });

  onMount(() => {
    // Lazy gallery cards boot only once scrolled into view (browsers cap
  // WebGL contexts at ~16); the $effect above boots when `inView` flips.
    if (lazy && typeof IntersectionObserver !== 'undefined' && wrapEl) {
      observer = new IntersectionObserver(
        (entries) => {
          if (entries[0]?.isIntersecting) {
            inView = true;
            observer?.disconnect();
            observer = null;
          }
        },
        { rootMargin: '100px' },
      );
      observer.observe(wrapEl);
      return () => observer?.disconnect();
    }
    inView = true;
    if (skinUrl && !viewerFailed) void boot(skinUrl, variant === 'slim' ? 'slim' : 'default');
  });

  onDestroy(() => {
    observer?.disconnect();
    observer = null;
    disposeViewer();
  });
</script>

<div class="skin-viewer" bind:this={wrapEl}>
  <canvas
    bind:this={canvasEl}
    class="skin-viewer__canvas"
    class:skin-viewer__canvas--hidden={viewerFailed || !skinUrl}
    aria-hidden="true"
  ></canvas>
  {#if viewerReady}
    <button
      type="button"
      class="skin-viewer__spin"
      class:skin-viewer__spin--paused={!rotating}
      onclick={toggleSpin}
      title={rotating ? t('vault.skin.spinPause') : t('vault.skin.spinPlay')}
      aria-label={rotating ? t('vault.skin.spinPause') : t('vault.skin.spinPlay')}
      aria-pressed={rotating}
    >
      {#if rotating}
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><rect x="2" y="1.5" width="3" height="9" rx="1" fill="currentColor"></rect><rect x="7" y="1.5" width="3" height="9" rx="1" fill="currentColor"></rect></svg>
      {:else}
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 1.8v8.4c0 .8.9 1.3 1.6.9l6-4.2c.6-.4.6-1.4 0-1.8l-6-4.2c-.7-.4-1.6.1-1.6.9z" fill="currentColor"></path></svg>
      {/if}
    </button>
  {/if}
  {#if viewerFailed && skinUrl}
    <img src={skinUrl} alt="" draggable="false" class="skin-viewer__fallback" />
    <p class="skin-viewer__error">{t('vault.skin.viewerError')}</p>
  {/if}
 </div>

<style>
  .skin-viewer {
    position: relative;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2, 8px);
    min-height: 200px;
  }

  .skin-viewer__canvas {
    width: 100%;
    max-width: 300px;
    /* Height follows the aspect-ratio lock set at boot (never stretch). */
    height: auto;
    border-radius: var(--radius-md, 8px);
    cursor: grab;
    touch-action: pan-y;
  }

  .skin-viewer__canvas:active {
    cursor: grabbing;
  }

  .skin-viewer__canvas--hidden {
    display: none;
  }

  .skin-viewer__spin {
    position: absolute;
    right: 8px;
    bottom: 8px;
    width: 32px;
    height: 32px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    background: rgba(10, 14, 22, 0.72);
    color: var(--text, #e8eef7);
    cursor: pointer;
  }

  .skin-viewer__spin:hover {
    background: rgba(20, 28, 42, 0.85);
  }

  .skin-viewer__spin--paused {
    color: var(--accent, #10b981);
  }

  .skin-viewer__fallback {
    width: auto;
    max-width: 100%;
    image-rendering: pixelated;
    border-radius: var(--radius-md, 8px);
  }

  .skin-viewer__loading,
  .skin-viewer__error {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    text-align: center;
  }
</style>
