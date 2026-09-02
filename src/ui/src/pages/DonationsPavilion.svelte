<!--
  ============================================================================
  DonationsPavilion.svelte — Supporter Pavilion & VIP Lounge (#/donaciones)
  ============================================================================
  Horizon Glass supporter showcase presenting official Espectral donation
  channels (direct PayPal, Tebex web store, Discord Nitro boosts), tiered
  VIP rank pedestals with cosmetic perk previews, and the community unban
  fair-play matrix.
-->
<script lang="ts">
  import GlassCard from '../components/GlassCard.svelte';
  import GradientText from '../components/GradientText.svelte';
  import Badge from '../components/Badge.svelte';
  import Btn from '../components/Btn.svelte';
  import { useCopy } from '../lib/useCopy.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { t } from '../lib/i18n.svelte';

  type RankCategory = 'all' | 'permanent' | 'service';
  type TierTone = 'diamond' | 'emerald' | 'gold' | 'purple' | 'amber';

  interface RankItem {
    id: string;
    nameKey: string;
    priceKey: string;
    taglineKey: string;
    category: 'permanent' | 'service';
    tierTone: TierTone;
    badgeKey?: string;
    featured?: boolean;
    perks: string[];
    accentColor: string;
  }

  const RANKS: RankItem[] = [
    {
      id: 'vip',
      nameKey: 'donations.rank.vip',
      priceKey: 'donations.rank.vip.price',
      taglineKey: 'pav.rank.vip.tagline',
      category: 'permanent',
      tierTone: 'diamond',
      accentColor: '#38bdf8',
      perks: [
        'donations.rank.vip.perk1',
        'donations.rank.vip.perk2',
        'donations.rank.vip.perk3',
        'donations.rank.vip.perk4',
        'donations.rank.vip.perk5',
        'donations.rank.vip.perk6',
      ],
    },
    {
      id: 'vip-plus',
      nameKey: 'donations.rank.vipPlus',
      priceKey: 'donations.rank.vipPlus.price',
      taglineKey: 'pav.rank.vipPlus.tagline',
      category: 'permanent',
      tierTone: 'emerald',
      accentColor: '#10b981',
      perks: [
        'donations.rank.vipPlus.perk1',
        'donations.rank.vipPlus.perk2',
        'donations.rank.vipPlus.perk3',
        'donations.rank.vipPlus.perk4',
      ],
    },
    {
      id: 'mvp',
      nameKey: 'donations.rank.mvp',
      priceKey: 'donations.rank.mvp.price',
      taglineKey: 'pav.rank.mvp.tagline',
      category: 'permanent',
      tierTone: 'diamond',
      accentColor: '#06b6d4',
      perks: [
        'donations.rank.mvp.perk1',
        'donations.rank.mvp.perk2',
        'donations.rank.mvp.perk3',
        'donations.rank.mvp.perk4',
        'donations.rank.mvp.perk5',
        'donations.rank.mvp.perk6',
        'donations.rank.mvp.perk7',
      ],
    },
    {
      id: 'mvp-plus',
      nameKey: 'donations.rank.mvpPlus',
      priceKey: 'donations.rank.mvpPlus.price',
      taglineKey: 'pav.rank.mvpPlus.tagline',
      category: 'permanent',
      tierTone: 'emerald',
      featured: true,
      badgeKey: 'pav.rankFeatured',
      accentColor: '#059669',
      perks: [
        'donations.rank.mvpPlus.perk1',
        'donations.rank.mvpPlus.perk2',
        'donations.rank.mvpPlus.perk3',
      ],
    },
    {
      id: 'custom',
      nameKey: 'donations.rank.custom',
      priceKey: 'donations.rank.custom.price',
      taglineKey: 'pav.rank.custom.tagline',
      category: 'permanent',
      tierTone: 'gold',
      badgeKey: 'pav.rankExclusive',
      accentColor: '#f59e0b',
      perks: [
        'donations.rank.custom.perk1',
        'donations.rank.custom.perk2',
      ],
    },
    {
      id: 'dhost',
      nameKey: 'donations.rank.dhost',
      priceKey: 'donations.rank.dhost.price',
      taglineKey: 'pav.rank.dhost.tagline',
      category: 'service',
      tierTone: 'purple',
      badgeKey: 'pav.rankMonthly',
      accentColor: '#a855f7',
      perks: [
        'donations.rank.dhost.perk1',
        'donations.rank.dhost.perk2',
      ],
    },
    {
      id: 'private-uhc',
      nameKey: 'donations.rank.privateUhc',
      priceKey: 'donations.rank.privateUhc.price',
      taglineKey: 'pav.rank.privateUhc.tagline',
      category: 'service',
      tierTone: 'amber',
      badgeKey: 'pav.rankEvent',
      accentColor: '#f97316',
      perks: [
        'donations.rank.privateUhc.perk1',
        'donations.rank.privateUhc.perk2',
      ],
    },
  ];

  const UNBAN_ROWS = [
    { k: 'donations.unbans.first', v: 'donations.unbans.firstPrice', level: '1' },
    { k: 'donations.unbans.second', v: 'donations.unbans.secondPrice', level: '2' },
    { k: 'donations.unbans.third', v: 'donations.unbans.thirdPrice', level: '3+' },
  ];

  const PAYPAL_URL = 'https://paypal.me/uhcespectral';
  const TEBEX_URL = 'https://espectral.tebex.io/';
  const DISCORD_URL = 'https://discord.gg/espectral';

  let selectedCategory = $state<RankCategory>('all');
  let searchQuery = $state('');

  const paypalCopier = useCopy(1600);
  const tebexCopier = useCopy(1600);
  const discordCopier = useCopy(1600);

  function copyUrl(copier: { copy: (t: string) => void }, url: string) {
    copier.copy(url);
    pushToast({
      kind: 'ok',
      text: t('pav.copiedToast'),
    });
  }

  // Filtered ranks based on category tab & search keyword
  const filteredRanks = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return RANKS.filter((rank) => {
      // Category filter
      if (selectedCategory !== 'all' && rank.category !== selectedCategory) {
        return false;
      }
      // Search filter
      if (q) {
        const name = t(rank.nameKey).toLowerCase();
        const price = t(rank.priceKey).toLowerCase();
        const tagline = t(rank.taglineKey).toLowerCase();
        const matchesPerks = rank.perks.some((pKey) => t(pKey).toLowerCase().includes(q));
        if (!name.includes(q) && !price.includes(q) && !tagline.includes(q) && !matchesPerks) {
          return false;
        }
      }
      return true;
    });
  });

  const permanentCount = $derived(RANKS.filter((r) => r.category === 'permanent').length);
  const servicesCount = $derived(RANKS.filter((r) => r.category === 'service').length);
