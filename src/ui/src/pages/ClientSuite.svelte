<!--
  ============================================================================
  ClientSuite.svelte — Horizon Glass In-Game Suite & Macro Matrix (#/client)
  ============================================================================
  Next-generation control console for Espectral in-game client features,
  runtime toggles, and client-side macro automation engine.
  Features instant optimistic patching, SSE launch-exit synchronization,
  and tactile dual-column glass layout.
-->
<script lang="ts">
  import { instances } from '../lib/stores';
  import { getInstanceClient, patchInstanceClient, subscribeEvents } from '../lib/api';
  import type { ClientInfo, ClientMacro, ClientRegistryEntry } from '../lib/types';
  import { pushToast } from '../lib/toast.svelte';
  import { t } from '../lib/i18n.svelte';
  import { fade, flyY } from '../lib/motion';

  // Horizon Glass UI Primitives
  import Btn from '../components/Btn.svelte';
  import GlassCard from '../components/GlassCard.svelte';
  import Badge from '../components/Badge.svelte';
  import Field from '../components/Field.svelte';
  import LoaderBadge from '../components/LoaderBadge.svelte';
  import GradientText from '../components/GradientText.svelte';

  const STORAGE_KEY_INSTANCE = 'horizon:client-instance';

  // Engine macro constraints (mirrored client-side to prevent 400s)
  const MAX_MACROS = 32;
  const MAX_MACRO_NAME = 40;
  const MAX_KEYBIND = 32;
  const MAX_ACTIONS = 16;
  const MAX_ACTION_TEXT = 256;
  const MACRO_ID_RE = /^[A-Za-z0-9_-]{1,32}$/;

  // Reactive state
  let targetInstance = $state(
    (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_KEY_INSTANCE)) || ''
  );
  let info = $state<ClientInfo | null>(null);
  let loading = $state(true);
  let error = $state('');
  let togglingFeatureId = $state('');
  let patchError = $state('');

  // Macro Editor Draft State
  let macros = $state<ClientMacro[]>([]);
  let macrosDirty = $state(false);
  let savingMacros = $state(false);
  let macroError = $state('');
  let savedMsg = $state('');
  let activeMacroIndex = $state(0);

  // Selected Instance derivation
  const selectedInstance = $derived(
    $instances.value.find((inst) => inst.name === targetInstance) ?? null
  );

  // Sync instance selection with localStorage
  $effect(() => {
    if (typeof localStorage !== 'undefined' && targetInstance) {
      localStorage.setItem(STORAGE_KEY_INSTANCE, targetInstance);
    }
  });

  // Ensure target instance default exists
  $effect(() => {
    if ($instances.value.length > 0) {
      const exists = $instances.value.some((inst) => inst.name === targetInstance);
      if (!exists || !targetInstance) {
        targetInstance = $instances.value[0].name;
      }
    }
  });

  // Load client info on targetInstance change
  $effect(() => {
    if (targetInstance) {
      void load();
    }
  });

  let loadSeq = 0;
  async function load(skipMacrosIfDirty = false) {
    if (!targetInstance) return;
    const currentSeq = ++loadSeq;
    loading = true;
    error = '';
    patchError = '';
    savedMsg = '';

    try {
      const data = await getInstanceClient(targetInstance);
      if (currentSeq !== loadSeq) return; // Discard race condition
      info = data;

      if (!skipMacrosIfDirty || !macrosDirty) {
        macros = cloneMacros(data.config.macros || []);
        macrosDirty = false;
      }
    } catch (e) {
      if (currentSeq !== loadSeq) return;
      error = e instanceof Error ? e.message : String(e);
      info = null;
    } finally {
      if (currentSeq === loadSeq) {
        loading = false;
      }
    }
  }

  function cloneMacros(list: ClientMacro[]): ClientMacro[] {
    return list.map((m) => ({
      ...m,
      actions: (m.actions || []).map((a) => ({ ...a })),
    }));
  }

  // Contract A: Refresh mirror after game launch exits
  $effect(() => {
    const unsub = subscribeEvents(({ type }) => {
      if (type === 'launch-exit' && targetInstance) {
        void load(true);
      }
    });
    return () => unsub();
  });

  function featureEnabled(id: string, defaultEnabled: boolean): boolean {
    const st = info?.config.features?.[id];
    return st ? st.enabled : defaultEnabled;
  }

  // Contract B: Merge patch responses gracefully (never revert the switch:
  // the server persisted intent — errors are warnings, toasted by callers).
  function absorbPatch(resp: ClientInfo): void {
    if (Array.isArray(resp.registry)) {
      info = resp;
    } else if (info && resp.config) {
      info = { ...info, config: resp.config, errors: resp.errors };
    }
  }

  function warnOrOk(resp: ClientInfo, okText: string): void {
    const ids = (resp.errors ?? []).map((e) => e.feature);
    if (ids.length > 0) {
      pushToast({ kind: 'info', text: t('client.savedWithWarnings', { features: ids.join(', ') }) });
    } else {
      pushToast({ kind: 'ok', text: okText });
    }
  }

  async function toggleFeature(feat: ClientRegistryEntry, next: boolean) {
    if (!targetInstance || togglingFeatureId !== '') return;
    togglingFeatureId = feat.id;
    patchError = '';

    try {
      const result = await patchInstanceClient(targetInstance, {
        features: { [feat.id]: { enabled: next } },
      });
      absorbPatch(result);
      warnOrOk(
        result,
        `${feat.name} ${next ? t('mods.enabled') || 'activado' : t('mods.disabled') || 'desactivado'}`
      );
    } catch (e) {
      patchError = e instanceof Error ? e.message : String(e);
      pushToast({
        kind: 'err',
        text: patchError,
      });
    } finally {
      togglingFeatureId = '';
    }
  }

  function newMacroId(): string {
    const taken = new Set(macros.map((m) => m.id));
    let n = macros.length + 1;
    let id = `macro${n}`;
    while (taken.has(id) || !MACRO_ID_RE.test(id)) {
      n += 1;
      id = `macro${n}`;
    }
    return id;
  }

  function addMacro() {
    if (macros.length >= MAX_MACROS) return;
    const newId = newMacroId();
    macros = [
      ...macros,
      { id: newId, name: '', keybind: '', actions: [{ type: 'chat', text: '' }] },
    ];
    activeMacroIndex = macros.length - 1;
    macrosDirty = true;
    macroError = '';
    savedMsg = '';
  }

  function removeMacro(index: number) {
    macros = macros.filter((_, i) => i !== index);
    if (activeMacroIndex >= macros.length) {
      activeMacroIndex = Math.max(0, macros.length - 1);
    }
    macrosDirty = true;
    macroError = '';
    savedMsg = '';
  }

  function addAction(macro: ClientMacro) {
    if (macro.actions.length >= MAX_ACTIONS) return;
    macro.actions = [...macro.actions, { type: 'chat', text: '' }];
    macrosDirty = true;
    savedMsg = '';
  }

  function removeAction(macro: ClientMacro, index: number) {
    if (macro.actions.length <= 1) return;
    macro.actions = macro.actions.filter((_, i) => i !== index);
    macrosDirty = true;
    savedMsg = '';
  }

  function markDirty() {
    macrosDirty = true;
    macroError = '';
    savedMsg = '';
  }

  // Keybind capture: translate window.event.code into a MacroEngine-valid string.
  // Verified against MacroEngine.getGlfwKeyCode/getGlfwKeyFromName — every
  // string produced here resolves to a real GLFW key code.
  let capturingKeybind = $state(false);

  function codeToKeybind(code: string): string | null {
    if (code.startsWith('Key') && code.length === 4) return code.slice(3).toLowerCase();
    if (code.startsWith('Digit') && code.length === 6) {
      const d = code.slice(5);
      if (d >= '0' && d <= '9') return d;
      return null;
    }
    if (code.startsWith('Numpad') && code.length === 7) {
      const d = code.slice(6);
      if (d >= '0' && d <= '9') return `numpad.${d}`;
    }
    if (code.startsWith('F')) {
      const n = Number(code.slice(1));
      if (Number.isInteger(n) && n >= 1 && n <= 15) return `f${n}`;
    }
    switch (code) {
      case 'NumpadAdd': return 'numpad.add';
      case 'NumpadSubtract': return 'numpad.subtract';
      case 'NumpadMultiply': return 'numpad.multiply';
      case 'NumpadDivide': return 'numpad.divide';
      case 'NumpadEnter': return 'numpad.enter';
      case 'NumpadDecimal': return 'numpad.decimal';
      case 'Space': return 'space';
      case 'Enter': return 'enter';
      case 'Tab': return 'tab';
      case 'Backspace': return 'backspace';
      case 'ShiftLeft': return 'shift';
      case 'ShiftRight': return 'right.shift';
      case 'ControlLeft': return 'control';
      case 'ControlRight': return 'right.control';
      case 'AltLeft': return 'alt';
      case 'AltRight': return 'right.alt';
      case 'Minus': return 'minus';
      case 'Equal': return 'equals';
      case 'BracketLeft': return 'lbracket';
      case 'BracketRight': return 'rbracket';
      case 'Semicolon': return 'semicolon';
      case 'Quote': return 'apostrophe';
      case 'Comma': return 'comma';
      case 'Period': return 'period';
      case 'Slash': return 'slash';
      case 'Backslash': return 'backslash';
      case 'Backquote': return 'grave';
      default: return null;
    }
  }

  function handleKeybindCapture(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    const target = e.currentTarget as HTMLInputElement | null;
    if (e.code === 'Escape') {
      // Abort capture WITHOUT assigning.
      capturingKeybind = false;
      target?.blur();
      return;
    }
    const mapped = codeToKeybind(e.code);
    if (mapped === null) return; // Unknown key: stay armed.
    if (activeMacro) {
      activeMacro.keybind = mapped;
      markDirty();
    }
    capturingKeybind = false;
    target?.blur();
  }

  const macrosValid = $derived(
    macros.length <= MAX_MACROS &&
      new Set(macros.map((m) => m.id)).size === macros.length &&
      macros.every(
        (m) =>
          MACRO_ID_RE.test(m.id) &&
          m.name.trim().length > 0 &&
          m.name.length <= MAX_MACRO_NAME &&
          m.keybind.trim().length > 0 &&
          m.keybind.length <= MAX_KEYBIND &&
          m.actions.length >= 1 &&
          m.actions.length <= MAX_ACTIONS &&
          m.actions.every((a) => a.text.trim().length > 0 && a.text.length <= MAX_ACTION_TEXT)
      )
  );

  async function saveMacros() {
    if (!targetInstance || !macrosValid || savingMacros) return;
    savingMacros = true;
    macroError = '';
    savedMsg = '';

    try {
      const result = await patchInstanceClient(targetInstance, { macros });
      absorbPatch(result);
      if (info?.config?.macros) {
        macros = cloneMacros(info.config.macros);
      }
      macrosDirty = false;
      savedMsg = t('client.saved');
      warnOrOk(result, t('client.saved'));
    } catch (e) {
      macroError = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: macroError });
    } finally {
      savingMacros = false;
    }
  }

  const activeMacro = $derived(macros[activeMacroIndex] ?? null);
