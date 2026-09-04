<!--
  ============================================================================
  Home.svelte — Horizon Glass Home & Instance Showcase Screen
  ============================================================================
  WOW console-grade dashboard featuring full-bleed horizontal instance rail,
  dynamic ambient lighting synchronization, docked telemetry flight capsule,
  live multiplayer server radar strip, and MOTD news ticker.
  ============================================================================
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { instances, servers, liveLaunches } from '../lib/stores';
  import { getAccounts } from '../lib/api';
  import { renderMotd } from '../lib/motd';
  import { t } from '../lib/i18n.svelte';

  // Components
  import HorizonRail from '../components/HorizonRail.svelte';
  import TelemetryCapsule from '../components/TelemetryCapsule.svelte';
  import ServerRadarCard from '../components/ServerRadarCard.svelte';
  import InstanceWizardModal from '../components/InstanceWizardModal.svelte';
  import MrpackModal from '../components/MrpackModal.svelte';
  import DryRunModal from '../components/DryRunModal.svelte';
  import MonogramTile from '../components/MonogramTile.svelte';
  import Btn from '../components/Btn.svelte';

  // Account State
  let activeUsername = $state('');

  // Selection & Modal States
  let selectedInstanceName = $state('');
  let wizardOpen = $state(false);
  let mrpackOpen = $state(false);
  let dryRunOpen = $state(false);

  // Greeting by time of day
  const greetingText = $derived.by(() => {
    const hour = new Date().getHours();
    const name = activeUsername || 'Jugador';
    if (hour >= 6 && hour < 13) {
      return t('home.greetingMorning', { name });
    } else if (hour >= 13 && hour < 21) {
      return t('home.greetingAfternoon', { name });
    } else {
      return t('home.greetingEvening', { name });
    }
  });

  // Top 4 servers for the radar strip
  const topServers = $derived.by(() => {
    const list = [...$servers.value];
    // Prioritize online servers, then by player count
    return list
      .sort((a, b) => {
        if (a.online !== b.online) return a.online ? -1 : 1;
        return (b.players_online || 0) - (a.players_online || 0);
      })
      .slice(0, 4);
  });

  // Featured MOTD line
  const featuredMotd = $derived.by(() => {
    const onlineServerWithMotd = $servers.value.find(
      (s) => s.online && (s.motd_raw?.length || s.motd_clean?.length),
    );
    if (onlineServerWithMotd) {
      const line = onlineServerWithMotd.motd_raw?.[0] || onlineServerWithMotd.motd_clean?.[0] || '';
      return {
        html: renderMotd(line),
        serverName: onlineServerWithMotd.hostname || onlineServerWithMotd.host,
      };
    }
    return {
      html: t('home.motdDefault'),
      serverName: 'Espectral Network',
    };
  });

  const plainMotd = $derived((featuredMotd.html || '').replace(/<[^>]*>/g, ''));
  // Derive running state for selected instance
  const isSelectedRunning = $derived.by(() => {
    if (!selectedInstanceName) return false;
    return $liveLaunches.value.some((l) => l.instance === selectedInstanceName && l.running);
  });

  async function loadAccountsList(): Promise<void> {
    try {
      const list = await getAccounts();
      let preferred = '';
      try {
        preferred = localStorage.getItem('horizon:last-account') || '';
      } catch {
        // storage unavailable
      }

      if (preferred && list.some((a) => a.username === preferred)) {
        activeUsername = preferred;
      } else {
        const sorted = [...list].sort(
          (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
        );
        if (sorted.length > 0) {
          activeUsername = sorted[0].username;
        }
      }
    } catch (e) {
      console.error('Failed to load accounts:', e);
    }
  }


  onMount(() => {
    void loadAccountsList();
    void $instances.refresh();
    void $servers.refresh();

    const onAccountChanged = (e: Event) => {
      const detail = (e as CustomEvent<{ username: string }>).detail;
      if (detail?.username) {
        activeUsername = detail.username;
      }
      void loadAccountsList();
    };

    window.addEventListener('horizon:account-changed', onAccountChanged);
    return () => window.removeEventListener('horizon:account-changed', onAccountChanged);
  });
</script>

<svelte:head>
  <title>Espectral Client — {t('home.title')}</title>
</svelte:head>

<div class="home-page animate-fade-up">
  <!-- Top Hero Header Bar -->
  <header class="home-hero-header">
    <div class="home-hero-header__left">
      <h1 class="home-greeting">
        {greetingText}
      </h1>
      <p class="home-tagline">
        {t('home.motdDefault')}
      </p>
    </div>

    <!-- Right: Active Account Chip -->
    <div class="home-hero-header__right">
      <a href="#/account" class="home-account-chip" title={t('home.activeAccount')}>
        <MonogramTile
          name={activeUsername || 'Steve'}
          size={36}
          shape="rounded"
        />
        <div class="home-account-chip__meta">
          <span class="home-account-chip__label">{t('home.activeAccount')}</span>
          <span class="home-account-chip__name">
            {activeUsername || t('home.noActiveAccount')}
          </span>
        </div>
      </a>
    </div>
  </header>

  <!-- Quick Action Toolbar -->
  <section class="home-toolbar" aria-label={t('home.quickActions')}>
    <div class="home-toolbar__actions">
      <Btn
        variant="primary"
        size="md"
        onclick={() => (wizardOpen = true)}
      >
        ✨ + {t('home.newInstance')}
      </Btn>

      <Btn
        variant="secondary"
        size="md"
        onclick={() => (mrpackOpen = true)}
      >
        📦 {t('home.importModpack')}
      </Btn>

      <Btn
        variant="ghost"
        size="md"
        onclick={() => {
          window.location.hash = '#/client';
        }}
      >
        🧩 {t('home.clientSuite')}
      </Btn>
    </div>

    <!-- Featured Live MOTD Ticker Badge -->
    <div class="home-motd-ticker" title="{featuredMotd.serverName}: {plainMotd}">
      <span class="home-motd-badge">
        <span class="home-motd-dot"></span>
        {t('home.motdLive')}
      </span>
      <div class="home-motd-content">
        <span class="home-motd-server">{featuredMotd.serverName}:</span>
        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
        <span class="home-motd-text">{@html featuredMotd.html}</span>
      </div>
    </div>
  </section>

  <!-- Center Stage: PS4-Style Instance Rail -->
  <section class="home-rail-stage">
    <div class="home-stage-header">
      <h2 class="home-stage-title">{t('home.instancesRailTitle')}</h2>
      <span class="home-stage-count">
        ({$instances.value.length})
      </span>
    </div>

    <HorizonRail
      bind:selectedName={selectedInstanceName}
      onopenwizard={() => (wizardOpen = true)}
    />
  </section>

  <!-- Launch Flight Deck Docked Capsule -->
  <TelemetryCapsule
    instanceName={selectedInstanceName}
    running={isSelectedRunning}
  />

  <!-- Multiplayer Server Radar Strip -->
  <section class="home-servers-section">
    <div class="home-servers-header">
      <div class="home-servers-header__title-wrap">
        <h3 class="home-servers-title">{t('home.serversTitle')}</h3>
        <span class="home-servers-subtitle">{t('home.serversSubtitle')}</span>
      </div>

      <a href="#/servers" class="home-servers-more-link">
        {t('home.viewAllServers')} →
      </a>
    </div>

    {#if topServers.length > 0}
      <div class="home-servers-grid">
        {#each topServers as server (server.host)}
          <ServerRadarCard {server} compact={true} interactive={true} />
        {/each}
      </div>
    {:else}
      <div class="home-servers-empty">
        <p>{t('home.noServers')}</p>
      </div>
    {/if}
  </section>

  <!-- Modals -->
  <InstanceWizardModal bind:open={wizardOpen} />
  <MrpackModal bind:open={mrpackOpen} />
  <DryRunModal bind:open={dryRunOpen} />
</div>

<style>
  .home-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-6, 24px);
    padding: var(--space-4, 16px) var(--space-6, 24px) var(--space-12, 48px);
    max-width: var(--content-max, 82rem);
    margin: 0 auto;
    width: 100%;
  }

  /* --- Hero Header --- */
  .home-hero-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-6, 24px);
    flex-wrap: wrap;
    padding-top: var(--space-2, 8px);
  }

  .home-greeting {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-hero, 2.25rem);
    font-weight: 800;
    line-height: var(--leading-tight, 1.2);
    letter-spacing: -0.03em;
    background: linear-gradient(135deg, var(--text, #ffffff) 30%, var(--muted-strong, #b3c5e3) 100%);
    -webkit-background-clip: text;
    background-clip: text;
    -webkit-text-fill-color: transparent;
  }

  .home-tagline {
    margin: 4px 0 0 0;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #e8ecf4);
  }

  .home-hero-header__right {
    display: flex;
    align-items: center;
    gap: var(--space-4, 16px);
  }

  /* Account Mirror Chip */
  .home-account-chip {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: 4px 14px 4px 6px;
    background: var(--surface, rgba(16, 22, 42, 0.55));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-pill, 9999px);
    box-shadow: var(--shadow-sm);
    text-decoration: none;
    color: var(--text, #e8ecf4);
    transition: all var(--dur-fast, 120ms) ease;
  }

  .home-account-chip:hover {
    background: var(--surface-up, rgba(25, 32, 64, 0.7));
    border-color: var(--accent, rgba(16, 185, 129, 0.4));
    transform: translateY(-2px);
    box-shadow: var(--shadow-md, 0 4px 16px rgba(0, 0, 0, 0.3));
  }

  .home-account-chip__meta {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  .home-account-chip__label {
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--muted-strong, #8e9eb8);
  }

  .home-account-chip__name {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 700;
    line-height: 1.1;
    color: var(--text, #e8ecf4);
  }

  /* --- Toolbar & MOTD --- */
  .home-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    flex-wrap: wrap;
    padding: var(--space-3, 12px) var(--space-4, 16px);
    background: var(--surface, rgba(13, 18, 34, 0.5));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.35));
    border-radius: var(--radius-lg, 0.875rem);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
  }

  .home-toolbar__actions {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    flex-wrap: wrap;
  }

  .home-motd-ticker {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-size: var(--text-xs, 0.75rem);
    max-width: 620px;
    min-width: 0;
    overflow: hidden;
  }

  .home-motd-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid rgba(239, 68, 68, 0.35);
    border-radius: var(--radius-xs, 0.25rem);
    font-family: var(--font-mono-retro, 'Press Start 2P', monospace);
    font-size: 0.625rem;
    font-weight: 700;
    color: #ef4444;
    flex-shrink: 0;
  }

  .home-motd-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #ef4444;
    box-shadow: 0 0 6px #ef4444;
    animation: pulseGlow 1.6s infinite;
  }

  .home-motd-content {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .home-motd-server {
    font-weight: 700;
    color: var(--text, #e8ecf4);
    flex-shrink: 0;
  }

  .home-motd-text {
    color: var(--muted-strong, #b3c5e3);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* --- Horizon Rail Stage --- */
  .home-rail-stage {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    margin: var(--space-2, 8px) 0;
    background: transparent;
    border: none;
    box-shadow: none;
  }

  .home-stage-header {
    display: flex;
    align-items: baseline;
    gap: var(--space-2, 8px);
    padding: 0 var(--space-2, 8px);
  }

  .home-stage-title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-xl, 1.375rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .home-stage-count {
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted-strong, #8e9eb8);
    font-weight: 600;
  }

  /* --- Servers Section --- */
  .home-servers-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
    padding: var(--space-6, 24px);
    background: var(--surface, rgba(13, 18, 34, 0.45));
    border: 1px solid var(--border, rgba(40, 58, 96, 0.35));
    border-radius: var(--radius-xl, 1.25rem);
    box-shadow: var(--shadow-sm);
  }

  .home-servers-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .home-servers-header__title-wrap {
    display: flex;
    align-items: baseline;
    gap: var(--space-3, 12px);
  }

  .home-servers-title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .home-servers-subtitle {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted-strong, #8e9eb8);
  }

  .home-servers-more-link {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--accent, #10b981);
    text-decoration: none;
    transition: color var(--dur-fast, 120ms) ease;
  }

  .home-servers-more-link:hover {
    color: var(--accent-alt, #f59e0b);
  }

  .home-servers-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--space-4, 16px);
  }

  .home-servers-empty {
    padding: var(--space-6, 24px);
    text-align: center;
    color: var(--muted-strong, #8e9eb8);
    font-size: var(--text-sm, 0.875rem);
  }
</style>
