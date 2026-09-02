<!--
  ============================================================================
  SettingsDashboard.svelte — Horizon Glass System & Engine Control Center (#/settings)
  ============================================================================
  Complete control center for launcher preferences, JVM memory allocation,
  download concurrency, custom Java JDK runtime path, feature toggles (AOT,
  Discord RPC, Fullbright), system storage paths, and engine diagnostics.
-->
<script lang="ts">
  import {
    getConfig,
    patchConfig,
    getJvm,
    openFolder,
    API_BASE,
  } from '../lib/api';
  import { theme as themeStore, resolveTheme } from '../lib/theme.svelte';
  import { lang as langStore, t } from '../lib/i18n.svelte';
  import { health as healthStore } from '../lib/stores';
  import { pushToast } from '../lib/toast.svelte';
  import { useCopy } from '../lib/useCopy.svelte';
  import { memLabel } from '../lib/format';
  import { flyY, fade, scalePop } from '../lib/motion';
  import type { AppConfig, JvmInfo, Theme } from '../lib/types';

  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import Field from '../components/Field.svelte';
  import ThemeToggle from '../components/ThemeToggle.svelte';
  import LanguageToggle from '../components/LanguageToggle.svelte';
  import GradientText from '../components/GradientText.svelte';

  /* ==========================================================================
     State & Initialization
     ========================================================================== */
  let config = $state<AppConfig | null>(null);
  let error = $state('');

  let savingField = $state<string>('');
  let savingAll = $state(false);

  // Form states ($state)
  let memory = $state<number>(3072);
  let concurrency = $state<number>(6);
  let jdkOverride = $state<string>('');
  let aotAutoTrain = $state<boolean>(true);
  let fastBoot = $state<boolean>(false);
  let fullbright = $state<boolean>(false);
  let discordEnabled = $state<boolean>(true);

  // JDK Probe states ($state)
  let probing = $state(false);
  let probeMsg = $state('');
  let probeOk = $state(false);
  let jvm = $state<JvmInfo | null>(null);

  // Diagnostics check state ($state)
  let checkingHealth = $state(false);

  // Copy helper for data directory
  const dataDirCopy = useCopy(2000);

  /* ==========================================================================
     Derived Computations & Dirty Checking
     ========================================================================== */
  const memoryNum = $derived(Number(memory));
  const memoryError = $derived.by(() => {
    if (memory === null || memory === undefined || isNaN(memoryNum)) {
      return t('settings.memory.invalid');
    }
    if (!Number.isInteger(memoryNum) || memoryNum < 512 || memoryNum > 65536) {
      return t('settings.memory.invalid');
    }
    return '';
  });

  const memoryGbFormatted = $derived((memoryNum / 1024).toFixed(1).replace(/\.0$/, ''));

  // Dirty state per field vs server config
  const isMemoryDirty = $derived(
    config !== null && memoryNum !== config.default_memory_mb && !memoryError
  );
  const isConcurrencyDirty = $derived(
    config !== null && Number(concurrency) !== config.download_concurrency
  );
  const isJdkDirty = $derived(
    config !== null && (jdkOverride || '').trim() !== (config.jdk_path_override || '').trim()
  );
  const isAotDirty = $derived(
    config !== null && aotAutoTrain !== (config.aot_auto_train ?? true)
  );
  const isFastBootDirty = $derived(
    config !== null && fastBoot !== (config.fast_boot ?? false)
  );
  const isFullbrightDirty = $derived(
    config !== null && fullbright !== (config.fullbright_on_launch ?? false)
  );
  const isDiscordDirty = $derived(
    config !== null && discordEnabled !== (config.discord_enabled ?? true)
  );

  const isAnyDirty = $derived(
    isMemoryDirty || isConcurrencyDirty || isJdkDirty || isAotDirty || isFastBootDirty || isFullbrightDirty || isDiscordDirty
  );

  const healthData = $derived($healthStore.value);
  const isEngineOnline = $derived(Boolean(healthData?.ok));

  /* ==========================================================================
     Data Fetching
     ========================================================================== */
  async function loadConfig() {
    error = '';
    try {
      config = await getConfig();
      resetToConfig(config);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('dash.saveError') + ': ' + error });
    }

  }

  function resetToConfig(cfg: AppConfig | null) {
    if (!cfg) return;
    memory = cfg.default_memory_mb ?? 3072;
    concurrency = cfg.download_concurrency ?? 6;
    jdkOverride = cfg.jdk_path_override ?? '';
    aotAutoTrain = cfg.aot_auto_train ?? true;
    fastBoot = cfg.fast_boot ?? false;
    fullbright = cfg.fullbright_on_launch ?? false;
    discordEnabled = cfg.discord_enabled ?? true;
  }

  function handleResetAll() {
    if (!config) return;
    resetToConfig(config);
    pushToast({ kind: 'info', text: t('dash.reloading') });
  }

  $effect(() => {
    void loadConfig();
  });

  /* ==========================================================================
     Field-Level Save Handlers
     ========================================================================== */
  async function saveMemory() {
    if (memoryError) return;
    const mb = Number(memory);
    if (!Number.isInteger(mb) || mb < 512 || mb > 65536) return;
    savingField = 'memory';
    try {
      config = await patchConfig({ default_memory_mb: mb });
      pushToast({ kind: 'ok', text: t('settings.flash.memory') });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveConcurrency() {
    const n = Math.max(1, Math.min(16, Math.round(concurrency) || 1));
    concurrency = n;
    savingField = 'concurrency';
    try {
      config = await patchConfig({ download_concurrency: n });
      pushToast({ kind: 'ok', text: t('settings.flash.concurrency') });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveJdkOverride() {
    savingField = 'jdk';
    try {
      config = await patchConfig({ jdk_path_override: (jdkOverride || '').trim() });
      pushToast({ kind: 'ok', text: t('settings.flash.jdk') });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveAotAutoTrain() {
    savingField = 'aot';
    try {
      config = await patchConfig({ aot_auto_train: aotAutoTrain });
      pushToast({
        kind: 'ok',
        text: t(aotAutoTrain ? 'settings.flash.aotOn' : 'settings.flash.aotOff'),
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveFastBoot() {
    savingField = 'fastboot';
    try {
      config = await patchConfig({ fast_boot: fastBoot });
      pushToast({
        kind: 'ok',
        text: t(fastBoot ? 'settings.flash.fastBootOn' : 'settings.flash.fastBootOff'),
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveFullbright() {
    savingField = 'fullbright';
    try {
      config = await patchConfig({ fullbright_on_launch: fullbright });
      pushToast({
        kind: 'ok',
        text: t(fullbright ? 'settings.flash.fullbrightOn' : 'settings.flash.fullbrightOff'),
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  async function saveDiscordEnabled() {
    savingField = 'discord';
    try {
      config = await patchConfig({ discord_enabled: discordEnabled });
      pushToast({
        kind: 'ok',
        text: t(discordEnabled ? 'settings.flash.discordOn' : 'settings.flash.discordOff'),
      });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: msg });
    } finally {
      savingField = '';
    }
  }

  /* ==========================================================================
     Batch Save All
     ========================================================================== */
  async function handleSaveAll() {
    if (!config || memoryError) return;
    savingAll = true;
    try {
      const patch: Partial<AppConfig> = {};
      if (isMemoryDirty) patch.default_memory_mb = Number(memory);
      if (isConcurrencyDirty) patch.download_concurrency = Math.max(1, Math.min(16, Math.round(concurrency) || 1));
      if (isJdkDirty) patch.jdk_path_override = (jdkOverride || '').trim();
      if (isAotDirty) patch.aot_auto_train = aotAutoTrain;
      if (isFastBootDirty) patch.fast_boot = fastBoot;
      if (isFullbrightDirty) patch.fullbright_on_launch = fullbright;
      if (isDiscordDirty) patch.discord_enabled = discordEnabled;

      config = await patchConfig(patch);
      pushToast({ kind: 'ok', text: t('dash.allSaved') });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('dash.saveError') + ': ' + msg });
    } finally {
      savingAll = false;
    }
  }

  /* ==========================================================================
     JDK Probe Test
     ========================================================================== */
  async function probe() {
    probing = true;
    probeMsg = '';
    probeOk = false;
    try {
      jvm = await getJvm();
      probeMsg = `OK — ${jvm.version} · ${jvm.vendor} (${jvm.source})`;
      probeOk = true;
      pushToast({ kind: 'ok', text: t('dash.jdk.statusOk') + ` (${jvm.version})` });
    } catch (e: unknown) {
      probeMsg = e instanceof Error ? e.message : String(e);
      probeOk = false;
      pushToast({ kind: 'err', text: t('dash.jdk.statusErr') + ': ' + probeMsg });
    } finally {
      probing = false;
    }
  }

  /* ==========================================================================
     Theme & Language Handlers
     ========================================================================== */
  function setThemeValue(value: Theme) {
    themeStore.set(value);
  }

  /* ==========================================================================
     System Folder Openers
     ========================================================================== */
  async function handleOpenFolder(subPath = '') {
    if (!config?.data_dir) return;
    try {
      const cleanRoot = config.data_dir.replace(/[/\\]+$/, '');
      const fullPath = subPath
        ? `${cleanRoot}${cleanRoot.includes('\\') ? '\\' : '/'}${subPath}`
        : cleanRoot;
      await openFolder(fullPath);
    } catch (e: unknown) {
      pushToast({ kind: 'err', text: t('settings.openFolderError') });
    }
  }

  /* ==========================================================================
     Diagnostics Health Recheck
     ========================================================================== */
  async function recheckHealth() {
    checkingHealth = true;
    try {
      await $healthStore.refresh();
      pushToast({ kind: 'ok', text: t('dash.health.online') });
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('dash.health.offline') + ': ' + msg });
    } finally {
      checkingHealth = false;
    }
  }

  function navigateTo(hash: string) {
    if (typeof window !== 'undefined') {
      window.location.hash = hash;
    }
  }
</script>

<svelte:head>
  <title>{t('dash.tag')}</title>
</svelte:head>

<div class="settings-dashboard">
  <!-- Top Hero Header -->
  <header class="settings-hero">
    <div class="settings-hero__content">
      <div class="settings-hero__badge-row">
        <Badge variant="accent" size="sm" dot={true}>
          {t('dash.badge')}
        </Badge>
        {#if isEngineOnline}
          <span class="settings-hero__online-pill">
            <span class="status-dot status-dot--online"></span>
            {t('dash.health.online')}
          </span>
        {/if}
      </div>

      <h1 class="settings-hero__title">
        <GradientText variant="emerald" size="2xl">
          {t('dash.title')}
        </GradientText>
      </h1>

      <p class="settings-hero__subtitle">
        {t('dash.subtitle')}
      </p>
    </div>

    <!-- Quick Actions Dock -->
    <div class="settings-hero__actions">
      {#if isAnyDirty}
        <div class="dirty-dock" in:scalePop={{ duration: 180 }}>
          <Badge variant="warn" dot={true} size="md">
            {t('dash.unsavedBadge')}
          </Badge>
          <Btn
            variant="primary"
            size="md"
            loading={savingAll}
            disabled={!!memoryError}
            onclick={handleSaveAll}
          >
            {#snippet icon()}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/>
                <polyline points="17 21 17 13 7 13 7 21"/>
                <polyline points="7 3 7 8 15 8"/>
              </svg>
            {/snippet}
            {savingAll ? t('dash.savingAll') : t('dash.saveAll')}
          </Btn>
          <Btn variant="ghost" size="md" onclick={handleResetAll} title={t('dash.resetField')}>
            {t('dash.resetField')}
          </Btn>
        </div>
      {/if}
    </div>
  </header>

  {#if error}
    <div class="error-banner" in:flyY={{ y: -8, duration: 200 }}>
      <div class="error-banner__icon">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10"/>
          <line x1="12" y1="8" x2="12" y2="12"/>
          <line x1="12" y1="16" x2="12.01" y2="16"/>
        </svg>
      </div>
      <div class="error-banner__text">
        <strong>{t('dash.saveError')}:</strong> {error}
      </div>
      <Btn variant="secondary" size="sm" onclick={loadConfig}>
        {t('servers.retry')}
      </Btn>
    </div>
  {/if}

  <!-- Main Settings Grid Layout -->
  <div class="settings-content-grid">
    <!-- ======================================================================
         SECTION 1: APPEARANCE & LANGUAGE
         ====================================================================== -->
    <section class="settings-section" aria-labelledby="section-appearance">
      <div class="section-header">
        <div class="section-header__meta">
          <h2 id="section-appearance" class="section-header__title">
            {t('dash.appearance.title')}
          </h2>
          <p class="section-header__desc">{t('dash.appearance.subtitle')}</p>
        </div>
      </div>

      <div class="cards-duo">
        <!-- Theme Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <span class="card-head__title">{t('dash.appearance.themeLabel')}</span>
              <span class="card-head__desc">{t('dash.appearance.themeDesc')}</span>
            </div>
            <ThemeToggle />
          </div>

          <!-- Segmented Theme Choices -->
          <div class="theme-segmented-grid">
            {#each (['dark', 'light', 'system'] as const) as opt}
              <button
                type="button"
                class="theme-choice-btn"
                class:theme-choice-btn--active={themeStore.value === opt}
                onclick={() => setThemeValue(opt)}
              >
                <div class="theme-choice-btn__icon">
                  {#if opt === 'dark'}
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>
                    </svg>
                  {:else if opt === 'light'}
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <circle cx="12" cy="12" r="5"/>
                      <line x1="12" y1="1" x2="12" y2="3"/>
                      <line x1="12" y1="21" x2="12" y2="23"/>
                      <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/>
                      <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/>
                      <line x1="1" y1="12" x2="3" y2="12"/>
                      <line x1="21" y1="12" x2="23" y2="12"/>
                      <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/>
                      <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>
                    </svg>
                  {:else}
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
                      <line x1="8" y1="21" x2="16" y2="21"/>
                      <line x1="12" y1="17" x2="12" y2="21"/>
                    </svg>
                  {/if}
                </div>
                <span class="theme-choice-btn__label">
                  {opt === 'dark'
                    ? t('settings.theme.dark')
                    : opt === 'light'
                    ? t('settings.theme.light')
                    : t('settings.theme.system')}
                </span>
                {#if themeStore.value === opt}
                  <span class="choice-active-dot"></span>
                {/if}
              </button>
            {/each}
          </div>

          <!-- System Theme Explainer Note -->
          <div class="explainer-box">
            <div class="explainer-box__icon">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="10"/>
                <line x1="12" y1="16" x2="12" y2="12"/>
                <line x1="12" y1="8" x2="12.01" y2="8"/>
              </svg>
            </div>
            <p class="explainer-box__text">
              {t('dash.appearance.systemDesc')}
            </p>
          </div>

          <!-- Visual Theme Preview Stage -->
          <div
            class="theme-preview-stage"
            data-theme={resolveTheme(themeStore.value)}
          >
            <div class="theme-preview-stage__topbar">
              <span class="preview-dot preview-dot--red"></span>
              <span class="preview-dot preview-dot--yellow"></span>
              <span class="preview-dot preview-dot--green"></span>
              <span class="preview-title">Espectral Client</span>
            </div>
            <div class="theme-preview-stage__body">
              <div class="preview-chip">{resolveTheme(themeStore.value).toUpperCase()} MODE</div>
              <div class="preview-lines">
                <div class="preview-line preview-line--long"></div>
                <div class="preview-line preview-line--short"></div>
              </div>
            </div>
          </div>
        </GlassCard>

        <!-- Language Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <span class="card-head__title">{t('dash.appearance.langLabel')}</span>
              <span class="card-head__desc">{t('dash.appearance.langDesc')}</span>
            </div>
            <LanguageToggle variant="segmented" />
          </div>

          <div class="language-showcase">
            <div
              class="lang-tile"
              class:lang-tile--active={langStore.value === 'es'}
              onclick={() => langStore.set('es')}
              role="button"
              tabindex="0"
              onkeydown={(e) => e.key === 'Enter' && langStore.set('es')}
            >
              <div class="lang-tile__flag">🇪🇸</div>
              <div class="lang-tile__info">
                <span class="lang-tile__name">Español</span>
                <span class="lang-tile__sub">Predeterminado (Espectral)</span>
              </div>
              {#if langStore.value === 'es'}
                <Badge variant="accent" size="sm">ACTIVO</Badge>
              {/if}
            </div>

            <div
              class="lang-tile"
              class:lang-tile--active={langStore.value === 'en'}
              onclick={() => langStore.set('en')}
              role="button"
              tabindex="0"
              onkeydown={(e) => e.key === 'Enter' && langStore.set('en')}
            >
              <div class="lang-tile__flag">🇬🇧</div>
              <div class="lang-tile__info">
                <span class="lang-tile__name">English</span>
                <span class="lang-tile__sub">International Standard</span>
              </div>
              {#if langStore.value === 'en'}
                <Badge variant="accent" size="sm">ACTIVE</Badge>
              {/if}
            </div>
          </div>
        </GlassCard>
      </div>
    </section>

    <!-- ======================================================================
         SECTION 2: ENGINE, MEMORY & JVM
         ====================================================================== -->
    <section class="settings-section" aria-labelledby="section-engine">
      <div class="section-header">
        <div class="section-header__meta">
          <h2 id="section-engine" class="section-header__title">
            {t('dash.engine.title')}
          </h2>
          <p class="section-header__desc">{t('dash.engine.subtitle')}</p>
        </div>
      </div>

      <div class="cards-grid">
        <!-- Memory Allocation Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <div class="card-head__title-row">
                <span class="card-head__title">{t('dash.memory.title')}</span>
                {#if isMemoryDirty}
                  <Badge variant="warn" size="sm" dot={true}>
                    {t('dash.unsavedBadge')}
                  </Badge>
                {/if}
              </div>
              <span class="card-head__desc">
                {t('settings.memoryDefault', { mem: memLabel(memoryNum) })} — {t('dash.memory.desc')}
              </span>
            </div>
            <div class="card-head__ram-badge">
              <span class="ram-badge__val">{memoryGbFormatted}</span>
              <span class="ram-badge__unit">GB</span>
            </div>
          </div>

          <Field
            label={t('dash.memory.sliderLabel')}
            hint={t('settings.memoryHint')}
            error={memoryError}
          >
            <!-- Preset Quick Chips -->
            <div class="preset-chips-row">
              {#each [2048, 3072, 4096, 6144, 8192, 12288, 16384] as mb}
                <button
                  type="button"
                  class="preset-chip"
                  class:preset-chip--active={memory === mb}
                  onclick={() => (memory = mb)}
                >
                  {(mb / 1024).toFixed(0)} GB
                </button>
              {/each}
            </div>

            <!-- Slider + Custom Number Row -->
            <div class="slider-control-group">
              <div class="slider-wrapper">
                <input
                  type="range"
                  class="range-slider"
                  min="1024"
                  max="16384"
                  step="512"
                  bind:value={memory}
                  aria-label={t('dash.memory.sliderLabel')}
                />
                <div class="slider-track-fill" style:width="{Math.min(100, Math.max(0, ((memoryNum - 1024) / (16384 - 1024)) * 100))}%"></div>
              </div>

              <div class="custom-num-dock">
                <input
                  class="input input--num"
                  type="number"
                  min="512"
                  max="65536"
                  step="512"
                  bind:value={memory}
                  placeholder={t('settings.memory.custom')}
                  aria-label={t('settings.memory.custom')}
                />
                <span class="input-unit">MB</span>

                <Btn
                  variant={isMemoryDirty ? 'primary' : 'secondary'}
                  size="sm"
                  loading={savingField === 'memory'}
                  disabled={savingField === 'memory' || !!memoryError}
                  onclick={saveMemory}
                >
                  {savingField === 'memory' ? t('common.saving') : t('dash.memory.saveBtn')}
                </Btn>
              </div>
            </div>
          </Field>
        </GlassCard>

        <!-- Download Concurrency Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <div class="card-head__title-row">
                <span class="card-head__title">{t('dash.downloads.title')}</span>
                {#if isConcurrencyDirty}
                  <Badge variant="warn" size="sm" dot={true}>
                    {t('dash.unsavedBadge')}
                  </Badge>
                {/if}
              </div>
              <span class="card-head__desc">
                {t('dash.downloads.threads', { n: concurrency })} — {t('settings.downloadConcurrencyHint')}
              </span>
            </div>
          </div>

          <Field
            label={t('settings.downloadConcurrency')}
            hint={t('dash.downloads.desc')}
          >
            <div class="preset-chips-row">
              {#each [2, 4, 6, 8, 12, 16] as n}
                <button
                  type="button"
                  class="preset-chip"
                  class:preset-chip--active={concurrency === n}
                  onclick={() => (concurrency = n)}
                >
                  {n} {n === 1 ? 'thread' : 'threads'}
                </button>
              {/each}
            </div>

            <div class="slider-control-group">
              <div class="slider-wrapper">
                <input
                  type="range"
                  class="range-slider"
                  min="1"
                  max="16"
                  step="1"
                  bind:value={concurrency}
                  aria-label={t('settings.downloadConcurrency')}
                />
                <div class="slider-track-fill" style:width="{Math.min(100, Math.max(0, ((concurrency - 1) / (16 - 1)) * 100))}%"></div>
              </div>

              <div class="custom-num-dock">
                <input
                  class="input input--num"
                  type="number"
                  min="1"
                  max="16"
                  step="1"
                  bind:value={concurrency}
                  aria-label={t('settings.downloadConcurrency')}
                />
                <span class="input-unit">hilos</span>

                <Btn
                  variant={isConcurrencyDirty ? 'primary' : 'secondary'}
                  size="sm"
                  loading={savingField === 'concurrency'}
                  disabled={savingField === 'concurrency'}
                  onclick={saveConcurrency}
                >
                  {savingField === 'concurrency' ? t('common.saving') : t('dash.downloads.saveBtn')}
                </Btn>
              </div>
            </div>
          </Field>
        </GlassCard>

        <!-- Custom JDK Runtime Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <div class="card-head__title-row">
                <span class="card-head__title">{t('dash.jdk.title')}</span>
                {#if isJdkDirty}
                  <Badge variant="warn" size="sm" dot={true}>
                    {t('dash.unsavedBadge')}
                  </Badge>
                {/if}
              </div>
              <span class="card-head__desc">
                {t('dash.jdk.desc')}
              </span>
            </div>
          </div>

          <Field
            label={t('settings.jdkPath')}
            hint={t('settings.jdkPathHint')}
          >
            <div class="input-with-actions">
              <input
                class="input"
                type="text"
                bind:value={jdkOverride}
                placeholder={t('settings.jdkPlaceholder')}
                aria-label={t('settings.jdkPath')}
              />
              <div class="action-buttons-row">
                <Btn
                  variant={isJdkDirty ? 'primary' : 'secondary'}
                  size="sm"
                  loading={savingField === 'jdk'}
                  disabled={savingField === 'jdk'}
                  onclick={saveJdkOverride}
                >
                  {savingField === 'jdk' ? t('common.saving') : t('dash.jdk.saveBtn')}
                </Btn>
                <Btn
                  variant="ghost"
                  size="sm"
                  loading={probing}
                  disabled={probing}
                  onclick={probe}
                >
                  {#snippet icon()}
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <polygon points="5 3 19 12 5 21 5 3"/>
                    </svg>
                  {/snippet}
                  {probing ? t('dash.jdk.testingBtn') : t('dash.jdk.testBtn')}
                </Btn>
              </div>
            </div>

            <!-- Probe Result Feedback -->
            {#if probeMsg}
              <div
                class="probe-feedback-banner"
                class:probe-feedback-banner--ok={probeOk}
                class:probe-feedback-banner--err={!probeOk}
                in:flyY={{ y: 4, duration: 160 }}
              >
                <div class="probe-feedback-banner__icon">
                  {#if probeOk}
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/>
                      <polyline points="22 4 12 14.01 9 11.01"/>
                    </svg>
                  {:else}
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <circle cx="12" cy="12" r="10"/>
                      <line x1="12" y1="8" x2="12" y2="12"/>
                      <line x1="12" y1="16" x2="12.01" y2="16"/>
                    </svg>
                  {/if}
                </div>
                <span class="probe-feedback-banner__text">{probeMsg}</span>
              </div>
            {/if}

            {#if jvm && probeOk}
              <div class="jvm-telemetry-grid" in:fade={{ duration: 150 }}>
                <div class="jvm-kv">
                  <span class="jvm-k">{t('dash.jdk.versionLabel')}</span>
                  <span class="jvm-v">{jvm.version}</span>
                </div>
                <div class="jvm-kv">
                  <span class="jvm-k">{t('dash.jdk.vendorLabel')}</span>
                  <span class="jvm-v">{jvm.vendor}</span>
                </div>
                <div class="jvm-kv">
                  <span class="jvm-k">{t('dash.jdk.sourceLabel')}</span>
                  <Badge variant="accent" size="sm">{jvm.source}</Badge>
                </div>
                <div class="jvm-kv jvm-kv--full">
                  <span class="jvm-k">{t('dash.jdk.pathLabel')}</span>
                  <code class="code-pill" title={jvm.path}>{jvm.path}</code>
                </div>
              </div>
            {/if}
          </Field>
        </GlassCard>
      </div>
    </section>

    <!-- ======================================================================
         SECTION 3: ENGINE & CLIENT FEATURE TOGGLES
         ====================================================================== -->
    <section class="settings-section" aria-labelledby="section-toggles">
      <div class="section-header">
        <div class="section-header__meta">
          <h2 id="section-toggles" class="section-header__title">
            {t('dash.toggles.title')}
          </h2>
          <p class="section-header__desc">{t('dash.toggles.subtitle')}</p>
        </div>
      </div>

      <div class="toggles-grid">
        <!-- AOT Auto-Train Card -->
        <GlassCard elevation="sm" class="toggle-card">
          <div class="toggle-card__main">
            <div class="toggle-card__info">
              <div class="toggle-card__badge-row">
                <Badge variant="purple" size="sm">{t('dash.aot.badge')}</Badge>
                <span class="speedup-pill">{t('dash.aot.speedup')}</span>
              </div>
              <h3 class="toggle-card__title">{t('dash.aot.title')}</h3>
              <p class="toggle-card__desc">{t('dash.aot.desc')}</p>
            </div>

            <div class="toggle-card__switch">
              <label class="switch-control">
                <input
                  type="checkbox"
                  bind:checked={aotAutoTrain}
                  onchange={saveAotAutoTrain}
                  disabled={savingField === 'aot'}
                />
                <span class="switch-slider"></span>
              </label>
            </div>
          </div>
        </GlassCard>

        <!-- Fast Boot Card (C1-only JIT opt-in) -->
        <GlassCard elevation="sm" class="toggle-card">
          <div class="toggle-card__main">
            <div class="toggle-card__info">
              <div class="toggle-card__badge-row">
                <Badge variant="ok" size="sm">{t('dash.fastboot.badge')}</Badge>
                <span class="speedup-pill">{t('dash.fastboot.speedup')}</span>
              </div>
              <h3 class="toggle-card__title">{t('dash.fastboot.title')}</h3>
              <p class="toggle-card__desc">{t('dash.fastboot.desc')}</p>
            </div>

            <div class="toggle-card__switch">
              <label class="switch-control">
                <input
                  type="checkbox"
                  bind:checked={fastBoot}
                  onchange={saveFastBoot}
                  disabled={savingField === 'fastboot'}
                />
                <span class="switch-slider"></span>
              </label>
            </div>
          </div>
        </GlassCard>

        <!-- Discord Rich Presence Card -->
        <GlassCard elevation="sm" class="toggle-card">
          <div class="toggle-card__main">
            <div class="toggle-card__info">
              <div class="toggle-card__badge-row">
                <Badge variant="accent" size="sm">{t('dash.discord.badge')}</Badge>
              </div>
              <h3 class="toggle-card__title">{t('dash.discord.title')}</h3>
              <p class="toggle-card__desc">{t('dash.discord.desc')}</p>
            </div>

            <div class="toggle-card__switch">
              <label class="switch-control">
                <input
                  type="checkbox"
                  bind:checked={discordEnabled}
                  onchange={saveDiscordEnabled}
                  disabled={savingField === 'discord'}
                />
                <span class="switch-slider"></span>
              </label>
            </div>
          </div>
        </GlassCard>

        <!-- Fullbright On Launch Card -->
        <GlassCard elevation="sm" class="toggle-card">
          <div class="toggle-card__main">
            <div class="toggle-card__info">
              <div class="toggle-card__badge-row">
                <Badge variant="gold" size="sm">{t('dash.fullbright.badge')}</Badge>
              </div>
              <h3 class="toggle-card__title">{t('dash.fullbright.title')}</h3>
              <p class="toggle-card__desc">{t('dash.fullbright.desc')}</p>
            </div>

            <div class="toggle-card__switch">
              <label class="switch-control">
                <input
                  type="checkbox"
                  bind:checked={fullbright}
                  onchange={saveFullbright}
                  disabled={savingField === 'fullbright'}
                />
                <span class="switch-slider"></span>
              </label>
            </div>
          </div>
        </GlassCard>
      </div>
    </section>

    <!-- ======================================================================
         SECTION 4: SYSTEM STORAGE & DIRECTORIES
         ====================================================================== -->
    <section class="settings-section" aria-labelledby="section-data">
      <div class="section-header">
        <div class="section-header__meta">
          <h2 id="section-data" class="section-header__title">
            {t('dash.data.title')}
          </h2>
          <p class="section-header__desc">{t('dash.data.subtitle')}</p>
        </div>
      </div>

      <div class="cards-duo">
        <!-- Data Directory Root Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <span class="card-head__title">{t('dash.data.rootLabel')}</span>
              <span class="card-head__desc">{t('dash.data.rootDesc')}</span>
            </div>
          </div>

          <div class="path-display-card">
            <div class="path-display-card__code">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
              </svg>
              <code title={config?.data_dir || ''}>{config?.data_dir || '—'}</code>
            </div>

            <div class="path-display-card__actions">
              <Btn
                variant="ghost"
                size="sm"
                onclick={() => dataDirCopy.copy(config?.data_dir || '')}
              >
                {#snippet icon()}
                  {#if dataDirCopy.copied}
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <polyline points="20 6 9 17 4 12"/>
                    </svg>
                  {:else}
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/>
                      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
                    </svg>
                  {/if}
                {/snippet}
                {dataDirCopy.copied ? t('dash.data.copied') : t('dash.data.copyPath')}
              </Btn>

              <Btn
                variant="secondary"
                size="sm"
                onclick={() => handleOpenFolder('')}
              >
                {#snippet icon()}
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
                    <polyline points="15 3 21 3 21 9"/>
                    <line x1="10" y1="14" x2="21" y2="3"/>
                  </svg>
                {/snippet}
                {t('dash.data.openRoot')}
              </Btn>
            </div>
          </div>

          <!-- Quick System Subfolder Shortcuts -->
          <div class="subfolders-grid">
            <button
              type="button"
              class="subfolder-chip"
              onclick={() => handleOpenFolder('instances')}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="2" y="7" width="20" height="14" rx="2" ry="2"/>
                <path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16"/>
              </svg>
              <span>{t('dash.data.openInstances')}</span>
            </button>

            <button
              type="button"
              class="subfolder-chip"
              onclick={() => handleOpenFolder('logs')}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                <polyline points="14 2 14 8 20 8"/>
                <line x1="16" y1="13" x2="8" y2="13"/>
                <line x1="16" y1="17" x2="8" y2="17"/>
                <polyline points="10 9 9 9 8 9"/>
              </svg>
              <span>{t('dash.data.openLogs')}</span>
            </button>

            <button
              type="button"
              class="subfolder-chip"
              onclick={() => handleOpenFolder('mods')}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
                <line x1="12" y1="22.08" x2="12" y2="12"/>
              </svg>
              <span>{t('dash.data.openMods')}</span>
            </button>
          </div>
        </GlassCard>

        <!-- Health & Telemetry Diagnostics Card -->
        <GlassCard elevation="sm" class="setting-card">
          <div class="card-head">
            <div class="card-head__title-wrap">
              <span class="card-head__title">{t('dash.health.title')}</span>
              <span class="card-head__desc">{t('dash.health.subtitle')}</span>
            </div>
            <Btn
              variant="ghost"
              size="sm"
              loading={checkingHealth}
              onclick={recheckHealth}
            >
              {#snippet icon()}
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/>
                </svg>
              {/snippet}
              {checkingHealth ? t('dash.health.reconnecting') : t('dash.health.reconnect')}
            </Btn>
          </div>

          <div class="health-status-rows">
            <div class="health-row">
              <span class="health-k">Estado del Engine</span>
              <div class="health-v">
                {#if isEngineOnline}
                  <Badge variant="ok" dot={true} size="sm">
                    {t('dash.health.online')}
                  </Badge>
                {:else}
                  <Badge variant="err" dot={true} size="sm">
                    {t('dash.health.offline')}
                  </Badge>
                {/if}
              </div>
            </div>

            <div class="health-row">
              <span class="health-k">{t('dash.health.version')}</span>
              <span class="health-v health-v--mono">{healthData?.version ?? '—'}</span>
            </div>

            <div class="health-row">
              <span class="health-k">{t('dash.health.apiEndpoint')}</span>
              <span class="health-v health-v--mono">{API_BASE}</span>
            </div>
          </div>
        </GlassCard>
      </div>
    </section>

    <!-- ======================================================================
         SECTION 5: ABOUT & COMMUNITY VIP FOOTER
         ====================================================================== -->
    <section class="settings-section" aria-labelledby="section-about">
      <div class="about-pavilion-card">
        <div class="about-pavilion-card__left">
          <div class="about-badge-row">
            <Badge variant="accent" size="sm">ESPECTRAL ECOSYSTEM</Badge>
          </div>
          <h2 class="about-title">
            <GradientText variant="emerald" size="xl">
              {t('dash.about.title')}
            </GradientText>
          </h2>
          <p class="about-tagline">{t('dash.about.tagline')}</p>

          <div class="about-meta-grid">
            <div class="about-meta-item">
              <span class="about-meta-k">{t('settings.name')}</span>
              <span class="about-meta-v">Espectral Client</span>
            </div>
            <div class="about-meta-item">
              <span class="about-meta-k">{t('settings.version')}</span>
              <span class="about-meta-v">{healthData?.version ?? '2.4.0-horizon'}</span>
            </div>
            <div class="about-meta-item">
              <span class="about-meta-k">{t('dash.about.stack')}</span>
              <span class="about-meta-v">{t('dash.about.stackDetails')}</span>
            </div>
            <div class="about-meta-item">
              <span class="about-meta-k">{t('settings.web')}</span>
              <a href="https://espectral.es" target="_blank" rel="noopener" class="about-link">
                espectral.es
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
                  <polyline points="15 3 21 3 21 9"/>
                  <line x1="10" y1="14" x2="21" y2="3"/>
                </svg>
              </a>
            </div>
          </div>
        </div>

        <!-- Right Side: Donations Pavilion Bridge Chip -->
        <div class="about-pavilion-card__right">
          <div class="vip-banner-box">
            <div class="vip-banner-box__icon">
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
              </svg>
            </div>
            <div class="vip-banner-box__text">
              <span class="vip-banner-box__title">{t('dash.about.supportBanner')}</span>
              <span class="vip-banner-box__sub">{t('dash.about.rankPerks')}</span>
            </div>
            <Btn
              variant="primary"
              size="md"
              onclick={() => navigateTo('#/donaciones')}
            >
              {#snippet icon()}
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
                </svg>
              {/snippet}
              {t('dash.about.viewDonations')}
            </Btn>
          </div>
        </div>
      </div>
    </section>
  </div>
</div>

<style>
  /* ==========================================================================
     Layout & Base Container
     ========================================================================== */
  .settings-dashboard {
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
    padding: 2rem 2.5rem 4rem;
    max-width: 1400px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* ==========================================================================
     Hero Header & Action Dock
     ========================================================================== */
  .settings-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1.5rem;
    padding-bottom: 1.25rem;
    border-bottom: 1px solid var(--border);
  }

  .settings-hero__content {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .settings-hero__badge-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .settings-hero__online-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: var(--muted);
    font-weight: 500;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--muted);
  }

  .status-dot--online {
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent);
  }

  .settings-hero__title {
    margin: 0;
    line-height: 1.1;
  }

  .settings-hero__subtitle {
    margin: 0;
    font-size: 0.95rem;
    color: var(--muted);
    max-width: 650px;
    line-height: 1.5;
  }

  .settings-hero__actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .dirty-dock {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: var(--surface-solid);
    border: 1px solid var(--border-focus);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md);
  }

  /* ==========================================================================
     Error Banner
     ========================================================================== */
  .error-banner {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.875rem 1.25rem;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: var(--radius-md);
    color: var(--accent-red);
  }

  .error-banner__icon {
    display: flex;
    flex-shrink: 0;
  }

  .error-banner__text {
    flex: 1;
    font-size: 0.875rem;
    line-height: 1.4;
  }

  /* ==========================================================================
     Sections & Grids
     ========================================================================== */
  .settings-content-grid {
    display: flex;
    flex-direction: column;
    gap: 2.25rem;
  }

  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .section-header__meta {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .section-header__title {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 600;
    color: var(--text);
    letter-spacing: -0.01em;
  }

  .section-header__desc {
    margin: 0;
    font-size: 0.85rem;
    color: var(--muted);
  }

  .cards-duo {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1.25rem;
  }

  .cards-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 1.25rem;
  }

  /* ==========================================================================
     Setting Card Primitives
     ========================================================================== */
  :global(.setting-card) {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    padding: 1.5rem;
    background: var(--surface-solid);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .card-head__title-wrap {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .card-head__title-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .card-head__title {
    font-size: 1rem;
    font-weight: 600;
    color: var(--text);
  }

  .card-head__desc {
    font-size: 0.825rem;
    color: var(--muted);
    line-height: 1.4;
  }

  .card-head__ram-badge {
    display: flex;
    align-items: baseline;
    gap: 0.25rem;
    padding: 0.35rem 0.75rem;
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: var(--radius-md);
  }

  .ram-badge__val {
    font-size: 1.25rem;
    font-weight: 700;
    color: var(--accent);
    line-height: 1;
  }

  .ram-badge__unit {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--accent);
  }

  /* ==========================================================================
     Segmented Theme Buttons
     ========================================================================== */
  .theme-segmented-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
  }

  .theme-choice-btn {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 1rem 0.5rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--muted);
    font-family: inherit;
    font-size: 0.85rem;
    font-weight: 500;
    cursor: pointer;
    position: relative;
    transition: all var(--dur-fast) ease-out;
  }

  .theme-choice-btn:hover {
    color: var(--text);
    border-color: var(--border-focus);
    background: var(--surface-up-solid);
  }

  .theme-choice-btn--active {
    color: var(--text);
    border-color: var(--accent);
    background: rgba(16, 185, 129, 0.1);
  }

  .theme-choice-btn__icon {
    display: flex;
  }

  .choice-active-dot {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 6px var(--accent);
  }

  /* ==========================================================================
     Explainer Box
     ========================================================================== */
  .explainer-box {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .explainer-box__icon {
    display: flex;
    color: var(--accent-cyan);
    margin-top: 0.1rem;
    flex-shrink: 0;
  }

  .explainer-box__text {
    margin: 0;
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.45;
  }

  /* ==========================================================================
     Theme Preview Stage
     ========================================================================== */
  .theme-preview-stage {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-md);
    background: var(--surface-up-solid);
    border: 1px solid var(--border);
    overflow: hidden;
  }

  .theme-preview-stage__topbar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem;
    background: var(--topbar-bg);
    border-bottom: 1px solid var(--border);
  }

  .preview-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .preview-dot--red { background: #ef4444; }
  .preview-dot--yellow { background: #f59e0b; }
  .preview-dot--green { background: #10b981; }

  .preview-title {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--muted);
    margin-left: 0.5rem;
  }

  .theme-preview-stage__body {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 1.25rem;
    gap: 1rem;
  }

  .preview-chip {
    font-size: 0.7rem;
    font-weight: 600;
    padding: 0.25rem 0.5rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    color: var(--text);
  }

  .preview-lines {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    flex: 1;
    max-width: 160px;
  }

  .preview-line {
    height: 4px;
    background: var(--muted);
    opacity: 0.3;
    border-radius: 2px;
  }

  .preview-line--long { width: 100%; }
  .preview-line--short { width: 60%; }

  /* ==========================================================================
     Language Showcase
     ========================================================================== */
  .language-showcase {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .lang-tile {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.875rem 1.25rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: all var(--dur-fast) ease-out;
  }

  .lang-tile:hover {
    border-color: var(--border-focus);
    background: var(--surface-up-solid);
  }

  .lang-tile--active {
    border-color: var(--accent);
    background: rgba(16, 185, 129, 0.08);
  }

  .lang-tile__flag {
    font-size: 1.5rem;
    margin-right: 0.75rem;
  }

  .lang-tile__info {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    flex: 1;
  }

  .lang-tile__name {
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text);
  }

  .lang-tile__sub {
    font-size: 0.75rem;
    color: var(--muted);
  }

  /* ==========================================================================
     Preset Chips & Sliders
     ========================================================================== */
  .preset-chips-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }

  .preset-chip {
    padding: 0.35rem 0.75rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-pill);
    color: var(--muted);
    font-size: 0.8rem;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--dur-fast) ease-out;
  }

  .preset-chip:hover {
    color: var(--text);
    border-color: var(--border-focus);
  }

  .preset-chip--active {
    background: var(--accent);
    color: var(--text-inverse);
    border-color: var(--accent);
    font-weight: 600;
  }

  .slider-control-group {
    display: flex;
    align-items: center;
    gap: 1.25rem;
  }

  .slider-wrapper {
    flex: 1;
    position: relative;
    display: flex;
    align-items: center;
  }

  .range-slider {
    width: 100%;
    height: 6px;
    -webkit-appearance: none;
    appearance: none;
    background: var(--surface-up);
    border-radius: 3px;
    outline: none;
    position: relative;
    z-index: 2;
    cursor: pointer;
  }

  .range-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--accent);
    border: 2px solid #ffffff;
    box-shadow: 0 0 8px rgba(16, 185, 129, 0.5);
    cursor: pointer;
    transition: transform var(--dur-fast) ease-out;
  }

  .range-slider::-webkit-slider-thumb:hover {
    transform: scale(1.15);
  }

  .slider-track-fill {
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%);
    height: 6px;
    background: var(--accent);
    border-radius: 3px;
    pointer-events: none;
    z-index: 1;
  }

  .custom-num-dock {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .input--num {
    width: 110px;
    text-align: right;
    font-family: var(--font-mono);
  }

  .input-unit {
    font-size: 0.85rem;
    color: var(--muted);
    font-weight: 500;
  }

  /* ==========================================================================
     JDK Inputs & Probes
     ========================================================================== */
  .input-with-actions {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .action-buttons-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .probe-feedback-banner {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    font-size: 0.85rem;
    margin-top: 0.5rem;
  }

  .probe-feedback-banner--ok {
    background: rgba(16, 185, 129, 0.1);
    border: 1px solid rgba(16, 185, 129, 0.3);
    color: var(--accent);
  }

  .probe-feedback-banner--err {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: var(--accent-red);
  }

  .jvm-telemetry-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
    padding: 1rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    margin-top: 0.75rem;
  }

  .jvm-kv {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .jvm-kv--full {
    grid-column: 1 / -1;
  }

  .jvm-k {
    font-size: 0.75rem;
    color: var(--muted);
    font-weight: 500;
  }

  .jvm-v {
    font-size: 0.875rem;
    color: var(--text);
    font-weight: 600;
  }

  .code-pill {
    display: inline-block;
    padding: 0.35rem 0.6rem;
    background: var(--log-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: 0.8rem;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }

  /* ==========================================================================
     Feature Toggles Grid
     ========================================================================== */
  .toggles-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1.25rem;
  }

  :global(.toggle-card) {
    padding: 1.25rem;
    background: var(--surface-solid);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .toggle-card__main {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    height: 100%;
  }

  .toggle-card__info {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1;
  }

  .toggle-card__badge-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .speedup-pill {
    font-size: 0.7rem;
    color: var(--accent-purple);
    font-weight: 600;
  }

  .toggle-card__title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text);
  }

  .toggle-card__desc {
    margin: 0;
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.4;
  }

  .toggle-card__switch {
    display: flex;
    align-items: center;
    padding-top: 0.25rem;
  }

  /* Switch Control */
  .switch-control {
    position: relative;
    display: inline-block;
    width: 44px;
    height: 24px;
    cursor: pointer;
  }

  .switch-control input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .switch-slider {
    position: absolute;
    inset: 0;
    background-color: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: 24px;
    transition: all var(--dur-fast) ease-out;
  }

  .switch-slider:before {
    position: absolute;
    content: "";
    height: 16px;
    width: 16px;
    left: 3px;
    bottom: 3px;
    background-color: #ffffff;
    border-radius: 50%;
    transition: transform var(--dur-fast) ease-out;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  }

  .switch-control input:checked + .switch-slider {
    background-color: var(--accent);
    border-color: var(--accent);
  }

  .switch-control input:checked + .switch-slider:before {
    transform: translateX(20px);
  }

  /* ==========================================================================
     Data Directory Display & Subfolders
     ========================================================================== */
  .path-display-card {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .path-display-card__code {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--accent);
  }

  .path-display-card__code code {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    color: var(--text);
    word-break: break-all;
  }

  .path-display-card__actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .subfolders-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
  }

  .subfolder-chip {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--muted);
    font-family: inherit;
    font-size: 0.85rem;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--dur-fast) ease-out;
  }

  .subfolder-chip:hover {
    color: var(--text);
    border-color: var(--border-focus);
    background: var(--surface-up-solid);
  }

  /* ==========================================================================
     Health Status Rows
     ========================================================================== */
  .health-status-rows {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .health-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 1rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .health-k {
    font-size: 0.85rem;
    color: var(--muted);
  }

  .health-v {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--text);
  }

  .health-v--mono {
    font-family: var(--font-mono);
    font-size: 0.8rem;
  }

  /* ==========================================================================
     About & Community VIP Banner
     ========================================================================== */
  .about-pavilion-card {
    display: grid;
    grid-template-columns: 1.2fr 1fr;
    gap: 1.5rem;
    padding: 2rem;
    background: var(--surface-solid);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-sm);
  }

  .about-pavilion-card__left {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .about-badge-row {
    display: flex;
  }

  .about-title {
    margin: 0;
    line-height: 1.2;
  }

  .about-tagline {
    margin: 0;
    font-size: 0.9rem;
    color: var(--muted);
    line-height: 1.4;
  }

  .about-meta-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 0.75rem;
    margin-top: 0.75rem;
  }

  .about-meta-item {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .about-meta-k {
    font-size: 0.75rem;
    color: var(--muted);
  }

  .about-meta-v {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--text);
  }

  .about-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.85rem;
    color: var(--accent);
    text-decoration: none;
    font-weight: 600;
  }

  .about-link:hover {
    text-decoration: underline;
  }

  .about-pavilion-card__right {
    display: flex;
    align-items: center;
  }

  .vip-banner-box {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 1rem;
    padding: 1.75rem;
    width: 100%;
    background: radial-gradient(circle at center, rgba(16, 185, 129, 0.12) 0%, rgba(6, 182, 212, 0.04) 100%), var(--surface-up);
    border: 1px solid rgba(16, 185, 129, 0.3);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px rgba(16, 185, 129, 0.08);
  }

  .vip-banner-box__icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: rgba(16, 185, 129, 0.15);
    color: var(--accent);
  }

  .vip-banner-box__text {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .vip-banner-box__title {
    font-size: 1rem;
    font-weight: 700;
    color: var(--text);
  }

  .vip-banner-box__sub {
    font-size: 0.8rem;
    color: var(--muted);
    line-height: 1.4;
  }

  /* ==========================================================================
     Responsive Media Queries
     ========================================================================== */
  @media (max-width: 1100px) {
    .cards-duo,
    .toggles-grid,
    .about-pavilion-card {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 768px) {
    .settings-dashboard {
      padding: 1.25rem 1rem 3rem;
    }

    .settings-hero {
      flex-direction: column;
      align-items: stretch;
    }

    .theme-segmented-grid,
    .subfolders-grid,
    .jvm-telemetry-grid,
    .about-meta-grid {
      grid-template-columns: 1fr;
    }

    .slider-control-group {
      flex-direction: column;
      align-items: stretch;
    }

    .custom-num-dock {
      justify-content: space-between;
    }
  }
</style>
