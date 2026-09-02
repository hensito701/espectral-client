<!--
  ============================================================================
  LoginGate.svelte — Horizon Glass Discord OAuth Login Gate
  ============================================================================
  Full-screen glass portal rendered before routed application content.
  Enforces Discord supporter identity authentication with graceful anonymous bypass.
-->
<script lang="ts">
  import { t } from '../lib/i18n.svelte';
  import {
    discordSession,
    loginWithDiscord,
    continueAnonymous,
  } from '../lib/discord.svelte';
  import { fade, scalePop, flyY } from '../lib/motion';
  import Btn from './Btn.svelte';
  import GlassCard from './GlassCard.svelte';
  import Badge from './Badge.svelte';

  let localError = $state<string | null>(null);
  let localLoading = $state(false);
  let successDisplay = $state(false);

  // Monitor auth state changes to show brief success animation before gate dismisses
  $effect(() => {
    if (discordSession.status === 'authed' && discordSession.user && !successDisplay) {
      successDisplay = true;
    }
  });

  async function handleDiscordLogin() {
    localError = null;
    localLoading = true;
    try {
      await loginWithDiscord();
    } catch (err: unknown) {
      localError = err instanceof Error ? err.message : t('gate.error');
    } finally {
      localLoading = false;
    }
  }

  function handleAnonymous() {
    continueAnonymous();
  }
</script>

<div
  class="login-gate"
  in:fade={{ duration: 220 }}
  out:fade={{ duration: 280 }}
  role="dialog"
  aria-modal="true"
  aria-labelledby="gate-title"
