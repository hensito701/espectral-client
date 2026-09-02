<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { instances, liveLaunches, launchLog } from '../lib/stores';
  import {
    launchInstance,
    stopInstance,
    openFolder,
    getConfig,
  } from '../lib/api';
  import type { InstanceSummary, LiveLaunch } from '../lib/types';
  import { theme, resolveTheme } from '../lib/theme.svelte';
  import { lang, t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';

  interface CommandItem {
    id: string;
    category: 'recent' | 'running' | 'instances' | 'navigation' | 'actions';
    title: string;
    description: string;
    iconKind: 'instance' | 'play' | 'stop' | 'nav' | 'action' | 'theme' | 'lang' | 'folder' | 'wizard' | 'logs';
    badge: string;
    badgeType: 'launch' | 'nav' | 'action' | 'running' | 'default';
    keywords: string[];
    run: () => Promise<void> | void;
  }

  const STORAGE_KEY_RECENT = 'espectral_recent_commands';
  const MAX_RECENT = 6;

  let isOpen = $state(false);
  let rawQuery = $state('');
  let debouncedQuery = $state('');
  let highlightedIndex = $state(0);
  let recentIds = $state<string[]>([]);
  let inputEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Load recent commands from localStorage
  onMount(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY_RECENT);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed)) {
          recentIds = parsed.slice(0, MAX_RECENT);
        }
      }
    } catch {
      recentIds = [];
    }

    const onCustomOpen = () => openPalette();
    window.addEventListener('horizon:open-command-palette', onCustomOpen);

    const onGlobalKeyDown = (e: KeyboardEvent) => {
      // Ctrl+K / Cmd+K toggle
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        e.stopPropagation();
        togglePalette();
        return;
      }

      // '/' when no interactive element is focused
      if (e.key === '/' && !isOpen) {
        const target = e.target as HTMLElement | null;
        const tag = target?.tagName?.toLowerCase();
        const isEditable = target?.isContentEditable || tag === 'input' || tag === 'textarea' || tag === 'select';
        if (!isEditable) {
          e.preventDefault();
          e.stopPropagation();
          openPalette();
          return;
        }
      }

      // Global Escape handling when palette is open
      if (e.key === 'Escape' && isOpen) {
        e.preventDefault();
        e.stopPropagation();
        closePalette();
      }
    };

    window.addEventListener('keydown', onGlobalKeyDown, true);

    return () => {
      window.removeEventListener('horizon:open-command-palette', onCustomOpen);
      window.removeEventListener('keydown', onGlobalKeyDown, true);
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  });

  // Debounce query
  $effect(() => {
    const q = rawQuery;
    if (debounceTimer) clearTimeout(debounceTimer);
    if (!q.trim()) {
      debouncedQuery = '';
      highlightedIndex = 0;
    } else {
      debounceTimer = setTimeout(() => {
        debouncedQuery = q;
        highlightedIndex = 0;
      }, 150);
    }
  });

  function recordRecent(id: string): void {
    try {
      const next = [id, ...recentIds.filter((x) => x !== id)].slice(0, MAX_RECENT);
      recentIds = next;
      localStorage.setItem(STORAGE_KEY_RECENT, JSON.stringify(next));
    } catch {
      // ignore storage errors
    }
  }

  export function openPalette(): void {
    isOpen = true;
    rawQuery = '';
    debouncedQuery = '';
    highlightedIndex = 0;
    void tick().then(() => {
      inputEl?.focus();
      inputEl?.select();
    });
  }

  export function closePalette(): void {
    isOpen = false;
    rawQuery = '';
    debouncedQuery = '';
    highlightedIndex = 0;
  }

  function togglePalette(): void {
    if (isOpen) {
      closePalette();
    } else {
      openPalette();
    }
  }

  // Build command catalogue
  const instanceList = $derived<InstanceSummary[]>($instances.value ?? []);
  const runningList = $derived<LiveLaunch[]>($liveLaunches.value ?? []);

  const allCommands = $derived.by<CommandItem[]>(() => {
    const items: CommandItem[] = [];

    // 1. Running launches (actions to stop)
    for (const r of runningList) {
      if (r.running) {
        items.push({
          id: `stop-${r.instance}`,
          category: 'running',
          title: t('command.action.stopInstance', { name: r.instance }),
          description: t('command.action.stopInstanceDesc'),
          iconKind: 'stop',
          badge: t('command.badge.running'),
          badgeType: 'running',
          keywords: ['detener', 'stop', 'cerrar', 'kill', 'parar', r.instance],
          run: async () => {
            try {
              await stopInstance(r.instance);
              pushToast({ kind: 'info', text: t('pip.stopSuccess') });
            } catch (err) {
              pushToast({ kind: 'err', text: String(err) });
            }
          },
        });
      }
    }

    // 2. Instances (View and Launch)
    for (const inst of instanceList) {
      // View instance details
      items.push({
        id: `view-inst-${inst.name}`,
        category: 'instances',
        title: t('command.action.viewInstance', { name: inst.name }),
        description: `${inst.version} · ${inst.loader}`,
        iconKind: 'instance',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: [inst.name, inst.version, inst.loader, 'instancia', 'ver', 'configurar', 'mods'],
        run: () => {
          window.location.hash = `#/instances/${encodeURIComponent(inst.name)}`;
        },
      });

      // Quick Launch instance
      items.push({
        id: `launch-inst-${inst.name}`,
        category: 'instances',
        title: t('command.action.launchInstance', { name: inst.name, version: inst.version, loader: inst.loader }),
        description: t('command.action.launchInstanceDesc', { version: inst.version, loader: inst.loader }),
        iconKind: 'play',
        badge: t('command.badge.launch'),
        badgeType: 'launch',
        keywords: [inst.name, inst.version, inst.loader, 'lanzar', 'jugar', 'iniciar', 'play', 'launch', 'start'],
        run: async () => {
          try {
            const res = await launchInstance(inst.name, { mode: 'normal', dry_run: false });
            if ('key' in res && res.key) {
              await launchLog.start(res.key, { instance: inst.name });
              pushToast({
                kind: 'ok',
                text: t('command.action.launchInstance', { name: inst.name, version: inst.version, loader: inst.loader }),
              });
            }
          } catch (err) {
            pushToast({ kind: 'err', text: String(err) });
          }
        },
      });
    }

    // 3. Navigation
    items.push(
      {
        id: 'nav-home',
        category: 'navigation',
        title: t('command.nav.home'),
        description: t('command.nav.homeDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['inicio', 'home', 'carrusel', 'instancias', 'juegos', 'main'],
        run: () => {
          window.location.hash = '#/';
        },
      },
      {
        id: 'nav-servers',
        category: 'navigation',
        title: t('command.nav.servers'),
        description: t('command.nav.serversDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['servidores', 'servers', 'radar', 'multijugador', 'online', 'ping', 'espectral'],
        run: () => {
          window.location.hash = '#/servers';
        },
      },
      {
        id: 'nav-versions',
        category: 'navigation',
        title: t('command.nav.versions'),
        description: t('command.nav.versionsDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['versiones', 'versions', 'manifest', 'minecraft', 'release', 'snapshot', 'armory'],
        run: () => {
          window.location.hash = '#/library/versions';
        },
      },
      {
        id: 'nav-import',
        category: 'navigation',
        title: t('command.nav.import'),
        description: t('command.nav.importDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['importar', 'import', 'mrpack', 'modpack', 'lunar', 'fastclient', 'perfil'],
        run: () => {
          window.location.hash = '#/library/import';
        },
      },
      {
        id: 'nav-mods',
        category: 'navigation',
        title: t('command.nav.mods'),
        description: t('command.nav.modsDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['mods', 'modrinth', 'catalog', 'presets', 'performance', 'optifine', 'sodium'],
        run: () => {
          window.location.hash = '#/mods';
        },
      },
      {
        id: 'nav-client',
        category: 'navigation',
        title: t('command.nav.client'),
        description: t('command.nav.clientDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['cliente', 'client', 'suite', 'macros', 'fullbright', 'aot', 'optimizaciones'],
        run: () => {
          window.location.hash = '#/client';
        },
      },
      {
        id: 'nav-account',
        category: 'navigation',
        title: t('command.nav.account'),
        description: t('command.nav.accountDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['cuenta', 'account', 'microsoft', 'msa', 'offline', 'perfil', 'avatar', 'vault'],
        run: () => {
          window.location.hash = '#/account';
        },
      },
      {
        id: 'nav-settings',
        category: 'navigation',
        title: t('command.nav.settings'),
        description: t('command.nav.settingsDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['ajustes', 'settings', 'configuracion', 'jvm', 'memoria', 'java', 'jdk', 'idioma', 'tema'],
        run: () => {
          window.location.hash = '#/settings';
        },
      },
      {
        id: 'nav-donations',
        category: 'navigation',
        title: t('command.nav.donations'),
        description: t('command.nav.donationsDesc'),
        iconKind: 'nav',
        badge: t('command.badge.nav'),
        badgeType: 'nav',
        keywords: ['donaciones', 'donations', 'donar', 'apoyo', 'vip', 'donar', 'pabellon'],
        run: () => {
          window.location.hash = '#/donaciones';
        },
      },
    );

    // 4. System Actions
    items.push(
      {
        id: 'action-new-instance',
        category: 'actions',
        title: t('command.action.newInstance'),
        description: t('command.action.newInstanceDesc'),
        iconKind: 'wizard',
        badge: t('command.badge.action'),
        badgeType: 'action',
        keywords: ['crear', 'nueva', 'instancia', 'new', 'instance', 'wizard', 'agregar'],
        run: () => {
          window.location.hash = '#/';
          window.dispatchEvent(new CustomEvent('horizon:open-wizard', { bubbles: true }));
        },
      },
      {
        id: 'action-open-folder',
        category: 'actions',
        title: t('command.action.openFolder'),
        description: t('command.action.openFolderDesc'),
        iconKind: 'folder',
        badge: t('command.badge.action'),
        badgeType: 'action',
        keywords: ['carpeta', 'directorio', 'folder', 'data_dir', 'abrir', 'archivos', 'explorer'],
        run: async () => {
          try {
            const cfg = await getConfig();
            if (cfg?.data_dir) {
              await openFolder(cfg.data_dir);
              pushToast({ kind: 'ok', text: t('command.action.openFolder') });
            }
          } catch (err) {
            pushToast({ kind: 'err', text: String(err) });
          }
        },
      },
      {
        id: 'action-toggle-theme',
        category: 'actions',
        title: t('command.action.toggleTheme'),
        description: t('command.action.toggleThemeDesc'),
        iconKind: 'theme',
        badge: t('command.badge.action'),
        badgeType: 'action',
        keywords: ['tema', 'theme', 'oscuro', 'claro', 'dark', 'light', 'modo'],
        run: () => {
          const resolved = resolveTheme(theme.value);
          theme.set(resolved === 'dark' ? 'light' : 'dark');
        },
      },
      {
        id: 'action-toggle-lang',
        category: 'actions',
        title: t('command.action.toggleLang'),
        description: t('command.action.toggleLangDesc'),
        iconKind: 'lang',
        badge: t('command.badge.action'),
        badgeType: 'action',
        keywords: ['idioma', 'language', 'ingles', 'english', 'espanol', 'spanish', 'lang'],
        run: () => {
          lang.set(lang.value === 'es' ? 'en' : 'es');
        },
      },
      {
        id: 'action-toggle-logs',
        category: 'actions',
        title: t('command.action.toggleLogs'),
        description: t('command.action.toggleLogsDesc'),
        iconKind: 'logs',
        badge: t('command.badge.action'),
        badgeType: 'action',
        keywords: ['logs', 'registros', 'terminal', 'pip', 'consola', 'salida', 'drawer'],
        run: () => {
          window.dispatchEvent(new CustomEvent('horizon:toggle-pip-log', { bubbles: true }));
        },
      },
    );

    return items;
  });

  // Fuzzy Subsequence Scoring
  function fuzzyScore(query: string, target: string, keywords: string[]): number {
    const q = query.toLowerCase().trim();
    if (!q) return 1;
    const tStr = target.toLowerCase();
    const kw = keywords.map((k) => k.toLowerCase()).join(' ');
    const combined = `${tStr} ${kw}`;

    if (tStr === q) return 1000;
    if (tStr.startsWith(q)) return 500 + (100 - q.length);
    if (tStr.includes(q)) return 300 + (50 - tStr.indexOf(q));
    if (kw.includes(q)) return 200;

    let score = 0;
    let qIdx = 0;
    let prevMatchIdx = -1;
    let consecutiveCount = 0;

    for (let i = 0; i < combined.length && qIdx < q.length; i++) {
      if (combined[i] === q[qIdx]) {
        score += 10;
        if (prevMatchIdx === i - 1) {
          consecutiveCount++;
          score += consecutiveCount * 5;
        } else {
          consecutiveCount = 0;
        }
        if (i === 0 || combined[i - 1] === ' ' || combined[i - 1] === '-' || combined[i - 1] === '/') {
          score += 20;
        }
        prevMatchIdx = i;
        qIdx++;
      }
    }

    if (qIdx === q.length) {
      return score;
    }
    return 0;
  }

  // Filtered and sorted items
  const filteredItems = $derived.by<CommandItem[]>(() => {
    const q = debouncedQuery.trim();
    const all = allCommands;

    if (!q) {
      // Empty query state: Recent commands first (if any), then running, then navigation & actions
      const recentList: CommandItem[] = [];
      const recentSet = new Set<string>();

      for (const id of recentIds) {
        const item = all.find((x) => x.id === id);
        if (item && !recentSet.has(item.id)) {
          recentSet.add(item.id);
          recentList.push({
            ...item,
            category: 'recent',
          });
        }
      }

      const others = all.filter((x) => !recentSet.has(x.id));
      return [...recentList, ...others];
    }

    // Scored query filter
    const scored: { item: CommandItem; score: number }[] = [];
    for (const item of all) {
      const score = fuzzyScore(q, item.title, [item.description, ...item.keywords]);
      if (score > 0) {
        scored.push({ item, score });
      }
    }

    scored.sort((a, b) => b.score - a.score);
    return scored.map((s) => s.item);
  });

  // Group items by category for rendering headers
  const groupedSections = $derived.by<{ category: string; label: string; items: CommandItem[] }[]>(() => {
    const list = filteredItems;
    const groups: Record<string, CommandItem[]> = {};

    for (const item of list) {
      const cat = item.category;
      if (!groups[cat]) groups[cat] = [];
      groups[cat].push(item);
    }

    const order: { key: string; labelKey: string }[] = [
      { key: 'recent', labelKey: 'command.group.recent' },
      { key: 'running', labelKey: 'command.group.running' },
      { key: 'instances', labelKey: 'command.group.instances' },
      { key: 'navigation', labelKey: 'command.group.navigation' },
      { key: 'actions', labelKey: 'command.group.actions' },
    ];

    const result: { category: string; label: string; items: CommandItem[] }[] = [];
    for (const ord of order) {
      if (groups[ord.key] && groups[ord.key].length > 0) {
        result.push({
          category: ord.key,
          label: t(ord.labelKey),
          items: groups[ord.key],
        });
      }
    }

    return result;
  });

  // Execute selected item
  async function executeItem(item: CommandItem): Promise<void> {
    recordRecent(item.id);
    closePalette();
    try {
      await item.run();
    } catch (err) {
      pushToast({ kind: 'err', text: String(err) });
    }
  }

  // Keyboard navigation inside the palette
  function handleInputKeyDown(e: KeyboardEvent): void {
    const count = filteredItems.length;
    if (count === 0) {
      if (e.key === 'Escape') {
        closePalette();
      }
      return;
    }

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      highlightedIndex = (highlightedIndex + 1) % count;
      scrollHighlightedIntoView();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      highlightedIndex = (highlightedIndex - 1 + count) % count;
      scrollHighlightedIntoView();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const current = filteredItems[highlightedIndex];
      if (current) {
        void executeItem(current);
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closePalette();
    }
  }

  function scrollHighlightedIntoView(): void {
    void tick().then(() => {
      const activeEl = listEl?.querySelector('.cmd-item--active') as HTMLElement | null;
      if (activeEl) {
        activeEl.scrollIntoView({ block: 'nearest' });
      }
    });
  }

  function handleBackdropClick(e: MouseEvent): void {
    if (e.target === e.currentTarget) {
      closePalette();
    }
  }
</script>

{#if isOpen}
  <!-- Global Command Matrix Modal (z50) -->
  <div
    class="cmd-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label={t('command.placeholder')}
    tabindex="-1"
    onclick={handleBackdropClick}
    onkeydown={(e) => {
      if (e.key === 'Escape') closePalette();
    }}
  >
    <div class="cmd-modal" onclick={(e) => e.stopPropagation()} role="presentation">
      <!-- Minecraft In-Game Chat / Command Header -->
      <div class="cmd-input-bar">
        <span class="cmd-prompt-prefix" aria-hidden="true">&gt;</span>
        <input
          bind:this={inputEl}
          bind:value={rawQuery}
          type="text"
          class="cmd-input"
          placeholder={t('command.placeholder')}
          autocomplete="off"
          autocorrect="off"
          spellcheck="false"
          onkeydown={handleInputKeyDown}
        />
        <div class="cmd-chip-esc" aria-hidden="true">ESC</div>
      </div>

      <!-- Command Results Body -->
      <div class="cmd-body" bind:this={listEl}>
        {#if filteredItems.length === 0}
          <div class="cmd-empty">
            <div class="cmd-empty__icon" aria-hidden="true">⌨</div>
            <div class="cmd-empty__title">{t('command.noResults', { query: debouncedQuery })}</div>
            <div class="cmd-empty__hint">{t('command.noResultsHint')}</div>
          </div>
        {:else}
          {#each groupedSections as section (section.category)}
            <div class="cmd-section">
              <div class="cmd-section__header">{section.label}</div>
              <div class="cmd-section__items">
                {#each section.items as item (item.id)}
                  {@const isSelected = filteredItems[highlightedIndex]?.id === item.id}
                  <button
                    type="button"
                    class="cmd-item"
                    class:cmd-item--active={isSelected}
                    onclick={() => executeItem(item)}
                    onmouseenter={() => {
                      const idx = filteredItems.findIndex((x) => x.id === item.id);
                      if (idx !== -1) highlightedIndex = idx;
                    }}
                  >
                    <div class="cmd-item__left">
                      <!-- Icon glyph based on kind -->
                      <div class="cmd-item__icon cmd-item__icon--{item.iconKind}" aria-hidden="true">
                        {#if item.iconKind === 'instance'}
                          ⛏
                        {:else if item.iconKind === 'play'}
                          ▶
                        {:else if item.iconKind === 'stop'}
                          ⏹
                        {:else if item.iconKind === 'nav'}
                          ◈
                        {:else if item.iconKind === 'wizard'}
                          ✦
                        {:else if item.iconKind === 'folder'}
                          📁
                        {:else if item.iconKind === 'theme'}
                          ◐
                        {:else if item.iconKind === 'lang'}
                          🌐
                        {:else if item.iconKind === 'logs'}
                          📋
                        {:else}
                          ⚙
                        {/if}
                      </div>

                      <div class="cmd-item__meta">
                        <span class="cmd-item__title">{item.title}</span>
                        <span class="cmd-item__desc">{item.description}</span>
                      </div>
                    </div>

                    <div class="cmd-item__right">
                      <span class="cmd-badge cmd-badge--{item.badgeType} font-pixel">{item.badge}</span>
                    </div>
                  </button>
                {/each}
              </div>
            </div>
          {/each}
        {/if}
      </div>

      <!-- Retro HUD Command Footer -->
      <div class="cmd-footer">
        <div class="cmd-footer__hints">
          <span class="cmd-kbh"><kbd class="font-pixel">↑↓</kbd> {t('command.kbd.navigate')}</span>
          <span class="cmd-kbh"><kbd class="font-pixel">↵</kbd> {t('command.kbd.select')}</span>
          <span class="cmd-kbh"><kbd class="font-pixel">ESC</kbd> {t('command.kbd.close')}</span>
        </div>
        <div class="cmd-footer__brand font-pixel">ESPECTRAL HORIZON</div>
      </div>
    </div>
  </div>
{/if}

<style>
  .cmd-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-palette, 50);
    background: var(--backdrop);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: clamp(40px, 12vh, 120px);
    padding-left: var(--space-4);
    padding-right: var(--space-4);
    animation: cmdFadeIn var(--dur-fast) var(--ease-out-expo) both;
  }

  .cmd-modal {
    width: 100%;
    max-width: 640px;
    background: var(--card-bg, #0d1222);
    border: 1px solid var(--border-focus);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg), 0 0 32px rgba(var(--accent-rgb), 0.2);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: cmdPopIn var(--dur-fast) var(--ease-out-expo) both;
  }

  /* In-Game Chat Inspired Prompt Bar */
  .cmd-input-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5);
    background: var(--surface-solid);
    border-bottom: 1px solid var(--border);
  }

  .cmd-prompt-prefix {
    font-family: var(--font-mono);
    font-size: var(--text-xl);
    font-weight: 700;
    color: var(--accent);
    text-shadow: 0 0 8px rgba(var(--accent-rgb), 0.5);
    user-select: none;
    line-height: 1;
  }

  .cmd-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.4;
    padding: 0;
  }

  .cmd-input::placeholder {
    color: var(--muted);
    font-weight: 400;
  }

  .cmd-chip-esc {
    font-family: var(--font-mono-retro);
    font-size: 0.625rem;
    color: var(--muted);
    padding: 3px 6px;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    user-select: none;
  }

  /* Body & List */
  .cmd-body {
    max-height: 380px;
    overflow-y: auto;
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .cmd-section {
    margin-bottom: var(--space-2);
  }

  .cmd-section__header {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: var(--space-2) var(--space-3) var(--space-1);
  }

  .cmd-section__items {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  /* Command Item Button */
  .cmd-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease-out-expo),
                border-color var(--dur-fast) var(--ease-out-expo),
                transform var(--dur-fast) var(--ease-out-expo);
  }

  .cmd-item:hover,
  .cmd-item--active {
    background: var(--surface-up);
    border-color: var(--border-focus);
    box-shadow: 0 0 16px rgba(var(--accent-rgb), 0.12);
  }

  .cmd-item--active {
    transform: translateX(2px);
  }

  .cmd-item__left {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .cmd-item__icon {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    background: var(--surface-solid);
    border: 1px solid var(--border);
    font-size: 0.875rem;
    flex-shrink: 0;
    color: var(--muted-strong);
  }

  .cmd-item__icon--play {
    color: var(--accent);
    background: rgba(var(--accent-rgb), 0.1);
    border-color: rgba(var(--accent-rgb), 0.3);
  }

  .cmd-item__icon--stop {
    color: var(--accent-red);
    background: rgba(var(--accent-red-rgb), 0.1);
    border-color: rgba(var(--accent-red-rgb), 0.3);
  }

  .cmd-item__icon--wizard {
    color: var(--accent-gold);
    background: rgba(var(--accent-gold-rgb), 0.1);
    border-color: rgba(var(--accent-gold-rgb), 0.3);
  }

  .cmd-item__meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .cmd-item__title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cmd-item__desc {
    font-size: var(--text-xs);
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .cmd-item__right {
    margin-left: var(--space-3);
    flex-shrink: 0;
  }

  .cmd-badge {
    font-size: 0.5625rem;
    padding: 3px 6px;
    border-radius: var(--radius-xs);
    letter-spacing: 0.05em;
    user-select: none;
    line-height: 1;
    border: 1px solid transparent;
  }

  .cmd-badge--launch {
    color: var(--accent);
    background: rgba(var(--accent-rgb), 0.12);
    border-color: rgba(var(--accent-rgb), 0.35);
  }

  .cmd-badge--nav {
    color: var(--accent-cyan);
    background: rgba(var(--accent-cyan-rgb), 0.1);
    border-color: rgba(var(--accent-cyan-rgb), 0.3);
  }

  .cmd-badge--action {
    color: var(--accent-gold);
    background: rgba(var(--accent-gold-rgb), 0.1);
    border-color: rgba(var(--accent-gold-rgb), 0.3);
  }

  .cmd-badge--running {
    color: var(--accent-red);
    background: rgba(var(--accent-red-rgb), 0.15);
    border-color: rgba(var(--accent-red-rgb), 0.4);
    animation: cmdPulse 1.8s ease-in-out infinite;
  }

  .cmd-badge--default {
    color: var(--muted);
    background: var(--surface-solid);
    border-color: var(--border);
  }

  /* Empty State */
  .cmd-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-8) var(--space-4);
    text-align: center;
  }

  .cmd-empty__icon {
    font-size: 2rem;
    color: var(--muted);
    margin-bottom: var(--space-2);
    opacity: 0.6;
  }

  .cmd-empty__title {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text);
    margin-bottom: var(--space-1);
  }

  .cmd-empty__hint {
    font-size: var(--text-xs);
    color: var(--muted);
    max-width: 360px;
  }

  /* Footer */
  .cmd-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-5);
    background: var(--chrome-bg);
    border-top: 1px solid var(--border);
    user-select: none;
  }

  .cmd-footer__hints {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .cmd-kbh {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 0.6875rem;
    color: var(--muted);
  }

  .cmd-kbh kbd {
    font-size: 0.5625rem;
    padding: 2px 5px;
    background: var(--surface-solid);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    color: var(--muted-strong);
  }

  .cmd-footer__brand {
    font-size: 0.5625rem;
    color: var(--muted);
    letter-spacing: 0.05em;
    opacity: 0.5;
  }

  /* Keyframe Animations */
  @keyframes cmdFadeIn {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes cmdPopIn {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(-10px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  @keyframes cmdPulse {
    0%, 100% {
      opacity: 1;
    }
    50% {
      opacity: 0.6;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cmd-backdrop,
    .cmd-modal,
    .cmd-item,
    .cmd-badge--running {
      animation: none;
      transition: none;
      transform: none;
    }
  }
</style>