</script>

<svelte:head>
  <title>{t('client.tag')}</title>
</svelte:head>

<div class="client-suite" in:flyY={{ y: 8, duration: 180 }}>
  <!-- Page Header -->
  <header class="suite-header">
    <div class="suite-header__main">
      <div class="suite-header__tagline">
        <span class="suite-header__dot"></span>
        <span class="font-pixel text-xs">ESPECTRAL IN-GAME MATRIX</span>
      </div>
      <h1 class="suite-title">
        <GradientText tone="cyan">{t('client.title')}</GradientText>
      </h1>
      <p class="suite-subhead muted">
        {t('client.subhead')}
      </p>
    </div>

    <div class="suite-header__actions">
      {#if selectedInstance}
        <div class="suite-status-chip glass-panel">
          <span class="suite-status-dot"></span>
          <span class="font-mono text-xs">{selectedInstance.name}</span>
          <LoaderBadge loader={selectedInstance.loader} version={selectedInstance.version} size="sm" />
        </div>
      {/if}
      <Btn variant="ghost" size="sm" onclick={() => load(false)} title={t('client.reload')}>
        {#snippet icon()}
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="23 4 23 10 17 10" />
            <polyline points="1 20 1 14 7 14" />
            <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
          </svg>
        {/snippet}
        {t('client.reload')}
      </Btn>
    </div>
  </header>

  <!-- Instance Strip Selector -->
  <section class="instance-strip-card glass-panel" aria-label="Instance selector">
    <div class="instance-strip-header">
      <span class="instance-strip-label">{t('client.instance')}</span>
      {#if info && info.supported}
        <Badge tone="ok" dot>{t('client.statusReady')}</Badge>
      {:else if info && !info.supported}
        <Badge tone="warn" dot>{t('client.managed')}</Badge>
      {/if}
    </div>

    {#if $instances.value.length === 0}
      <div class="instance-strip-empty">
        <p class="muted">{t('client.createFirst')}</p>
        <Btn variant="primary" size="sm" onclick={() => (window.location.hash = '#/instances')}>
          {t('nav.instances') || 'Instancias'}
        </Btn>
      </div>
    {:else}
      <div class="instance-chips-scroll">
        {#each $instances.value as inst (inst.name)}
          {@const isSelected = inst.name === targetInstance}
          <button
            type="button"
            class="instance-chip"
            class:selected={isSelected}
            onclick={() => {
              targetInstance = inst.name;
            }}
          >
            <span class="instance-chip__glow"></span>
            <span class="instance-chip__name">{inst.name}</span>
            <span class="instance-chip__tag">{inst.loader} · {inst.version}</span>
            {#if isSelected}
              <span class="instance-chip__active-dot"></span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <!-- Main Content Layout -->
  {#if loading}
    <div class="loading-state glass-panel">
      <div class="spinner-pulse"></div>
      <p class="muted">{t('client.loading')}</p>
    </div>
  {:else if error}
    <div class="error-banner glass-panel">
      <span class="error-icon">!</span>
      <span>{error}</span>
      <Btn variant="ghost" size="sm" onclick={() => load(false)}>Reintentar</Btn>
    </div>
  {:else if info && !info.supported}
    <!-- Unsupported State Card -->
    <div class="unsupported-card glass-panel" in:fade={{ duration: 140 }}>
      <div class="unsupported-card__badge">⚠️</div>
      <h3>Instancia no compatible</h3>
      <p class="muted">{t('client.unsupported')}</p>
      <div class="unsupported-card__actions">
        <Btn variant="primary" size="md" onclick={() => (window.location.hash = '#/instances')}>
          {t('nav.instances') || 'Ver Instancias'}
        </Btn>
        <Btn variant="secondary" size="md" onclick={() => (window.location.hash = '#/mods')}>
          {t('mods.presetsTab') || 'Ver Presets de Mods'}
        </Btn>
      </div>
    </div>
  {:else if info}
    {#if patchError}
      <div class="error-banner glass-panel" in:flyY={{ y: -4, duration: 120 }}>
        <span>{patchError}</span>
      </div>
    {/if}

    <div class="console-grid">
      <!-- Left Column: In-Game Feature Controls -->
      <section class="console-column">
        <GlassCard elevation="md">
          <div class="card-header-deck">
            <div class="card-title-group">
              <span class="card-icon card-icon--cyan">⚡</span>
              <div>
                <h3 class="card-title">{t('client.features')}</h3>
                <p class="card-subhead muted">{t('client.featuresSub')}</p>
              </div>
            </div>
            <Badge tone="neutral" mono>{info.registry.length} {t('client.featuresCount', { count: info.registry.length })}</Badge>
          </div>

          <div class="features-list">
            {#each info.registry as feat (feat.id)}
              {@const on = featureEnabled(feat.id, feat.defaultEnabled)}
              {@const isToggling = togglingFeatureId === feat.id}
              <div class="feature-item glass-panel" class:feature-item--on={on}>
                <div class="feature-item__main">
                  <div class="feature-item__top">
                    <span class="feature-item__name">{feat.name}</span>
                    <Badge tone={feat.kind === 'managed' ? 'warn' : 'ok'} size="sm">
                      {feat.kind === 'managed' ? t('client.managed') : t('client.owned')}
                    </Badge>
                  </div>
                  <p class="feature-item__desc muted">{feat.description}</p>
                  <div class="feature-item__hint muted font-mono text-xs">
                    {feat.kind === 'managed' ? t('client.restartHint') : t('client.liveHint')}
                  </div>
                </div>

                <div class="feature-item__control">
                  <button
                    type="button"
                    class="switch"
                    class:on={on}
                    role="switch"
                    aria-checked={on}
                    aria-label={feat.name}
                    onclick={() => toggleFeature(feat, !on)}
                    disabled={isToggling}
                    title={feat.name}
                  >
                    <span class="switch__thumb"></span>
                  </button>
                </div>
              </div>
            {/each}
          </div>
        </GlassCard>
      </section>

      <!-- Right Column: Macro Automation Engine -->
      <section class="console-column">
        <GlassCard elevation="md" featured={macrosDirty}>
          <div class="card-header-deck">
            <div class="card-title-group">
              <span class="card-icon card-icon--purple">⌨</span>
              <div>
                <h3 class="card-title">{t('client.macros')}</h3>
                <p class="card-subhead muted">{t('client.macrosSub')}</p>
              </div>
            </div>
            <div class="macros-header-meta">
              {#if macrosDirty}
                <Badge tone="warn" dot>{t('client.unsavedChanges')}</Badge>
              {/if}
              <Badge tone="neutral" mono>{t('client.macroCount', { count: macros.length, max: MAX_MACROS })}</Badge>
            </div>
          </div>

          {#if macroError}
            <div class="error-banner glass-panel">
              <span>{macroError}</span>
            </div>
          {/if}

          {#if savedMsg}
            <div class="ok-banner glass-panel">
              <span>✓ {savedMsg}</span>
            </div>
          {/if}

          {#if macros.length === 0}
            <div class="empty-macros glass-panel">
              <div class="empty-macros__icon">⌨</div>
              <h4>{t('client.noMacros')}</h4>
              <p class="muted">Crea macros para lanzar comandos o mensajes con una sola pulsación de tecla.</p>
              <Btn variant="primary" size="sm" onclick={addMacro}>
                {t('client.add')}
              </Btn>
            </div>
          {:else}
            <!-- Macro Selector Pills -->
            <div class="macro-pills-bar">
              <div class="macro-pills-list">
                {#each macros as m, idx (m.id)}
                  <button
                    type="button"
                    class="macro-pill-btn"
                    class:active={idx === activeMacroIndex}
                    onclick={() => (activeMacroIndex = idx)}
                  >
                    <span class="font-mono">{m.name || m.id}</span>
                    {#if m.keybind}
                      <span class="macro-pill-key font-pixel text-xs">{m.keybind}</span>
                    {/if}
                  </button>
                {/each}
              </div>
              <button
                type="button"
                class="macro-pill-add"
                onclick={addMacro}
                disabled={macros.length >= MAX_MACROS}
                title={t('client.add')}
              >
                +
              </button>
            </div>

            <!-- Active Macro Configuration Inspector -->
            {#if activeMacro}
              <div class="active-macro-editor glass-panel" in:fade={{ duration: 100 }}>
                <div class="macro-editor-head">
                  <div class="macro-editor-fields">
                    <Field label={t('client.macroName')}>
                      <input
                        type="text"
                        class="macro-input"
                        placeholder={t('client.macroNamePlaceholder')}
                        bind:value={activeMacro.name}
                        maxlength={MAX_MACRO_NAME}
                        oninput={markDirty}
                      />
                    </Field>

                    <Field label={t('client.keybind')}>
                      <input
                        type="text"
                        class="macro-input font-mono"
                        class:capturing={capturingKeybind}
                        placeholder={capturingKeybind ? t('client.keybindCapture') : t('client.keybindPlaceholder')}
                        value={activeMacro.keybind}
                        readonly
                        maxlength={MAX_KEYBIND}
                        onfocus={() => (capturingKeybind = true)}
                        onblur={() => (capturingKeybind = false)}
                        onclick={() => (capturingKeybind = true)}
                        onkeydown={handleKeybindCapture}
                      />
                    </Field>
                  </div>

                  <Btn
                    variant="danger"
                    size="sm"
                    onclick={() => removeMacro(activeMacroIndex)}
                    title={t('client.remove')}
                  >
                    {#snippet icon()}
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <polyline points="3 6 5 6 21 6" />
                        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                      </svg>
                    {/snippet}
                    {t('client.remove')}
                  </Btn>
                </div>

                <!-- Actions Sequence Feed -->
                <div class="actions-sequence-deck">
                  <div class="actions-deck-header">
                    <span class="actions-deck-label">{t('client.actions')} ({activeMacro.actions.length} / {MAX_ACTIONS})</span>
                    <Btn
                      variant="ghost"
                      size="sm"
                      onclick={() => addAction(activeMacro)}
                      disabled={activeMacro.actions.length >= MAX_ACTIONS}
                    >
                      + {t('client.addAction')}
                    </Btn>
                  </div>

                  <div class="actions-list">
                    {#each activeMacro.actions as action, ai}
                      <div class="action-row glass-panel">
                        <div class="action-row__top">
                          <span class="action-index font-pixel text-xs">{ai + 1}</span>
                          <select
                            class="action-type-select"
                            bind:value={action.type}
                            onchange={markDirty}
                          >
                            <option value="chat">{t('client.actionChat')}</option>
                            <option value="command">{t('client.actionCommand')}</option>
                          </select>
                          <button
                            type="button"
                            class="action-remove-btn"
                            onclick={() => removeAction(activeMacro, ai)}
                            disabled={activeMacro.actions.length <= 1}
                            title={t('client.remove')}
                          >
                            ×
                          </button>
                        </div>
                        <input
                          type="text"
                          class="action-text-input action-text-input--full"
                          placeholder={action.type === 'command' ? t('client.actionCommandPlaceholder') : t('client.actionChatPlaceholder')}
                          bind:value={action.text}
                          maxlength={MAX_ACTION_TEXT}
                          oninput={markDirty}
                        />
                      </div>
                    {/each}
                  </div>
                </div>
              </div>
            {/if}
          {/if}

          {#snippet footerSnippet()}
            <div class="macros-card-footer">
              <Btn
                variant="secondary"
                size="md"
                onclick={addMacro}
                disabled={macros.length >= MAX_MACROS}
              >
                {t('client.add')}
              </Btn>
              <Btn
                variant="primary"
                size="md"
                onclick={saveMacros}
                loading={savingMacros}
                disabled={!macrosDirty || !macrosValid || savingMacros}
              >
                {t('client.save')}
              </Btn>
            </div>
          {/snippet}
        </GlassCard>
      </section>
    </div>
  {/if}
</div>

<style>
  /* ==========================================================================
     ClientSuite.svelte Styles & Dual-Column Glass Layout
     ========================================================================== */
  .client-suite {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 1rem);
    width: 100%;
    max-width: var(--content-max, 82rem);
    margin: 0 auto;
    padding: var(--space-6, 24px) var(--space-6, 24px) var(--space-12, 48px);
  }

  /* Header */
  .suite-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 1rem);
    flex-wrap: wrap;
  }

  .suite-header__tagline {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--accent-cyan, #06b6d4);
    margin-bottom: 0.25rem;
  }

  .suite-header__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-cyan, #06b6d4);
    box-shadow: 0 0 8px var(--accent-cyan, #06b6d4);
  }

  .suite-title {
    font-size: var(--text-2xl, 1.75rem);
    font-weight: 700;
    line-height: var(--leading-tight, 1.2);
  }

  .suite-subhead {
    font-size: var(--text-sm, 0.875rem);
    margin-top: 0.25rem;
  }

  .suite-header__actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .suite-status-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
  }

  .suite-status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    box-shadow: 0 0 6px var(--accent, #10b981);
  }

  /* Instance Strip */
  .instance-strip-card {
    padding: var(--space-3, 0.75rem) var(--space-4, 1rem);
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 0.5rem);
  }

  .instance-strip-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .instance-strip-label {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
  }

  .instance-chips-scroll {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    overflow-x: auto;
    padding-bottom: 2px;
    scrollbar-width: thin;
  }

  .instance-chip {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.85rem;
    border-radius: var(--radius-md, 0.5rem);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    color: var(--text, #e8ecf4);
    font-size: 0.8125rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: all var(--transition-fast, 0.15s);
  }

  .instance-chip:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.3);
  }

  .instance-chip.selected {
    background: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.12);
    border-color: var(--accent-cyan, #06b6d4);
    color: #ffffff;
    box-shadow: 0 0 16px rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.2);
  }

  .instance-chip__tag {
    font-size: 0.6875rem;
    font-weight: 400;
    color: var(--muted, #8e9eb8);
  }

  .instance-chip.selected .instance-chip__tag {
    color: rgba(255, 255, 255, 0.75);
  }

  .instance-chip__active-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-cyan, #06b6d4);
    box-shadow: 0 0 6px var(--accent-cyan, #06b6d4);
  }

  .instance-strip-empty {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0;
  }

  /* Console Grid */
  .console-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-4, 1rem);
    align-items: start;
  }

  @media (max-width: 1024px) {
    .console-grid {
      grid-template-columns: 1fr;
    }
  }

  .console-column {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  /* Card Head Elements */
  .card-header-deck {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-4, 1rem);
  }

  .card-title-group {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .card-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-md, 0.5rem);
    font-size: 1rem;
  }

  .card-icon--cyan {
    background: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.15);
    border: 1px solid rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.3);
  }

  .card-icon--purple {
    background: rgba(var(--accent-purple-rgb, 168, 85, 247), 0.15);
    border: 1px solid rgba(var(--accent-purple-rgb, 168, 85, 247), 0.3);
  }

  .card-title {
    font-size: 1rem;
    font-weight: 700;
  }

  .card-subhead {
    font-size: 0.75rem;
  }

  .macros-header-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  /* Features List */
  .features-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .feature-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    transition: all var(--transition-fast, 0.15s);
  }

  .feature-item--on {
    border-color: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.3);
  }

  .feature-item__main {
    flex: 1;
    min-width: 0;
  }

  .feature-item__top {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .feature-item__name {
    font-size: 0.875rem;
    font-weight: 600;
  }

  .feature-item__desc {
    font-size: 0.8125rem;
    margin-top: 0.2rem;
    line-height: var(--leading-body, 1.6);
  }

  .feature-item__hint {
    margin-top: 0.25rem;
    opacity: 0.8;
  }

  .feature-item__control {
    flex-shrink: 0;
  }

  /* Macros Section */
  .empty-macros {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .empty-macros__icon {
    font-size: 2rem;
  }

  .macro-pills-bar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-bottom: 0.75rem;
  }

  .macro-pills-list {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex: 1;
    overflow-x: auto;
    scrollbar-width: thin;
    padding-bottom: 2px;
  }

  .macro-pill-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    color: var(--muted, #8e9eb8);
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: all var(--transition-fast, 0.15s);
  }

  .macro-pill-btn:hover {
    color: var(--text, #e8ecf4);
    background: rgba(255, 255, 255, 0.08);
  }

  .macro-pill-btn.active {
    background: rgba(var(--accent-purple-rgb, 168, 85, 247), 0.15);
    border-color: var(--accent-purple, #a855f7);
    color: #ffffff;
  }

  .macro-pill-key {
    background: rgba(0, 0, 0, 0.3);
    padding: 0.1rem 0.3rem;
    border-radius: 3px;
    color: var(--accent-gold, #ffd700);
  }

  .macro-pill-add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    color: var(--text, #e8ecf4);
    font-size: 1.1rem;
    cursor: pointer;
    flex-shrink: 0;
  }

  .macro-pill-add:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  /* Active Macro Inspector */
  .active-macro-editor {
    padding: var(--space-4, 1rem);
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .macro-editor-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .macro-editor-fields {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex: 1;
    flex-wrap: wrap;
  }

  .macro-input {
    width: 100%;
    min-width: 180px;
    padding: 0.4rem 0.75rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    color: var(--text, #e8ecf4);
    font-size: 0.8125rem;
  }

  .macro-input:focus {
    border-color: var(--accent-purple, #a855f7);
    outline: none;
  }

  .macro-input.capturing {
    border-color: var(--accent-cyan, #22d3ee);
    box-shadow: 0 0 0 1px var(--accent-cyan, #22d3ee);
    cursor: pointer;
  }

  /* Actions Sequence */
  .actions-sequence-deck {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    border-top: 1px solid var(--border, rgba(255, 255, 255, 0.06));
    padding-top: 0.75rem;
  }

  .actions-deck-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .actions-deck-label {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--muted, #8e9eb8);
  }

  .actions-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .action-row {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.5rem;
    padding: 0.5rem 0.6rem;
  }

  .action-row__top {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .action-index {
    color: var(--muted, #8e9eb8);
    width: 16px;
    text-align: center;
  }

  .action-type-select {
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    color: var(--text, #e8ecf4);
    font-size: 0.75rem;
  }

  .action-text-input {
    flex: 1;
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-sm, 0.375rem);
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.12));
    color: var(--text, #e8ecf4);
    font-size: 0.8125rem;
  }

  .action-text-input:focus {
    border-color: var(--accent-purple, #a855f7);
    outline: none;
  }

  .action-text-input--full {
    width: 100%;
    min-height: 2.5rem;
    padding: 0.6rem 0.75rem;
  }

  .action-remove-btn {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm, 0.375rem);
    background: transparent;
    border: none;
    color: var(--muted, #8e9eb8);
    font-size: 1.1rem;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .action-remove-btn:hover:not(:disabled) {
    color: var(--accent-red, #ef4444);
  }

  .action-remove-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .macros-card-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  /* Unsupported Card */
  .unsupported-card {
    padding: var(--space-8, 2rem) var(--space-4, 1rem);
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }

  .unsupported-card__badge {
    font-size: 2.5rem;
  }

  .unsupported-card__actions {
    display: flex;
    gap: 0.75rem;
    margin-top: 0.5rem;
  }

  /* Universal States */
  .loading-state,
  .error-banner,
  .ok-banner {
    padding: 0.75rem 1rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .error-banner {
    background: rgba(var(--accent-red-rgb, 239, 68, 68), 0.1);
    border-color: rgba(var(--accent-red-rgb, 239, 68, 68), 0.3);
    color: var(--accent-red, #ef4444);
    font-size: 0.8125rem;
  }

  .ok-banner {
    background: rgba(var(--accent-green-rgb, 34, 197, 94), 0.1);
    border-color: rgba(var(--accent-green-rgb, 34, 197, 94), 0.3);
    color: var(--accent-green, #22c55e);
    font-size: 0.8125rem;
  }

  .spinner-pulse {
    width: 16px;
    height: 16px;
    border: 2px solid var(--accent-cyan, #06b6d4);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Switch Toggle Component */
  .switch {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 42px;
    height: 24px;
    padding: 2px;
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.14));
    border-radius: 9999px;
    cursor: pointer;
    transition: background-color 0.2s cubic-bezier(0.16, 1, 0.3, 1),
      border-color 0.2s cubic-bezier(0.16, 1, 0.3, 1),
      box-shadow 0.2s ease;
    flex-shrink: 0;
  }

  .switch:focus-visible {
    outline: 2px solid var(--accent-cyan, #06b6d4);
    outline-offset: 2px;
  }

  .switch.on {
    background: var(--accent-cyan, #06b6d4);
    border-color: rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.8);
    box-shadow: 0 0 12px rgba(var(--accent-cyan-rgb, 6, 182, 212), 0.35);
  }

  .switch__thumb {
    width: 18px;
    height: 18px;
    background: #ffffff;
    border-radius: 50%;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.4);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
    transform: translateX(0);
  }

  .switch.on .switch__thumb {
    transform: translateX(18px);
  }

  .switch:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
</style>