>
  <div class="login-gate__backdrop"></div>

  <div class="login-gate__container" in:scalePop={{ duration: 320, start: 0.92 }}>
    <GlassCard className="gate-card" elevation="lg" backdrop={false}>
      {#if successDisplay && discordSession.user}
        <!-- Success State: Welcomes user and fades out -->
        <div class="gate-success" in:flyY={{ y: 8, duration: 200 }}>
          <div class="gate-avatar-box">
            {#if discordSession.user.avatar}
              <img
                src={discordSession.user.avatar}
                alt={discordSession.user.name}
                class="gate-avatar"
              />
            {:else}
              <div class="gate-avatar gate-avatar--placeholder">
                <svg width="32" height="32" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z" />
                </svg>
              </div>
            {/if}
            <div class="gate-avatar-check" aria-hidden="true">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </div>
          </div>

          <Badge variant="purple" size="sm">{t('gate.badge')}</Badge>
          <h2 class="gate-title">{t('gate.success', { name: discordSession.user.name })}</h2>
          <p class="gate-desc">{t('gate.subtitle')}</p>
        </div>
      {:else}
        <!-- Standard Login Gate Prompt -->
        <div class="gate-header">
          <div class="gate-icon-badge" aria-hidden="true">
            <svg width="44" height="42" viewBox="0 0 24 24" fill="currentColor" shape-rendering="geometricPrecision">
              <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z" />
            </svg>
          </div>
          <div class="gate-tag">
            <Badge variant="purple" size="sm">{t('gate.badge')}</Badge>
          </div>
          <h1 id="gate-title" class="gate-title">{t('gate.welcome')}</h1>
          <p class="gate-desc">{t('gate.subtitle')}</p>
        </div>

        {#if localError || discordSession.authError}
          <div class="gate-error" role="alert">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="8" x2="12" y2="12" />
              <line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
            <span>{localError || discordSession.authError}</span>
          </div>
        {/if}

        <div class="gate-actions">
          <button
            type="button"
            class="gate-btn gate-btn--discord"
            onclick={handleDiscordLogin}
            disabled={localLoading || discordSession.isAuthenticating}
          >
            {#if localLoading || discordSession.isAuthenticating}
              <span class="gate-spinner" aria-hidden="true"></span>
              <span>{t('gate.connecting')}</span>
            {:else}
              <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" class="gate-btn__icon">
                <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z" />
              </svg>
              <span>{t('gate.discord')}</span>
            {/if}
          </button>

          <Btn
            variant="secondary"
            size="md"
            block={true}
            onclick={handleAnonymous}
            disabled={localLoading || discordSession.isAuthenticating}
          >
            <span>{t('gate.anon')}</span>
          </Btn>
        </div>

        <p class="gate-footnote">{t('gate.footnote')}</p>
      {/if}
    </GlassCard>
  </div>
</div>

<style>
  .login-gate {
    position: fixed;
    inset: 0;
    z-index: var(--z-gate, 9999);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    box-sizing: border-box;
  }

  .login-gate__backdrop {
    position: absolute;
    inset: 0;
    background: rgba(6, 10, 20, 0.82);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    z-index: 1;
  }

  .login-gate__container {
    position: relative;
    z-index: 2;
    width: 100%;
    max-width: 440px;
    display: flex;
    flex-direction: column;
  }

  :global(.gate-card) {
    border: 1px solid rgba(88, 101, 242, 0.35) !important;
    box-shadow:
      0 24px 60px rgba(0, 0, 0, 0.7),
      0 0 40px rgba(88, 101, 242, 0.15) !important;
    padding: 36px 32px !important;
    text-align: center;
    background: rgba(13, 19, 36, 0.92) !important;
  }

  .gate-header {
    display: flex;
    flex-direction: column;
    align-items: center;
    margin-bottom: 24px;
  }

  .gate-icon-badge {
    width: 72px;
    height: 72px;
    border-radius: 20px;
    background: #5865f2;
    border: none;
    color: #ffffff;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 16px;
    box-shadow: 0 4px 28px rgba(88, 101, 242, 0.55);
  }

  .gate-tag {
    margin-bottom: 10px;
  }

  .gate-title {
    margin: 0 0 8px 0;
    font-size: 26px;
    font-weight: 700;
    color: #ffffff;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    letter-spacing: -0.01em;
    text-shadow: 0 1px 8px rgba(6, 10, 20, 0.55);
  }

  .gate-desc {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.55;
    color: var(--text-muted, #94a3b8);
  }

  .gate-error {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    margin-bottom: 20px;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.35);
    border-radius: var(--radius-md, 8px);
    color: #fca5a5;
    font-size: 13px;
    text-align: left;
  }

  .gate-actions {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 20px;
  }

  .gate-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    width: 100%;
    height: 44px;
    padding: 0 20px;
    border-radius: var(--radius-md, 8px);
    font-size: 14.5px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition:
      background var(--dur-fast, 120ms) ease,
      border-color var(--dur-fast, 120ms) ease,
      transform var(--dur-fast, 120ms) ease,
      box-shadow var(--dur-fast, 120ms) ease;
    border: 1px solid transparent;
  }

  .gate-btn:active:not(:disabled) {
    transform: scale(0.985);
  }

  .gate-btn--discord {
    background: #5865f2;
    color: #ffffff;
    box-shadow: 0 4px 18px rgba(88, 101, 242, 0.4);
  }

  .gate-btn--discord:hover:not(:disabled) {
    background: #4752c4;
    box-shadow: 0 6px 24px rgba(88, 101, 242, 0.55);
  }

  .gate-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .gate-btn__icon {
    flex-shrink: 0;
  }

  .gate-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #ffffff;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .gate-footnote {
    margin: 0;
    font-size: 13px;
    font-weight: 500;
    line-height: 1.55;
    color: #dbe4f3;
  }

  /* Success State */
  .gate-success {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 12px 0;
  }

  .gate-avatar-box {
    position: relative;
    margin-bottom: 16px;
  }

  .gate-avatar {
    width: 68px;
    height: 68px;
    border-radius: 50%;
    border: 2px solid #5865f2;
    box-shadow: 0 0 24px rgba(88, 101, 242, 0.4);
    object-fit: cover;
    display: block;
    background: #0f172a;
  }

  .gate-avatar--placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: #5865f2;
  }

  .gate-avatar-check {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: #10b981;
    color: #ffffff;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px solid #060a14;
    box-shadow: 0 0 8px rgba(16, 185, 129, 0.5);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
