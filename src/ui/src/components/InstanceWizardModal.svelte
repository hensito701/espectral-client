<!--
  ============================================================================
  InstanceWizardModal.svelte — Horizon Glass Instance Creator
  ============================================================================
  z40 modal wizard guiding players through version selection, mod loader,
  memory allocation, launcher import, and instance provisioning.

  Listens to 'horizon:open-wizard' event globally on window.

  Props:
    - open?: boolean
    - onclose?: () => void
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { CreateInstanceRequest, ImportSource, Loader } from '../lib/types';
  import { createInstance, getImportSources, pickFolder } from '../lib/api';
  import { versions as versionsStore, instances as instancesStore } from '../lib/stores';
  import { pushToast } from '../lib/toast.svelte';
  import { scalePop, fade } from '../lib/motion';
  import Btn from './Btn.svelte';
  import Field from './Field.svelte';
  import LoaderBadge from './LoaderBadge.svelte';
  import Badge from './Badge.svelte';
  import { memLabel } from '../lib/format';
  import { t } from '../lib/i18n.svelte';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let { open = $bindable(false), onclose }: Props = $props();

  // Wizard Steps
  let step = $state<1 | 2 | 3>(1);

  // Form State
  let selectedVersion = $state('');
  let selectedLoader = $state<Loader>('fabric');
  let instanceName = $state('');
  let memoryMb = $state(4096);
  let gameDir = $state('');
  let wizardHue = $state(0);
  let importEnabled = $state(false);
  let pickingFolder = $state(false);
  let importSources = $state<ImportSource[]>([]);
  let selectedSourceId = $state('');
  // Search & Filter State
  let searchQuery = $state('');
  let versionFilter = $state<'all' | 'release' | 'snapshot' | 'old'>('release');

  // Operation State
  let isCreating = $state(false);
  let errorMsg = $state('');

  const manifest = $derived($versionsStore.value);

  // Filtered versions list
  const filteredVersions = $derived.by(() => {
    if (!manifest || !manifest.versions) return [];
    let list = manifest.versions;

    // Filter by type
    if (versionFilter === 'release') {
      list = list.filter((v) => v.type === 'release');
    } else if (versionFilter === 'snapshot') {
      list = list.filter((v) => v.type === 'snapshot');
    } else if (versionFilter === 'old') {
      list = list.filter((v) => v.type === 'old_beta' || v.type === 'old_alpha');
    }

    // Filter by query
    const q = searchQuery.trim().toLowerCase();
    if (q) {
      list = list.filter((v) => v.id.toLowerCase().includes(q));
    }

    return list.slice(0, 50);
  });

  // Auto-fill latest version when manifest loads
  $effect(() => {
    if (!selectedVersion && manifest?.latest_release) {
      selectedVersion = manifest.latest_release;
      suggestName();
    }
  });

  function suggestName(): void {
    if (!instanceName || instanceName.startsWith('Minecraft') || instanceName.includes('-')) {
      const ver = selectedVersion || '1.21.1';
      instanceName = `${ver}-${selectedLoader}`;
    }
  }

  function handleSelectVersion(id: string): void {
    selectedVersion = id;
    suggestName();
  }

  function handleSelectLoader(loader: Loader): void {
    selectedLoader = loader;
    suggestName();
  }

  async function loadSources(): Promise<void> {
    if (importSources.length > 0) return;
    try {
      importSources = await getImportSources();
      if (importSources.length > 0) {
        selectedSourceId = importSources[0].id;
      }
    } catch (e) {
      console.error('Failed to load import sources:', e);
    }
  }

  function randomHue(): number {
    return Math.floor(Math.random() * 360);
  }

  function resetForm(): void {
    step = 1;
    selectedVersion = manifest?.latest_release || '';
    selectedLoader = 'fabric';
    instanceName = selectedVersion ? `${selectedVersion}-fabric` : '';
    memoryMb = 4096;
    gameDir = '';
    wizardHue = randomHue();
    pickingFolder = false;
    importEnabled = false;
    selectedSourceId = '';
    searchQuery = '';
    versionFilter = 'release';
    errorMsg = '';
    isCreating = false;
  }

  function closeModal(): void {
    if (isCreating) return;
    open = false;
    if (onclose) onclose();
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape' && !isCreating) {
      e.stopPropagation();
      closeModal();
    }
  }

  async function handleCreate(): Promise<void> {
    if (!instanceName.trim()) {
      errorMsg = t('home.wizardErrorName');
      return;
    }
    if (!selectedVersion) {
      errorMsg = t('home.wizardErrorVersion');
      return;
    }

    isCreating = true;
    errorMsg = '';

    const req: CreateInstanceRequest = {
      name: instanceName.trim(),
      version: selectedVersion,
      loader: selectedLoader,
      memory_mb: memoryMb,
      import_from: importEnabled && selectedSourceId ? selectedSourceId : undefined,
      merge_optionslc: importEnabled && selectedSourceId ? true : undefined,
      hue: wizardHue,
      game_dir: gameDir.trim() ? gameDir.trim() : undefined,
    };

    try {
      const summary = await createInstance(req);
      await $instancesStore.refresh();
      pushToast({
        kind: 'ok',
        text: t('home.wizardSuccess', { name: summary.name }),
      });
      closeModal();
      if (typeof window !== 'undefined') {
        window.location.hash = `#/instances/${encodeURIComponent(summary.name)}`;
      }
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      isCreating = false;
    }
  }

  async function handlePickGameDir(): Promise<void> {
    if (pickingFolder) return;
    pickingFolder = true;
    try {
      const res = await pickFolder(t('home.wizardGameDirTitle'));
      if (res?.path) gameDir = res.path;
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : String(e);
    } finally {
      pickingFolder = false;
    }
  }

  onMount(() => {
    const onOpen = () => {
      resetForm();
      open = true;
    };
    window.addEventListener('horizon:open-wizard', onOpen);
    return () => window.removeEventListener('horizon:open-wizard', onOpen);
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label={t('home.wizardTitle')}
    tabindex="-1"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeModal();
    }}
    onkeydown={handleKeyDown}
    in:fade={{ duration: 150 }}
    out:fade={{ duration: 100 }}
  >
    <div
      class="wizard-modal"
      in:scalePop={{ start: 0.94, duration: 200 }}
      out:scalePop={{ start: 0.94, duration: 150 }}
    >
      <!-- Modal Header -->
      <header class="wizard-header">
        <div class="wizard-header__info">
          <h2 class="wizard-title">{t('home.wizardTitle')}</h2>
          <p class="wizard-subtitle">{t('home.wizardSubtitle')}</p>
        </div>

        <!-- Step Indicator Pills -->
        <div class="wizard-steps-pills">
          <button
            type="button"
            class="wizard-step-pill"
            class:wizard-step-pill--active={step === 1}
            class:wizard-step-pill--done={step > 1}
            onclick={() => (step = 1)}
          >
            1. {t('home.wizardStep1').split('. ')[1] || 'Versión'}
          </button>
          <div class="wizard-step-line"></div>
          <button
            type="button"
            class="wizard-step-pill"
            class:wizard-step-pill--active={step === 2}
            class:wizard-step-pill--done={step > 2}
            onclick={() => (step = 2)}
          >
            2. {t('home.wizardStep2').split('. ')[1] || 'Loader'}
          </button>
          <div class="wizard-step-line"></div>
          <button
            type="button"
            class="wizard-step-pill"
            class:wizard-step-pill--active={step === 3}
            onclick={() => (step = 3)}
          >
            3. {t('home.wizardStep3').split('. ')[1] || 'Ajustes'}
          </button>
        </div>

        <button
          type="button"
          class="wizard-close-btn"
          onclick={closeModal}
          aria-label={t('home.wizardCancelBtn')}
        >
          ✕
        </button>
      </header>

      <!-- Modal Body (Steps) -->
      <div class="wizard-body">
        {#if errorMsg}
          <div class="wizard-error-banner animate-pop-in">
            <span class="wizard-error-icon">⚠️</span>
            <span>{errorMsg}</span>
          </div>
        {/if}

        <!-- STEP 1: Version Selector -->
        {#if step === 1}
          <div class="wizard-step-content animate-fade-up">
            <div class="wizard-quick-versions">
              {#if manifest?.latest_release}
                <button
                  type="button"
                  class="wizard-quick-btn"
                  class:wizard-quick-btn--active={selectedVersion === manifest.latest_release}
                  onclick={() => handleSelectVersion(manifest.latest_release)}
                >
                  <span class="wizard-quick-tag">{t('home.wizardLatestRelease')}</span>
                  <span class="wizard-quick-val">{manifest.latest_release}</span>
                </button>
              {/if}
              {#if manifest?.latest_snapshot}
                <button
                  type="button"
                  class="wizard-quick-btn"
                  class:wizard-quick-btn--active={selectedVersion === manifest.latest_snapshot}
                  onclick={() => handleSelectVersion(manifest.latest_snapshot)}
                >
                  <span class="wizard-quick-tag">{t('home.wizardLatestSnapshot')}</span>
                  <span class="wizard-quick-val">{manifest.latest_snapshot}</span>
                </button>
              {/if}
            </div>

            <!-- Version Filter Bar & Search Input -->
            <div class="wizard-filter-bar">
              <div class="wizard-search-box">
                <svg class="wizard-search-icon" viewBox="0 0 20 20" fill="currentColor">
                  <path fill-rule="evenodd" d="M9 3.5a5.5 5.5 0 100 11 5.5 5.5 0 000-11zM2 9a7 7 0 1112.452 4.391l3.328 3.329a.75.75 0 11-1.06 1.06l-3.329-3.328A7 7 0 012 9z" clip-rule="evenodd" />
                </svg>
                <input
                  type="search"
                  class="wizard-search-input"
                  placeholder={t('home.wizardSearchVersion')}
                  bind:value={searchQuery}
                />
              </div>

              <div class="wizard-type-segments">
                <button
                  type="button"
                  class="wizard-seg-btn"
                  class:wizard-seg-btn--active={versionFilter === 'release'}
                  onclick={() => (versionFilter = 'release')}
                >
                  {t('home.wizardFilterReleases')}
                </button>
                <button
                  type="button"
                  class="wizard-seg-btn"
                  class:wizard-seg-btn--active={versionFilter === 'snapshot'}
                  onclick={() => (versionFilter = 'snapshot')}
                >
                  {t('home.wizardFilterSnapshots')}
                </button>
                <button
                  type="button"
                  class="wizard-seg-btn"
                  class:wizard-seg-btn--active={versionFilter === 'all'}
                  onclick={() => (versionFilter = 'all')}
                >
                  {t('home.wizardFilterAll')}
                </button>
              </div>
            </div>

            <!-- Version Grid List -->
            <div class="wizard-versions-grid">
              {#each filteredVersions as ver (ver.id)}
                <button
                  type="button"
                  class="wizard-version-card"
                  class:wizard-version-card--selected={ver.id === selectedVersion}
                  onclick={() => handleSelectVersion(ver.id)}
                >
                  <span class="wizard-ver-id">{ver.id}</span>
                  <Badge
                    variant={ver.type === 'release' ? 'ok' : ver.type === 'snapshot' ? 'accent' : 'muted'}
                    size="sm"
                  >
                    {ver.type}
                  </Badge>
                </button>
              {/each}
            </div>
          </div>

        <!-- STEP 2: Loader Selector -->
        {:else if step === 2}
          <div class="wizard-step-content animate-fade-up">
            <div class="wizard-loaders-grid">
              <!-- Fabric Card -->
              <button
                type="button"
                class="wizard-loader-card"
                class:wizard-loader-card--selected={selectedLoader === 'fabric'}
                onclick={() => handleSelectLoader('fabric')}
              >
                <div class="wizard-loader-card__head">
                  <LoaderBadge loader="fabric" version={selectedVersion} size="lg" />
                  <Badge variant="accent" size="sm">Recomendado</Badge>
                </div>
                <h4 class="wizard-loader-card__name">{t('home.wizardLoaderFabric')}</h4>
                <p class="wizard-loader-card__desc">{t('home.wizardLoaderFabricDesc')}</p>
              </button>

              <!-- Vanilla Card -->
              <button
                type="button"
                class="wizard-loader-card"
                class:wizard-loader-card--selected={selectedLoader === 'vanilla'}
                onclick={() => handleSelectLoader('vanilla')}
              >
                <div class="wizard-loader-card__head">
                  <LoaderBadge loader="vanilla" version={selectedVersion} size="lg" />
                  <Badge variant="ok" size="sm">Oficial</Badge>
                </div>
                <h4 class="wizard-loader-card__name">{t('home.wizardLoaderVanilla')}</h4>
                <p class="wizard-loader-card__desc">{t('home.wizardLoaderVanillaDesc')}</p>
              </button>

              <!-- NeoForge Card -->
              <button
                type="button"
                class="wizard-loader-card"
                class:wizard-loader-card--selected={selectedLoader === 'neoforge'}
                onclick={() => handleSelectLoader('neoforge')}
              >
                <div class="wizard-loader-card__head">
                  <LoaderBadge loader="neoforge" version={selectedVersion} size="lg" />
                  <Badge variant="warn" size="sm">Avanzado</Badge>
                </div>
                <h4 class="wizard-loader-card__name">{t('home.wizardLoaderNeoForge')}</h4>
                <p class="wizard-loader-card__desc">{t('home.wizardLoaderNeoForgeDesc')}</p>
              </button>
            </div>
          </div>

        <!-- STEP 3: Instance Name, Memory & Options -->
        {:else if step === 3}
          <div class="wizard-step-content animate-fade-up">
            <div class="wizard-form-stack">
              <!-- Name Field -->
              <Field
                label={t('home.wizardInstanceName')}
                hint="Identificador único para tu carpeta y perfil de juego."
                required
              >
                <input
                  type="text"
                  class="wizard-input"
                  placeholder={t('home.wizardInstanceNamePlaceholder')}
                  bind:value={instanceName}
                />
              </Field>

              <!-- Memory Slider & Value -->
              <Field
                label={t('home.wizardMemory')}
                hint={t('home.wizardMemoryHint')}
              >
                <div class="wizard-memory-box">
                  <input
                    type="range"
                    class="wizard-range"
                    min="1024"
                    max="16384"
                    step="512"
                    bind:value={memoryMb}
                  />
                  <div class="wizard-memory-val">
                    <span class="wizard-memory-num">{memLabel(memoryMb)}</span>
                    <span class="wizard-memory-raw">({memoryMb} MB)</span>
                  </div>
                </div>
              </Field>
              <!-- Game Directory (optional override) -->
              <Field
                label={t('home.wizardGameDirTitle')}
                hint={t('home.wizardGameDirHint')}
              >
                <div class="wizard-gamedir-row">
                  <input
                    type="text"
                    class="wizard-input wizard-gamedir-input"
                    placeholder={t('home.wizardGameDirDefault')}
                    value={gameDir}
                    readonly
                  />
                  <Btn
                    variant="secondary"
                    size="sm"
                    loading={pickingFolder}
                    disabled={pickingFolder}
                    onclick={handlePickGameDir}
                  >
                    {t('home.wizardGameDirBrowse')}
                  </Btn>
                  {#if gameDir}
                    <Btn
                      variant="ghost"
                      size="sm"
                      disabled={pickingFolder}
                      onclick={() => (gameDir = '')}
                    >
                      {t('home.wizardGameDirClear')}
                    </Btn>
                  {/if}
                </div>
              </Field>

              <!-- Auto-assigned accent hue note -->
              <p class="wizard-hue-note">
                <span
                  class="wizard-hue-dot"
                  style="background: hsl({wizardHue}, 70%, 55%);"
                  aria-hidden="true"
                ></span>
                {t('home.wizardHueNote')}
              </p>

              <!-- Import Settings Option -->
              <div class="wizard-import-box">
                <label class="wizard-checkbox-label">
                  <input
                    type="checkbox"
                    bind:checked={importEnabled}
                    onchange={() => {
                      if (importEnabled) void loadSources();
                    }}
                  />
                  <span class="wizard-checkbox-title">{t('home.wizardImportOptions')}</span>
                </label>

                {#if importEnabled}
                  <div class="wizard-import-select-wrap animate-fade-up">
                    <label for="wizard-source-select" class="wizard-field-lbl">
                      {t('home.wizardImportSource')}:
                    </label>
                    {#if importSources.length > 0}
                      <select
                        id="wizard-source-select"
                        class="wizard-select"
                        bind:value={selectedSourceId}
                      >
                        {#each importSources as src (src.id)}
                          <option value={src.id}>{src.label} ({src.kind})</option>
                        {/each}
                      </select>
                    {:else}
                      <p class="wizard-no-sources">Buscando launchers instalados...</p>
                    {/if}
                  </div>
                {/if}
              </div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Modal Footer -->
      <footer class="wizard-footer">
        <div class="wizard-footer__left">
          {#if step > 1}
            <Btn variant="secondary" onclick={() => (step = (step - 1) as 1 | 2 | 3)}>
              ← {t('home.wizardPrevBtn')}
            </Btn>
          {/if}
        </div>

        <div class="wizard-footer__right">
          <Btn variant="ghost" onclick={closeModal}>
            {t('home.wizardCancelBtn')}
          </Btn>

          {#if step < 3}
            <Btn
              variant="primary"
              onclick={() => (step = (step + 1) as 1 | 2 | 3)}
              disabled={step === 1 && !selectedVersion}
            >
              {t('home.wizardNextBtn')} →
            </Btn>
          {:else}
            <Btn
              variant="primary"
              loading={isCreating}
              disabled={!instanceName.trim() || !selectedVersion}
              onclick={handleCreate}
            >
              ✨ {isCreating ? t('home.wizardCreating') : t('home.wizardCreateBtn')}
            </Btn>
          {/if}
        </div>
      </footer>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(3, 6, 14, 0.78);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    z-index: var(--z-modal, 40);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4, 16px);
  }

  .wizard-modal {
    width: 100%;
    max-width: 680px;
    max-height: 85vh;
    background: var(--surface-solid, #0d1222);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-xl, 1.25rem);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.8), 0 0 32px rgba(16, 185, 129, 0.1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* --- Header --- */
  .wizard-header {
    padding: var(--space-5, 20px) var(--space-6, 24px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    background: rgba(16, 22, 42, 0.4);
  }

  .wizard-title {
    margin: 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-xl, 1.375rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .wizard-subtitle {
    margin: 2px 0 0 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .wizard-steps-pills {
    display: flex;
    align-items: center;
    gap: 6px;
    background: rgba(255, 255, 255, 0.04);
    padding: 3px 6px;
    border-radius: var(--radius-pill, 9999px);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }

  .wizard-step-pill {
    background: transparent;
    border: none;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--muted, #8e9eb8);
    padding: 2px 8px;
    border-radius: var(--radius-pill, 9999px);
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .wizard-step-pill--active {
    background: var(--accent, #10b981);
    color: #ffffff;
    box-shadow: 0 0 8px rgba(16, 185, 129, 0.4);
  }

  .wizard-step-pill--done {
    color: var(--text, #e8ecf4);
  }

  .wizard-step-line {
    width: 8px;
    height: 1px;
    background: rgba(255, 255, 255, 0.1);
  }

  .wizard-close-btn {
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 16px;
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    transition: color var(--dur-fast, 120ms) ease;
  }

  .wizard-close-btn:hover {
    color: var(--text, #e8ecf4);
  }

  /* --- Body --- */
  .wizard-body {
    flex: 1;
    overflow-y: auto;
    padding: var(--space-6, 24px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .wizard-error-banner {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: var(--radius-md, 0.625rem);
    color: #fca5a5;
    font-size: var(--text-sm, 0.875rem);
  }

  /* --- Step 1: Version Picker --- */
  .wizard-quick-versions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3, 12px);
    margin-bottom: var(--space-4, 16px);
  }

  .wizard-quick-btn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: var(--space-3, 12px);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
    cursor: pointer;
    text-align: left;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .wizard-quick-btn:hover {
    background: rgba(16, 185, 129, 0.08);
    border-color: rgba(16, 185, 129, 0.4);
  }

  .wizard-quick-btn--active {
    background: rgba(16, 185, 129, 0.15);
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.2);
  }

  .wizard-quick-tag {
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    color: var(--accent, #10b981);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .wizard-quick-val {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .wizard-filter-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    margin-bottom: var(--space-3, 12px);
  }

  .wizard-search-box {
    position: relative;
    flex: 1;
  }

  .wizard-search-icon {
    position: absolute;
    left: 10px;
    top: 50%;
    transform: translateY(-50%);
    width: 16px;
    height: 16px;
    color: var(--muted, #8e9eb8);
  }

  .wizard-search-input {
    width: 100%;
    padding: 8px 12px 8px 34px !important;
    font-size: var(--text-sm, 0.875rem) !important;
  }

  .wizard-type-segments {
    display: inline-flex;
    padding: 2px;
    background: rgba(255, 255, 255, 0.05);
    border-radius: var(--radius-pill, 9999px);
    border: 1px solid rgba(255, 255, 255, 0.08);
  }

  .wizard-seg-btn {
    padding: 4px 10px;
    font-family: var(--font-body, 'Space Grotesk', sans-serif);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
    background: transparent;
    border: none;
    border-radius: var(--radius-pill, 9999px);
    color: var(--muted, #8e9eb8);
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .wizard-seg-btn--active {
    background: var(--surface-up-solid, #161e36);
    color: #ffffff;
  }

  .wizard-versions-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: var(--space-2, 8px);
    max-height: 260px;
    overflow-y: auto;
    padding-right: 4px;
  }

  .wizard-version-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-sm, 0.375rem);
    cursor: pointer;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .wizard-version-card:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.2);
  }

  .wizard-version-card--selected {
    background: rgba(16, 185, 129, 0.15);
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 10px rgba(16, 185, 129, 0.2);
  }

  .wizard-ver-id {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-weight: 600;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #e8ecf4);
  }

  /* --- Step 2: Loader Cards --- */
  .wizard-loaders-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-4, 16px);
  }

  .wizard-loader-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: var(--space-4, 16px);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-lg, 0.875rem);
    cursor: pointer;
    text-align: left;
    transition: all var(--dur-fast, 120ms) ease;
  }

  .wizard-loader-card:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.2);
    transform: translateY(-2px);
  }

  .wizard-loader-card--selected {
    background: rgba(16, 185, 129, 0.12);
    border-color: var(--accent, #10b981);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4), 0 0 16px rgba(16, 185, 129, 0.2);
  }

  .wizard-loader-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    margin-bottom: var(--space-2, 8px);
  }

  .wizard-loader-card__name {
    margin: 0 0 4px 0;
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--text, #e8ecf4);
  }

  .wizard-loader-card__desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--muted, #8e9eb8);
    line-height: var(--leading-snug, 1.35);
  }

  /* --- Step 3: Form Stack --- */
  .wizard-form-stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-5, 20px);
  }

  .wizard-input {
    width: 100%;
    font-size: var(--text-base, 1rem) !important;
  }

  .wizard-memory-box {
    display: flex;
    align-items: center;
    gap: var(--space-4, 16px);
  }

  .wizard-range {
    flex: 1;
    height: 6px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 3px;
    outline: none;
  }

  .wizard-memory-val {
    display: flex;
    align-items: baseline;
    gap: 4px;
    min-width: 120px;
    justify-content: flex-end;
  }

  .wizard-memory-num {
    font-family: var(--font-display, 'Space Grotesk', sans-serif);
    font-size: var(--text-lg, 1.125rem);
    font-weight: 700;
    color: var(--accent, #10b981);
  }

  .wizard-memory-raw {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }
  .wizard-gamedir-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .wizard-gamedir-input {
    flex: 1;
    min-width: 0;
    text-overflow: ellipsis;
  }

  .wizard-hue-note {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    margin: 0;
  }

  .wizard-hue-dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    flex-shrink: 0;
    box-shadow: 0 0 8px rgba(255, 255, 255, 0.25);
  }

  .wizard-import-box {
    padding: var(--space-4, 16px);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    border-radius: var(--radius-md, 0.625rem);
  }

  .wizard-checkbox-label {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    cursor: pointer;
  }

  .wizard-checkbox-title {
    font-weight: 600;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #e8ecf4);
  }

  .wizard-import-select-wrap {
    margin-top: var(--space-3, 12px);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .wizard-field-lbl {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
  }

  .wizard-select {
    width: 100%;
  }

  .wizard-no-sources {
    font-size: var(--text-xs, 0.75rem);
    color: var(--muted, #8e9eb8);
    font-style: italic;
  }

  /* --- Footer --- */
  .wizard-footer {
    padding: var(--space-4, 16px) var(--space-6, 24px);
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(16, 22, 42, 0.4);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .wizard-footer__right {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }
</style>