</script>

<svelte:head>
  <title>{t('pav.tag')}</title>
</svelte:head>

<div class="pavilion-page">
  <!-- 1. Hero Pavilion Showcase -->
  <header class="pavilion-hero">
    <div class="pavilion-hero__inner">
      <div class="pavilion-hero__badge-row">
        <Badge variant="gold" size="sm" dot={true}>
          {t('pav.heroBadge')}
        </Badge>
        <span class="pavilion-hero__tagline">ESPECTRAL CLIENT NETWORK</span>
      </div>

      <div class="pavilion-hero__main">
        <h1 class="pavilion-hero__title">
          <GradientText as="span">{t('pav.heroTitle')}</GradientText>
        </h1>
        <p class="pavilion-hero__pitch">
          {t('pav.heroPitch')}
        </p>
      </div>

      <!-- Feature highlight capsules -->
      <div class="pavilion-hero__capsules">
        <div class="capsule">
          <span class="capsule__icon" aria-hidden="true">🛡️</span>
          <span class="capsule__text">{t('pav.perkHighlight1')}</span>
        </div>
        <div class="capsule">
          <span class="capsule__icon" aria-hidden="true">⚡</span>
          <span class="capsule__text">{t('pav.perkHighlight2')}</span>
        </div>
        <div class="capsule">
          <span class="capsule__icon" aria-hidden="true">🎮</span>
          <span class="capsule__text">{t('pav.perkHighlight3')}</span>
        </div>
      </div>
    </div>
  </header>

  <!-- 2. Official Support Channels Grid -->
  <section class="pavilion-section" aria-labelledby="channels-heading">
    <div class="section-header">
      <div>
        <h2 id="channels-heading" class="section-title">{t('pav.channelsTitle')}</h2>
        <p class="section-subtitle">{t('pav.channelsSubtitle')}</p>
      </div>
    </div>

    <div class="channels-grid">
      <!-- PayPal Direct Card -->
      <GlassCard className="channel-card channel-card--paypal" elevation="md">
        <div class="channel-card__head">
          <div class="channel-card__brand">
            <div class="channel-card__icon-box channel-card__icon-box--paypal" aria-hidden="true">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M7 11.5L9.5 2h6a4.5 4.5 0 0 1 0 9H11.5L10 18.5H6.5L7 11.5z" />
                <path d="M10 8.5L12 2h5a3.5 3.5 0 0 1 0 7h-4l-1.5 6.5H8" stroke-opacity="0.6" />
              </svg>
            </div>
            <div>
              <h3 class="channel-card__title">{t('pav.paypalTitle')}</h3>
              <span class="channel-card__url">paypal.me/uhcespectral</span>
            </div>
          </div>
          <Badge variant="accent" size="sm">{t('pav.paypalBadge')}</Badge>
        </div>

        <p class="channel-card__desc">{t('pav.paypalDesc')}</p>

        <div class="channel-card__actions">
          <a
            href={PAYPAL_URL}
            target="_blank"
            rel="noopener noreferrer"
            class="channel-btn channel-btn--primary"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
              <polyline points="15 3 21 3 21 9" />
              <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
            <span>{t('pav.paypalAction')}</span>
          </a>
          <Btn
            variant="secondary"
            size="md"
            onclick={() => copyUrl(paypalCopier, PAYPAL_URL)}
            title={t('pav.copyLink')}
          >
            {#snippet icon()}
              {#if paypalCopier.copied}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="2.5">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {:else}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                </svg>
              {/if}
            {/snippet}
            <span>{paypalCopier.copied ? t('pav.copied') : t('pav.copyLink')}</span>
          </Btn>
        </div>
      </GlassCard>

      <!-- Tebex Official Store Card -->
      <GlassCard className="channel-card channel-card--tebex" elevation="md">
        <div class="channel-card__head">
          <div class="channel-card__brand">
            <div class="channel-card__icon-box channel-card__icon-box--tebex" aria-hidden="true">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="9" cy="21" r="1" />
                <circle cx="20" cy="21" r="1" />
                <path d="M1 1h4l2.68 13.39a2 2 0 0 0 2 1.61h9.72a2 2 0 0 0 2-1.61L23 6H6" />
              </svg>
            </div>
            <div>
              <h3 class="channel-card__title">{t('pav.tebexTitle')}</h3>
              <span class="channel-card__url">espectral.tebex.io</span>
            </div>
          </div>
          <Badge variant="neutral" size="sm">{t('pav.tebexBadge')}</Badge>
        </div>

        <p class="channel-card__desc">{t('pav.tebexDesc')}</p>

        <div class="channel-card__actions">
          <a
            href={TEBEX_URL}
            target="_blank"
            rel="noopener noreferrer"
            class="channel-btn channel-btn--secondary"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
              <polyline points="15 3 21 3 21 9" />
              <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
            <span>{t('pav.tebexAction')}</span>
          </a>
          <Btn
            variant="secondary"
            size="md"
            onclick={() => copyUrl(tebexCopier, TEBEX_URL)}
            title={t('pav.copyLink')}
          >
            {#snippet icon()}
              {#if tebexCopier.copied}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="2.5">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {:else}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                </svg>
              {/if}
            {/snippet}
            <span>{tebexCopier.copied ? t('pav.copied') : t('pav.copyLink')}</span>
          </Btn>
        </div>
      </GlassCard>

      <!-- Discord Nitro Boost Card -->
      <GlassCard className="channel-card channel-card--discord" elevation="md">
        <div class="channel-card__head">
          <div class="channel-card__brand">
            <div class="channel-card__icon-box channel-card__icon-box--discord" aria-hidden="true">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor">
                <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994.021-.041.001-.09-.041-.106a13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.929 1.793 8.18 1.793 12.061 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.893.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.028zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
              </svg>
            </div>
            <div>
              <h3 class="channel-card__title">{t('pav.discordTitle')}</h3>
              <span class="channel-card__url">discord.gg/espectral</span>
            </div>
          </div>
          <Badge variant="purple" size="sm">{t('pav.discordBadge')}</Badge>
        </div>

        <p class="channel-card__desc">{t('pav.discordDesc')}</p>

        <div class="channel-card__actions">
          <a
            href={DISCORD_URL}
            target="_blank"
            rel="noopener noreferrer"
            class="channel-btn channel-btn--discord"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
              <polyline points="15 3 21 3 21 9" />
              <line x1="10" y1="14" x2="21" y2="3" />
            </svg>
            <span>{t('pav.discordAction')}</span>
          </a>
          <Btn
            variant="secondary"
            size="md"
            onclick={() => copyUrl(discordCopier, DISCORD_URL)}
            title={t('pav.copyLink')}
          >
            {#snippet icon()}
              {#if discordCopier.copied}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="2.5">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              {:else}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                  <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                </svg>
              {/if}
            {/snippet}
            <span>{discordCopier.copied ? t('pav.copied') : t('pav.copyLink')}</span>
          </Btn>
        </div>
      </GlassCard>
    </div>
  </section>

  <!-- 3. Ranks & VIP Lounge Pedestals -->
  <section class="pavilion-section" aria-labelledby="ranks-heading">
    <div class="ranks-toolbar">
      <div class="section-header">
        <h2 id="ranks-heading" class="section-title">{t('pav.ranksTitle')}</h2>
        <p class="section-subtitle">{t('pav.ranksSubtitle')}</p>
      </div>

      <!-- Category Filter Tabs + Search -->
      <div class="toolbar-controls">
        <div class="category-tabs" role="tablist">
          <button
            type="button"
            role="tab"
            class="tab-btn"
            class:tab-btn--active={selectedCategory === 'all'}
            aria-selected={selectedCategory === 'all'}
            onclick={() => (selectedCategory = 'all')}
          >
            {t('pav.filterAll', { count: RANKS.length })}
          </button>
          <button
            type="button"
            role="tab"
            class="tab-btn"
            class:tab-btn--active={selectedCategory === 'permanent'}
            aria-selected={selectedCategory === 'permanent'}
            onclick={() => (selectedCategory = 'permanent')}
          >
            {t('pav.filterPermanent', { count: permanentCount })}
          </button>
          <button
            type="button"
            role="tab"
            class="tab-btn"
            class:tab-btn--active={selectedCategory === 'service'}
            aria-selected={selectedCategory === 'service'}
            onclick={() => (selectedCategory = 'service')}
          >
            {t('pav.filterServices', { count: servicesCount })}
          </button>
        </div>

        <div class="search-wrap">
          <svg class="search-icon" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="search"
            class="search-input"
            placeholder={t('pav.searchPlaceholder')}
            bind:value={searchQuery}
          />
          {#if searchQuery}
            <button
              type="button"
              class="clear-btn"
              onclick={() => (searchQuery = '')}
              title={t('pav.clearSearch')}
            >
              ×
            </button>
          {/if}
        </div>
      </div>
    </div>

    <!-- General Disclaimer Banner -->
    <div class="disclaimer-banner">
      <span class="disclaimer-banner__icon" aria-hidden="true">ℹ️</span>
      <p class="disclaimer-banner__text">{t('donations.disclaimer')}</p>
    </div>

    <!-- Ranks Grid -->
    {#if filteredRanks.length === 0}
      <div class="empty-ranks">
        <p class="empty-ranks__text">{t('pav.noRanksFound')}</p>
        <Btn variant="secondary" size="sm" onclick={() => { selectedCategory = 'all'; searchQuery = ''; }}>
          {t('pav.clearSearch')}
        </Btn>
      </div>
    {:else}
      <div class="ranks-grid">
        {#each filteredRanks as rank (rank.id)}
          <GlassCard
            className={`rank-card rank-card--${rank.tierTone} ${rank.featured ? 'rank-card--featured' : ''}`}
            elevation={rank.featured ? 'lg' : 'md'}
          >
            <!-- Rank Card Head -->
            <div class="rank-card__head">
              <div class="rank-card__badge-row">
                {#if rank.featured}
                  <Badge variant="gold" size="sm" dot={true}>
                    {t(rank.badgeKey || 'pav.rankFeatured')}
                  </Badge>
                {:else if rank.badgeKey}
                  <Badge variant="purple" size="sm">
                    {t(rank.badgeKey)}
                  </Badge>
                {:else if rank.tierTone === 'diamond'}
                  <Badge variant="info" size="sm">{t('pav.tierDiamond')}</Badge>
                {:else if rank.tierTone === 'gold'}
                  <Badge variant="gold" size="sm">{t('pav.tierGold')}</Badge>
                {:else}
                  <Badge variant="accent" size="sm">{t('pav.tierEmerald')}</Badge>
                {/if}
              </div>

              <div class="rank-card__identity">
                <h3 class="rank-card__name">{t(rank.nameKey)}</h3>
                <span class="rank-card__price">{t(rank.priceKey)}</span>
              </div>

              <p class="rank-card__tagline">{t(rank.taglineKey)}</p>
            </div>

            <div class="rank-card__divider"></div>

            <!-- Perk List -->
            <div class="rank-card__body">
              <div class="rank-card__perks-label">
                {t('pav.perksCount', { count: rank.perks.length })}
              </div>
              <ul class="perk-list">
                {#each rank.perks as perkKey}
                  <li class="perk-item">
                    <span class="perk-bullet" aria-hidden="true" style:background={rank.accentColor}></span>
                    <span class="perk-text">{t(perkKey)}</span>
                  </li>
                {/each}
              </ul>
            </div>

            <!-- Card Footer CTA -->
            <div class="rank-card__foot">
              <a
                href={TEBEX_URL}
                target="_blank"
                rel="noopener noreferrer"
                class="rank-cta-btn"
                style:--rank-accent={rank.accentColor}
              >
                <span>{t('pav.getRank')}</span>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                  <line x1="5" y1="12" x2="19" y2="12" />
                  <polyline points="12 5 19 12 12 19" />
                </svg>
              </a>
            </div>
          </GlassCard>
        {/each}
      </div>
    {/if}
  </section>

  <!-- 4. Community Unbans Transparency Matrix -->
  <section class="pavilion-section" aria-labelledby="unbans-heading">
    <GlassCard className="unbans-card" elevation="md">
      <div class="unbans-head">
        <div>
          <div class="unbans-badge-row">
            <Badge variant="warn" size="sm" dot={true}>FAIR PLAY & RECOVERY</Badge>
          </div>
          <h2 id="unbans-heading" class="unbans-title">{t('pav.unbansTitle')}</h2>
          <p class="unbans-subtitle">{t('pav.unbansSubtitle')}</p>
        </div>
      </div>

      <p class="unbans-intro">{t('pav.unbansNote')}</p>

      <!-- Unban Tiers Grid -->
      <div class="unbans-grid">
        {#each UNBAN_ROWS as row}
          <div class="unban-tier-card">
            <div class="unban-tier-card__level">
              <span class="unban-tier-card__chip"># {row.level}</span>
              <span class="unban-tier-card__label">{t(row.k)}</span>
            </div>
            <div class="unban-tier-card__price">{t(row.v)}</div>
          </div>
        {/each}
      </div>

      <!-- Strict Warnings & Policy Notes -->
      <div class="unbans-warnings">
        <div class="warning-box warning-box--info">
          <span class="warning-box__icon" aria-hidden="true">📌</span>
          <p class="warning-box__text">{t('donations.unbans.warning1')}</p>
        </div>
        <div class="warning-box warning-box--strict">
          <span class="warning-box__icon" aria-hidden="true">⚠️</span>
          <p class="warning-box__text">{t('donations.unbans.warning2')}</p>
        </div>
      </div>

      <!-- Ticket CTA -->
      <div class="unbans-foot">
        <span class="unbans-foot__hint">{t('pav.claimInstructions')}</span>
        <a
          href={DISCORD_URL}
          target="_blank"
          rel="noopener noreferrer"
          class="channel-btn channel-btn--discord"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
          </svg>
          <span>{t('pav.openTicketAction')}</span>
        </a>
      </div>
    </GlassCard>
  </section>
</div>

<style>
  /* ==========================================================================
     Donations Pavilion Styles — Horizon Glass Theme
     ========================================================================== */

  .pavilion-page {
    display: flex;
    flex-direction: column;
    gap: 2.25rem;
    padding: 1.5rem 2rem 4rem;
    max-width: 1400px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* 1. Hero Showcase */
  .pavilion-hero {
    position: relative;
    border-radius: var(--radius-xl);
    background: linear-gradient(135deg, rgba(245, 158, 11, 0.08) 0%, rgba(16, 185, 129, 0.06) 50%, rgba(6, 182, 212, 0.04) 100%);
    border: 1px solid var(--border);
    padding: 2.5rem 2.25rem;
    overflow: hidden;
    box-shadow: var(--shadow-md);
  }

  .pavilion-hero::before {
    content: '';
    position: absolute;
    top: -40%;
    right: -10%;
    width: 480px;
    height: 480px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(255, 215, 0, 0.12) 0%, rgba(16, 185, 129, 0.06) 50%, transparent 70%);
    filter: blur(48px);
    pointer-events: none;
  }

  .pavilion-hero__inner {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .pavilion-hero__badge-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .pavilion-hero__tagline {
    font-family: var(--font-mono-retro);
    font-size: 0.62rem;
    letter-spacing: 0.08em;
    color: var(--muted);
  }

  .pavilion-hero__title {
    margin: 0;
    font-size: 2.1rem;
    font-weight: 800;
    line-height: 1.2;
    letter-spacing: -0.02em;
  }

  .pavilion-hero__pitch {
    margin: 0.5rem 0 0;
    font-size: 1.02rem;
    line-height: 1.6;
    color: var(--muted-strong);
    max-width: 860px;
  }

  .pavilion-hero__capsules {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin-top: 0.5rem;
  }

  .capsule {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.9rem;
    border-radius: var(--radius-pill);
    background: var(--surface-up);
    border: 1px solid var(--border);
    font-size: 0.84rem;
    font-weight: 600;
    color: var(--text);
  }

  .capsule__icon {
    font-size: 0.95rem;
  }

  /* 2. Sections & Headers */
  .pavilion-section {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .section-header {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .section-title {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 700;
    color: var(--text);
    letter-spacing: -0.01em;
  }

  .section-subtitle {
    margin: 0;
    font-size: 0.9rem;
    color: var(--muted);
    line-height: 1.5;
  }

  /* Support Channels Grid */
  .channels-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
    gap: 1.25rem;
  }

  :global(.channel-card) {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 1.25rem;
    padding: 1.5rem;
    border-radius: var(--radius-lg);
    transition: transform var(--dur-fast) ease, border-color var(--dur-fast) ease;
  }

  :global(.channel-card:hover) {
    transform: translateY(-2px);
    border-color: rgba(255, 255, 255, 0.2);
  }

  :global(.channel-card--paypal:hover) {
    border-color: rgba(16, 185, 129, 0.4);
  }

  :global(.channel-card--tebex:hover) {
    border-color: rgba(6, 182, 212, 0.4);
  }

  :global(.channel-card--discord:hover) {
    border-color: rgba(168, 85, 247, 0.4);
  }

  .channel-card__head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .channel-card__brand {
    display: flex;
    align-items: center;
    gap: 0.85rem;
  }

  .channel-card__icon-box {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .channel-card__icon-box--paypal {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
    border: 1px solid rgba(16, 185, 129, 0.3);
  }

  .channel-card__icon-box--tebex {
    background: rgba(6, 182, 212, 0.15);
    color: #06b6d4;
    border: 1px solid rgba(6, 182, 212, 0.3);
  }

  .channel-card__icon-box--discord {
    background: rgba(168, 85, 247, 0.15);
    color: #a855f7;
    border: 1px solid rgba(168, 85, 247, 0.3);
  }

  .channel-card__title {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 700;
    color: var(--text);
  }

  .channel-card__url {
    font-size: 0.8rem;
    color: var(--muted);
    font-family: var(--font-mono);
  }

  .channel-card__desc {
    margin: 0;
    font-size: 0.88rem;
    line-height: 1.55;
    color: var(--muted-strong);
    flex: 1;
  }

  .channel-card__actions {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-top: 0.5rem;
  }

  .channel-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.55rem 1.1rem;
    border-radius: var(--radius-md);
    font-size: 0.88rem;
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
    transition: filter var(--dur-fast) ease, transform var(--dur-fast) ease;
    flex: 1;
  }

  .channel-btn:hover {
    filter: brightness(1.1);
    transform: translateY(-1px);
  }

  .channel-btn--primary {
    background: var(--accent);
    color: var(--text-inverse);
    border: 1px solid transparent;
  }

  .channel-btn--secondary {
    background: rgba(6, 182, 212, 0.2);
    color: #38bdf8;
    border: 1px solid rgba(6, 182, 212, 0.4);
  }

  .channel-btn--discord {
    background: rgba(168, 85, 247, 0.2);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.4);
  }

  /* 3. Ranks Catalog & Toolbar */
  .ranks-toolbar {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .toolbar-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

  .category-tabs {
    display: flex;
    align-items: center;
    background: var(--surface-up-solid);
    padding: 0.25rem;
    border-radius: var(--radius-lg);
    border: 1px solid var(--border);
    gap: 0.25rem;
  }

  .tab-btn {
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 0.86rem;
    font-weight: 600;
    padding: 0.45rem 0.9rem;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: all var(--dur-fast) ease;
  }

  .tab-btn:hover {
    color: var(--text);
  }

  .tab-btn--active {
    background: var(--card-bg);
    color: var(--text);
    box-shadow: var(--shadow-sm);
  }

  .search-wrap {
    position: relative;
    display: flex;
    align-items: center;
    min-width: 280px;
  }

  .search-icon {
    position: absolute;
    left: 0.85rem;
    color: var(--muted);
    pointer-events: none;
  }

  .search-input {
    width: 100%;
    padding: 0.5rem 2rem 0.5rem 2.3rem;
    background: var(--card-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--text);
    font-size: 0.86rem;
    outline: none;
    transition: border-color var(--dur-fast) ease;
  }

  .search-input:focus {
    border-color: var(--accent);
  }

  .clear-btn {
    position: absolute;
    right: 0.6rem;
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 1.1rem;
    cursor: pointer;
  }

  .disclaimer-banner {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.85rem 1.1rem;
    background: var(--card-bg-soft);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .disclaimer-banner__icon {
    font-size: 1.1rem;
    flex-shrink: 0;
  }

  .disclaimer-banner__text {
    margin: 0;
    font-size: 0.84rem;
    line-height: 1.5;
    color: var(--muted-strong);
  }

  .empty-ranks {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    padding: 3rem 1rem;
    text-align: center;
  }

  .empty-ranks__text {
    margin: 0;
    color: var(--muted);
    font-size: 0.95rem;
  }

  /* Ranks Grid */
  .ranks-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(290px, 1fr));
    gap: 1.25rem;
  }

  :global(.rank-card) {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 1.5rem;
    border-radius: var(--radius-lg);
    position: relative;
    transition: transform var(--dur-fast) ease, box-shadow var(--dur-fast) ease;
  }

  :global(.rank-card:hover) {
    transform: translateY(-3px);
  }

  :global(.rank-card--featured) {
    border: 1px solid rgba(255, 215, 0, 0.45) !important;
    background: linear-gradient(180deg, rgba(255, 215, 0, 0.05) 0%, var(--card-bg) 60%) !important;
    box-shadow: 0 8px 32px rgba(255, 215, 0, 0.12);
  }

  :global(.rank-card--diamond) {
    border-top: 2px solid #38bdf8;
  }

  :global(.rank-card--emerald) {
    border-top: 2px solid #10b981;
  }

  :global(.rank-card--gold) {
    border-top: 2px solid #f59e0b;
  }

  :global(.rank-card--purple) {
    border-top: 2px solid #a855f7;
  }

  :global(.rank-card--amber) {
    border-top: 2px solid #f97316;
  }

  .rank-card__head {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .rank-card__badge-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .rank-card__identity {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }

  .rank-card__name {
    margin: 0;
    font-size: 1.4rem;
    font-weight: 800;
    color: var(--text);
  }

  .rank-card__price {
    font-family: var(--font-mono-retro);
    font-size: 0.78rem;
    font-weight: 700;
    color: var(--accent-gold);
  }

  .rank-card__tagline {
    margin: 0;
    font-size: 0.84rem;
    color: var(--muted);
    line-height: 1.4;
  }

  .rank-card__divider {
    height: 1px;
    background: var(--border);
    margin: 1rem 0;
    opacity: 0.6;
  }

  .rank-card__body {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    flex: 1;
  }

  .rank-card__perks-label {
    font-size: 0.76rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
  }

  .perk-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .perk-item {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    font-size: 0.86rem;
    line-height: 1.45;
    color: var(--muted-strong);
  }

  .perk-bullet {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    margin-top: 0.45rem;
    flex-shrink: 0;
  }

  .perk-text {
    flex: 1;
  }

  .rank-card__foot {
    margin-top: 1.25rem;
  }

  .rank-cta-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.6rem 1rem;
    border-radius: var(--radius-md);
    background: var(--surface-up);
    border: 1px solid var(--border);
    color: var(--text);
    font-size: 0.86rem;
    font-weight: 700;
    text-decoration: none;
    transition: all var(--dur-fast) ease;
    box-sizing: border-box;
  }

  .rank-cta-btn:hover {
    background: var(--rank-accent, var(--accent));
    color: #ffffff;
    border-color: transparent;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
  }

  /* 4. Unbans Transparency Card */
  :global(.unbans-card) {
    padding: 1.85rem;
    border-radius: var(--radius-xl);
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }

  .unbans-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .unbans-badge-row {
    margin-bottom: 0.4rem;
  }

  .unbans-title {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 700;
    color: var(--text);
  }

  .unbans-subtitle {
    margin: 0.25rem 0 0;
    font-size: 0.88rem;
    color: var(--muted);
  }

  .unbans-intro {
    margin: 0;
    font-size: 0.92rem;
    color: var(--muted-strong);
  }

  .unbans-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 1rem;
  }

  .unban-tier-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1.25rem;
    background: var(--surface-up-solid);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .unban-tier-card__level {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .unban-tier-card__chip {
    font-family: var(--font-mono-retro);
    font-size: 0.65rem;
    color: var(--accent-gold);
  }

  .unban-tier-card__label {
    font-size: 0.9rem;
    font-weight: 600;
    color: var(--text);
  }

  .unban-tier-card__price {
    font-family: var(--font-mono-retro);
    font-size: 0.84rem;
    font-weight: 700;
    color: var(--accent);
  }

  .unbans-warnings {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .warning-box {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.85rem 1.1rem;
    border-radius: var(--radius-md);
    font-size: 0.84rem;
    line-height: 1.5;
  }

  .warning-box--info {
    background: rgba(6, 182, 212, 0.08);
    border: 1px solid rgba(6, 182, 212, 0.25);
    color: var(--text);
  }

  .warning-box--strict {
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: #fca5a5;
  }

  .warning-box__icon {
    font-size: 1rem;
    flex-shrink: 0;
  }

  .warning-box__text {
    margin: 0;
  }

  .unbans-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--border);
  }

  .unbans-foot__hint {
    font-size: 0.86rem;
    color: var(--muted);
    max-width: 680px;
    line-height: 1.45;
  }

  /* Responsive Adjustments */
  @media (max-width: 768px) {
    .pavilion-page {
      padding: 1rem 1rem 3rem;
      gap: 1.75rem;
    }

    .pavilion-hero {
      padding: 1.75rem 1.25rem;
    }

    .pavilion-hero__title {
      font-size: 1.6rem;
    }

    .channels-grid,
    .ranks-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
