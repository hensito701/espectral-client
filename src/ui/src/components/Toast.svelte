<!--
  ============================================================================
  Toast.svelte — Horizon Glass Minecraft Advancement Toast Container
  ============================================================================
  Achievement-styled floating alert notification stack inspired by Minecraft
  advancement unlocks. Features spring-loaded slide-in motion, specular border,
  and reactive toast store integration with zero store redesign.

  Subscribed to `src/ui/src/lib/toast.svelte.ts` (toasts, dismissToast).
-->
<script lang="ts">
  import { toasts, dismissToast, type ToastItem } from '../lib/toast.svelte';

  function handleToastClick(item: ToastItem): void {
    if (item.href) {
      if (item.href.startsWith('#')) {
        window.location.hash = item.href;
      } else {
        window.open(item.href, '_blank', 'noopener,noreferrer');
      }
    }
    dismissToast(item.id);
  }

  function getAdvancementTag(kind: ToastItem['kind']): string {
    switch (kind) {
      case 'ok':
        return 'LOGRO DESBLOQUEADO';
      case 'err':
        return 'AVISO DEL SISTEMA';
      case 'info':
      default:
        return 'NOTIFICACIÓN';
    }
  }
</script>

{#if toasts.length > 0}
  <div class="toast-stack" role="region" aria-label="Notificaciones" aria-live="polite">
    {#each toasts as item (item.id)}
      <div
        class="toast toast--{item.kind}"
        class:toast--interactive={!!item.href}
        role="alert"
      >
        <!-- Icon Container with Specular Halo -->
        <div class="toast__icon-wrap">
          {#if item.kind === 'ok'}
            <svg class="toast__icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 6L9 17l-5-5" />
            </svg>
          {:else if item.kind === 'err'}
            <svg class="toast__icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="8" x2="12" y2="12" />
              <line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
          {:else}
            <svg class="toast__icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="16" x2="12" y2="12" />
              <line x1="12" y1="8" x2="12.01" y2="8" />
            </svg>
          {/if}
        </div>

        <!-- Content Area -->
        <button
          type="button"
          class="toast__body-btn"
          onclick={() => handleToastClick(item)}
          title={item.href ? 'Click para abrir' : 'Click para descartar'}
        >
          <span class="toast__tag">{getAdvancementTag(item.kind)}</span>
          <span class="toast__message">{item.text}</span>
        </button>

        <!-- Close / Dismiss Button -->
        <button
          type="button"
          class="toast__close"
          onclick={(e) => { e.stopPropagation(); dismissToast(item.id); }}
          title="Cerrar notificación"
          aria-label="Cerrar notificación"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    bottom: var(--space-8, 32px);
    right: var(--space-6, 24px);
    z-index: var(--z-toast, 60);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    max-width: min(24rem, calc(100vw - 32px));
    pointer-events: none;
  }

  .toast {
    position: relative;
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: var(--space-3, 12px);
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    padding: var(--space-3, 12px);
    box-shadow: var(--shadow-lg, 0 16px 48px rgba(0, 0, 0, 0.5));
    overflow: hidden;
    transform: translateX(0);
    animation: toast-spring-in var(--dur-med, 260ms) var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1));
    transition:
      transform var(--dur-fast, 120ms) var(--ease-out-expo),
      box-shadow var(--dur-fast, 120ms) var(--ease-out-expo);
  }

  /* Specular top banner glow */
  .toast::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: linear-gradient(90deg, transparent, currentColor, transparent);
    opacity: 0.8;
  }

  .toast:hover {
    transform: translateY(-2px);
  }

  /* Kind Tints */
  .toast--ok {
    color: var(--accent, #10b981);
    border-color: rgba(16, 185, 129, 0.4);
    box-shadow: var(--shadow-lg), 0 0 20px rgba(16, 185, 129, 0.15);
  }

  .toast--err {
    color: var(--accent-red, #ef4444);
    border-color: rgba(239, 68, 68, 0.4);
    box-shadow: var(--shadow-lg), 0 0 20px rgba(239, 68, 68, 0.15);
  }

  .toast--info {
    color: var(--accent-cyan, #06b6d4);
    border-color: rgba(6, 182, 212, 0.4);
    box-shadow: var(--shadow-lg), 0 0 20px rgba(6, 182, 212, 0.15);
  }

  /* Icon Wrap */
  .toast__icon-wrap {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm, 0.375rem);
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    flex-shrink: 0;
    color: currentColor;
  }

  .toast__icon {
    width: 18px;
    height: 18px;
  }

  /* Body Button */
  .toast__body-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: transparent;
    border: none;
    padding: 0;
    margin: 0;
    text-align: left;
    color: inherit;
    font-family: inherit;
    cursor: pointer;
    outline: none;
  }

  .toast__tag {
    font-family: var(--font-mono-retro, monospace);
    font-size: 0.58rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: currentColor;
    line-height: var(--leading-tight, 1.2);
  }

  .toast__message {
    font-family: var(--font-body, inherit);
    font-size: var(--text-xs, 0.75rem);
    line-height: var(--leading-snug, 1.35);
    color: var(--text, #e8ecf4);
    word-break: break-word;
  }

  /* Close Button */
  .toast__close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-xs, 0.25rem);
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: color var(--dur-fast), background var(--dur-fast);
  }

  .toast__close:hover {
    color: var(--text, #e8ecf4);
    background: var(--card-bg-soft, rgba(20, 28, 52, 0.5));
  }

  .toast__close svg {
    width: 14px;
    height: 14px;
  }

  @keyframes toast-spring-in {
    from {
      opacity: 0;
      transform: translateX(120%) scale(0.95);
    }
    to {
      opacity: 1;
      transform: translateX(0) scale(1);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: none !important;
      transition: none !important;
    }
  }

  @media (max-width: 640px) {
    .toast-stack {
      bottom: calc(var(--statusbar-height, 28px) + 12px);
      right: 12px;
      left: 12px;
      max-width: none;
    }
  }
</style>
