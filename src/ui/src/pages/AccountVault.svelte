<!--
  ============================================================================
  AccountVault.svelte — Horizon Glass PSN Identity Switcher & Skin Pedestal
  Route: #/account
  ============================================================================
  Cinematic identity vault and profile pedestal. Features:
    - Hero Pedestal: Dynamic 3D avatar spotlight with ambient color shift,
      cryptographic signature, active status, last-played and created timestamps.
    - Account Roster Directory: Interactive account cards with instant switching,
      token badges (Microsoft/Offline/Lunar), and inline MSA sign-out confirmation.
    - Identity Forge: Instant offline account creation with validation and auto-switch.
    - Microsoft Device Flow (MSA): Polling loop honoring retry_after/pending,
      copy buttons for user_code and verification_uri, and Azure pending detection.
    - Advanced Azure Client ID: Collapsible configuration panel with get/set client ID.
    - Lunar Token Shield: Dedicated protection indicator and profile migration link.
-->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    getAccounts,
    createAccount,
    setActiveAccount,
    startMsLogin,
    pollMsLogin,
    logoutMsAccount,
    getMsClientId,
    setMsClientId,
    avatarUrl,
    uploadAccountAvatar,
    removeAccountAvatar,
    setAccountAvatarColor,
    getAccountSkin,
    skinPngUrl,
    resetAccountSkin,
    listLibrarySkins,
    librarySkinPngUrl,
    saveLibrarySkin,
    patchLibrarySkin,
    deleteLibrarySkin,
    applyLibrarySkin,
    importVanillaSkins,
    fileToPngDataUrl,
    UnsupportedImageError,
    ApiError,
  } from '../lib/api';
  import type { Account, MsDeviceFlow, SkinInfo, SkinLibraryEntry, SkinVariant } from '../lib/types';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import Field from '../components/Field.svelte';
  import GradientText from '../components/GradientText.svelte';
  import MonogramTile from '../components/MonogramTile.svelte';
  import SkinViewer from '../components/SkinViewer.svelte';
  import { timeAgo } from '../lib/format';
  import { t } from '../lib/i18n.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { useCopy } from '../lib/useCopy.svelte';
  import { flyY, fade, scalePop } from '../lib/motion';

  // --- Accounts State ---
  let accounts = $state<Account[]>([]);
  let activeUsername = $state<string>('');
  let loading = $state<boolean>(true);
  let refreshing = $state<boolean>(false);
  let loadError = $state<string>('');

  // Active account derived
  const activeAccount = $derived<Account | null>(
    accounts.find((a) => a.username === activeUsername) || accounts[0] || null,
  );

  // Sorting: active first, then last_used desc
  const sortedAccounts = $derived.by(() => {
    return [...accounts].sort((a, b) => {
      if (a.username === activeUsername) return -1;
      if (b.username === activeUsername) return 1;
      const tA = a.last_used ? Date.parse(a.last_used) : 0;
      const tB = b.last_used ? Date.parse(b.last_used) : 0;
      return tB - tA;
    });
  });

  // --- Offline Create Form State ---
  let newOfflineUsername = $state<string>('');
  let creatingOffline = $state<boolean>(false);
  let offlineError = $state<string>('');

  const nameValid = $derived(/^[A-Za-z0-9_]{2,16}$/.test(newOfflineUsername.trim()));

  // --- Microsoft (MSA) Device Flow State ---
  let msFlow = $state<MsDeviceFlow | null>(null);
  let msPolling = $state<boolean>(false);
  let msError = $state<string>('');
  let msPending = $state<boolean>(false);
  let msTimer: ReturnType<typeof setInterval> | null = null;

  // Active tab in Identity Forge ('offline' | 'msa')
  let forgeTab = $state<'offline' | 'msa'>('offline');

  // Inline logout confirmation state (account username being confirmed for logout)
  let confirmingLogoutUser = $state<string | null>(null);
  let loggingOut = $state<boolean>(false);
  // --- Advanced Azure Client ID State ---
  let showAdvancedClientId = $state<boolean>(false);
  let clientIdValue = $state<string>('');
  let loadingClientId = $state<boolean>(false);
  let savingClientId = $state<boolean>(false);
  let clientIdError = $state<string>('');
  // Copy helpers
  const codeCopier = useCopy(2000);
  const uriCopier = useCopy(2000);
  const uuidCopier = useCopy(2000);
  // Avatar customization state
  let avatarBust = $state(0);
  let avatarBusy = $state<string | null>(null);

  /** Preset accent hues offered for every account (6-8 swatches). */
  const AVATAR_HUES = [4, 32, 48, 145, 190, 222, 265, 315];

  const avatarSrc = (username: string, hasAvatar: boolean): string | undefined =>
    hasAvatar ? `${avatarUrl(username)}${avatarBust ? `?v=${avatarBust}` : ''}` : undefined;
  // --- Dynamic Ambient Lighting Shift (custom avatar color wins) ---
  function accountHue(username: string | undefined): number | null {
    const acc = accounts.find((a) => a.username === username);
    return typeof acc?.avatar_color === 'number' ? acc.avatar_color : null;
  }

  function updateAmbientForAccount(username: string | undefined): void {
    if (typeof document === 'undefined' || !username) return;
    const custom = accountHue(username);
    let hue1: number;
    if (custom !== null) {
      hue1 = ((Math.round(custom) % 360) + 360) % 360;
    } else {
      let hash = 0;
      for (let i = 0; i < username.length; i++) {
        hash = (hash << 5) - hash + username.charCodeAt(i);
        hash |= 0;
      }
      hue1 = Math.abs(hash % 360);
    }
    const hue2 = (hue1 + 45) % 360;
    document.documentElement.style.setProperty('--ambient-1', `hsla(${hue1}, 70%, 50%, 0.14)`);
    document.documentElement.style.setProperty('--ambient-2', `hsla(${hue2}, 80%, 45%, 0.09)`);
  }
  async function handleAvatarColor(username: string, color: number | null): Promise<void> {
    if (avatarBusy) return;
    avatarBusy = username;
    try {
      await setAccountAvatarColor(username, color);
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.colorSaved') });
    } catch (e) {
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: e instanceof Error ? e.message : String(e) }) });
    } finally {
      avatarBusy = null;
    }
  }

  async function handleAvatarFile(username: string, e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement | null;
    const file = input?.files?.[0];
    if (!file || avatarBusy) return;
    avatarBusy = username;
    try {
      const dataUrl = await fileToPngDataUrl(file, 256);
      await uploadAccountAvatar(username, dataUrl);
      avatarBust += 1;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.avatarSaved') });
    } catch (err) {
      const text =
        err instanceof UnsupportedImageError
          ? t('vault.avatarUnsupported', { label: err.label, name: err.fileName })
          : t('vault.errorAction', { error: err instanceof Error ? err.message : String(err) });
      pushToast({ kind: 'err', text });
    } finally {
      avatarBusy = null;
      if (input) input.value = '';
    }
  }

  async function handleRemoveAvatar(username: string): Promise<void> {
    if (avatarBusy) return;
    avatarBusy = username;
    try {
      await removeAccountAvatar(username);
      avatarBust += 1;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.avatarRemoved') });
    } catch (err) {
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: err instanceof Error ? err.message : String(err) }) });
    } finally {
      avatarBusy = null;
    }
  }

  // --- Skin Atelier state (component-local; per active account) ---
  let skinInfo = $state<SkinInfo | null>(null);
  let skinLoading = $state<boolean>(false);
  let skinBusy = $state<boolean>(false);
  let skinBust = $state(0);
  let skinVariant = $state<SkinVariant>('classic');
  let stagedSkin = $state<string | null>(null);
  let stagedName = $state<string>('');
  let confirmingSkinReset = $state<boolean>(false);
  // Gallery (global per installation, like the vanilla launcher).
  let gallery = $state<SkinLibraryEntry[]>([]);
  let galleryLoading = $state<boolean>(false);
  let galleryBust = $state(0);
  let previewEntryId = $state<string | null>(null);
  let skinStageEl: HTMLDivElement | null = $state(null);
  let applyingId = $state<string | null>(null);
  let deletingId = $state<string | null>(null);
  let renamingId = $state<string | null>(null);
  let confirmingDeleteId = $state<string | null>(null);
  let renameValue = $state<string>('');
  let importingVanilla = $state<boolean>(false);

  const isOfflineActive = $derived(activeAccount !== null && activeAccount.token_kind !== 'msa');

  const skinUrl = $derived<string | undefined>(
    activeAccount && activeAccount.token_kind === 'msa' && skinInfo?.has_skin
      ? `${skinPngUrl(activeAccount.username)}${skinBust ? `?v=${skinBust}` : ''}`
      : undefined,
  );

  const previewEntry = $derived<SkinLibraryEntry | null>(
    previewEntryId ? (gallery.find((g) => g.id === previewEntryId) ?? null) : null,
  );

  const previewEntryUrl = $derived<string | undefined>(
    previewEntry ? `${librarySkinPngUrl(previewEntry.id)}${galleryBust ? `?v=${galleryBust}` : ''}` : undefined,
  );

  // Staged file wins, then the gallery preview, then the live Mojang skin.
  const viewerUrl = $derived<string | undefined>(stagedSkin ?? previewEntryUrl ?? skinUrl);

  const viewerVariant = $derived<SkinVariant>(
    stagedSkin ? skinVariant : (previewEntry?.variant ?? skinVariant),
  );

  async function loadSkin(username: string): Promise<void> {
    skinLoading = true;
    try {
      const info = await getAccountSkin(username);
      // Race guard: only apply when still looking at the same account.
      if (activeAccount?.username === username) {
        skinInfo = info;
        skinVariant = info.variant;
      }
    } catch (e) {
      if (activeAccount?.username === username) skinInfo = null;
      // Offline accounts answer 404 OFFLINE_ACCOUNT — the section renders the
      // offline note instead, so only toast genuine failures.
      if (!(e instanceof ApiError && e.code === 'OFFLINE_ACCOUNT')) {
        pushToast({
          kind: 'err',
          text: t('vault.skin.loadError', { error: e instanceof Error ? e.message : String(e) }),
        });
      }
    } finally {
      skinLoading = false;
    }
  }

  async function handleSkinFile(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement | null;
    const file = input?.files?.[0];
    if (!file || skinBusy) return;
    skinBusy = true;
    try {
      stagedSkin = await fileToPngDataUrl(file, 256);
      previewEntryId = null;
      if (!stagedName.trim()) {
        stagedName = file.name.replace(/\.[^.]+$/, '').slice(0, 40);
      }
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      skinBusy = false;
      if (input) input.value = '';
    }
  }

  function clearStagedSkin(): void {
    stagedSkin = null;
    stagedName = '';
  }

  async function loadGallery(): Promise<void> {
    galleryLoading = true;
    try {
      gallery = await listLibrarySkins();
      if (previewEntryId && !gallery.some((g) => g.id === previewEntryId)) {
        previewEntryId = null;
      }
    } catch (e) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: e instanceof Error ? e.message : String(e) }),
      });
    } finally {
      galleryLoading = false;
    }
  }

  async function handleSaveToGallery(): Promise<void> {
    if (!stagedSkin || skinBusy) return;
    const name = stagedName.trim() || t('vault.skin.stageNamePlaceholder');
    skinBusy = true;
    try {
      const entry = await saveLibrarySkin(name, stagedSkin, skinVariant);
      await loadGallery();
      galleryBust += 1;
      stagedSkin = null;
      stagedName = '';
      previewEntryId = entry.id;
      pushToast({ kind: 'ok', text: t('vault.skin.gallerySaved', { name: entry.name }) });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      skinBusy = false;
    }
  }

  async function handleApplyEntry(id: string): Promise<void> {
    const username = activeAccount?.username;
    const entry = gallery.find((g) => g.id === id);
    if (!username || !entry || applyingId) return;
    applyingId = id;
    try {
      const res = await applyLibrarySkin(id, username);
      skinVariant = res.variant;
      skinBust += 1;
      await loadSkin(username);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.skin.galleryApplied', { name: entry.name, username }) });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      applyingId = null;
    }
  }

  async function handleDeleteEntry(id: string): Promise<void> {
    const entry = gallery.find((g) => g.id === id);
    if (!entry || deletingId) return;
    deletingId = id;
    try {
      await deleteLibrarySkin(id);
      if (previewEntryId === id) previewEntryId = null;
      await loadGallery();
      galleryBust += 1;
      pushToast({ kind: 'ok', text: t('vault.skin.galleryDeleted', { name: entry.name }) });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      deletingId = null;
    }
  }

  function promptDeleteEntry(id: string, e?: MouseEvent): void {
    e?.stopPropagation?.();
    renamingId = null;
    deletingId = null;
    confirmingDeleteId = id;
  }

  function cancelDeleteEntry(e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingDeleteId = null;
  }
  // Gallery "Probar" (and card-art click): stage the entry in the main 3D
  // preview — rotation happens there, nothing is sent to Mojang — then bring
  // the stage into view.
  function previewSkin(id: string): void {
    previewEntryId = id;
    skinStageEl?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  }

  function promptRenameEntry(entry: SkinLibraryEntry, e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingDeleteId = null;
    renamingId = entry.id;
    renameValue = entry.name;
  }

  function cancelRenameEntry(e?: MouseEvent): void {
    e?.stopPropagation?.();
    renamingId = null;
    renameValue = '';
  }

  async function confirmRenameEntry(id: string, e?: Event): Promise<void> {
    e?.stopPropagation?.();
    if (!renameValue.trim() || skinBusy) return;
    skinBusy = true;
    try {
      await patchLibrarySkin(id, { name: renameValue.trim() });
      renamingId = null;
      renameValue = '';
      await loadGallery();
      galleryBust += 1;
      pushToast({ kind: 'ok', text: t('vault.skin.galleryRenamed') });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      skinBusy = false;
    }
  }

  async function handleImportVanilla(): Promise<void> {
    if (importingVanilla) return;
    importingVanilla = true;
    try {
      const res = await importVanillaSkins();
      await loadGallery();
      galleryBust += 1;
      if (res.total === 0) {
        pushToast({ kind: 'ok', text: t('vault.skin.importVanillaNone') });
      } else {
        pushToast({ kind: 'ok', text: t('vault.skin.importVanillaDone', { imported: res.imported, skipped: res.skipped }) });
      }
    } catch (e) {
      if (e instanceof ApiError && e.code === 'NO_VANILLA_SKINS') {
        pushToast({ kind: 'err', text: t('vault.skin.importVanillaNone') });
      } else {
        pushToast({
          kind: 'err',
          text: t('vault.skin.loadError', { error: e instanceof Error ? e.message : String(e) }),
        });
      }
    } finally {
      importingVanilla = false;
    }
  }

  function promptSkinReset(e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingSkinReset = true;
  }

  function cancelSkinReset(e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingSkinReset = false;
  }

  async function confirmSkinReset(e?: MouseEvent): Promise<void> {
    e?.stopPropagation?.();
    const username = activeAccount?.username;
    if (!username || skinBusy) return;
    skinBusy = true;
    try {
      await resetAccountSkin(username);
      confirmingSkinReset = false;
      stagedSkin = null;
      skinBust += 1;
      await loadSkin(username);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.skin.resetDone') });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('vault.skin.loadError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      skinBusy = false;
    }
  }

  async function handleRefreshSkin(): Promise<void> {
    const username = activeAccount?.username;
    if (!username || skinLoading) return;
    skinBust += 1;
    await loadSkin(username);
  }

  // --- Account Loading & Selection ---
  async function loadAccounts(silent = false): Promise<void> {
    if (!silent) loading = true;
    else refreshing = true;
    loadError = '';
    try {
      const list = await getAccounts();
      accounts = list;
      const sorted = [...list].sort(
        (a, b) => Date.parse(b.last_used ?? '') - Date.parse(a.last_used ?? ''),
      );
      if (sorted.length > 0 && !activeUsername) {
        activeUsername = sorted[0].username;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorLoad', { error: loadError }) });
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  async function handleSetActive(username: string): Promise<void> {
    if (activeUsername === username) return;
    try {
      await setActiveAccount(username);
      activeUsername = username;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.switchSuccess', { name: username }) });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: msg }) });
    }
  }

  // --- Offline Create Action ---
  async function handleCreateOffline(): Promise<void> {
    if (!nameValid || creatingOffline) return;
    creatingOffline = true;
    offlineError = '';
    const name = newOfflineUsername.trim();
    try {
      const acc = await createAccount(name);
      await setActiveAccount(acc.username);
      activeUsername = acc.username;
      newOfflineUsername = '';
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: acc.username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.offlineSuccess', { name: acc.username }) });
    } catch (e) {
      offlineError = e instanceof Error ? e.message : String(e);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: offlineError }) });
    } finally {
      creatingOffline = false;
    }
  }

  // --- Microsoft (MSA) Device Flow Actions ---
  function scheduleMsPoll(delaySec: number): void {
    if (msTimer) clearInterval(msTimer);
    msTimer = setInterval(pollMs, Math.max(delaySec, 3) * 1000);
  }

  function isMsaPendingError(e: unknown): boolean {
    return e instanceof ApiError && e.code === 'MSA_DEVICE_FAILED';
  }

  async function startMs(): Promise<void> {
    msError = '';
    msPending = false;
    try {
      const flow = await startMsLogin();
      msFlow = flow;
      msPolling = true;
      scheduleMsPoll(flow.interval);
    } catch (e) {
      if (isMsaPendingError(e)) {
        msPending = true;
      } else {
        msError = e instanceof Error ? e.message : String(e);
        pushToast({ kind: 'err', text: msError });
      }
    }
  }

  async function pollMs(): Promise<void> {
    if (!msFlow) return;
    try {
      const result = await pollMsLogin(msFlow.flow_id);
      if ('pending' in result && result.pending) {
        scheduleMsPoll(result.retry_after ?? msFlow.interval ?? 5);
        return;
      }
      // Account created successfully
      if (msTimer) {
        clearInterval(msTimer);
        msTimer = null;
      }
      const acc = result as Account;
      msFlow = null;
      msPolling = false;
      await setActiveAccount(acc.username);
      activeUsername = acc.username;
      await loadAccounts(true);
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: acc.username } }),
        );
      }
      pushToast({ kind: 'ok', text: t('vault.msSuccess', { name: acc.username }) });
    } catch (e) {
      if (msTimer) {
        clearInterval(msTimer);
        msTimer = null;
      }
      msFlow = null;
      msPolling = false;
      if (isMsaPendingError(e)) {
        msPending = true;
      } else {
        msError = e instanceof Error ? e.message : String(e);
        pushToast({ kind: 'err', text: msError });
      }
    }
  }

  function cancelMs(): void {
    if (msTimer) {
      clearInterval(msTimer);
      msTimer = null;
    }
    msFlow = null;
    msPolling = false;
  }

  // --- Inline MSA Logout ---
  function promptLogout(username: string, e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingLogoutUser = username;
  }

  function cancelLogout(e?: MouseEvent): void {
    e?.stopPropagation?.();
    confirmingLogoutUser = null;
  }

  async function confirmLogout(username: string, e?: MouseEvent): Promise<void> {
    e?.stopPropagation?.();
    loggingOut = true;
    try {
      await logoutMsAccount(username);
      confirmingLogoutUser = null;
      await loadAccounts(true);
      if (activeUsername === username) {
        activeUsername = accounts[0]?.username || '';
        if (activeUsername) {
          await setActiveAccount(activeUsername);
        }
      }
      if (typeof window !== 'undefined') {
        window.dispatchEvent(
          new CustomEvent('horizon:account-changed', { detail: { username: activeUsername } }),
        );
      }
      pushToast({ kind: 'ok', text: `Microsoft account ${username} signed out.` });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      pushToast({ kind: 'err', text: t('vault.errorAction', { error: msg }) });
    } finally {
      loggingOut = false;
    }
  }

  // --- Advanced Client ID Management ---
  async function loadClientId(): Promise<void> {
    loadingClientId = true;
    clientIdError = '';
    try {
      const data = await getMsClientId();
      clientIdValue = data.client_id || '';
    } catch (err) {
      clientIdError = err instanceof Error ? err.message : String(err);
    } finally {
      loadingClientId = false;
    }
  }

  async function handleSaveClientId(): Promise<void> {
    savingClientId = true;
    clientIdError = '';
    try {
      await setMsClientId(clientIdValue.trim());
      pushToast({ kind: 'ok', text: t('vault.clientIdSuccess') });
    } catch (err) {
      clientIdError = err instanceof Error ? err.message : String(err);
      pushToast({ kind: 'err', text: clientIdError });
    } finally {
      savingClientId = false;
    }
  }

  function toggleAdvancedClientId(): void {
    showAdvancedClientId = !showAdvancedClientId;
    if (showAdvancedClientId && !clientIdValue) {
      void loadClientId();
    }
  }

  function navigateTo(hash: string): void {
    if (typeof window !== 'undefined') {
      window.location.hash = hash;
    }
  }

  // Effect to sync ambient light with active account
  $effect(() => {
    updateAmbientForAccount(activeAccount?.username);
  });

  // Reload skin metadata whenever the active identity changes. MSA only —
  // offline accounts keep the gallery, never a request that would 404.
  $effect(() => {
    const username = activeAccount?.username;
    const isMsa = activeAccount?.token_kind === 'msa';
    stagedSkin = null;
    stagedName = '';
    previewEntryId = null;
    confirmingSkinReset = false;
    confirmingDeleteId = null;
    renamingId = null;
    if (username && isMsa) {
      void loadSkin(username);
    } else {
      skinInfo = null;
      skinLoading = false;
    }
  });

  onMount(() => {
    void loadAccounts();
    void loadGallery();
  });

  onDestroy(() => {
    if (msTimer) {
      clearInterval(msTimer);
      msTimer = null;
    }
  });
</script>

<svelte:head>
  <title>{t('vault.tag')}</title>
</svelte:head>

<div class="account-vault-page">
  <!-- Top Command Header -->
  <header class="vault-hero">
    <div class="vault-hero__meta">
      <div class="vault-hero__badge-row">
        <Badge variant="accent" size="sm" dot={true}>
          {t('vault.badge')}
        </Badge>
        <span class="vault-hero__count-pill">
          {t('vault.rosterCount', { count: accounts.length })}
        </span>
      </div>

      <h1 class="vault-hero__title">
        <GradientText variant="emerald" size="2xl">
          {t('vault.title')}
        </GradientText>
      </h1>

      <p class="vault-hero__subtitle">
        {t('vault.subtitle')}
      </p>
    </div>

    <div class="vault-hero__actions">
      <Btn
        variant="secondary"
        size="md"
        loading={refreshing}
        onclick={() => loadAccounts(true)}
        ariaLabel={t('vault.refresh')}
      >
        {#snippet icon()}
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67" />
          </svg>
        {/snippet}
        {refreshing ? t('vault.refreshing') : t('vault.refresh')}
      </Btn>
    </div>
  </header>

  <!-- Hero Identity Pedestal Spotlight -->
  <section class="pedestal-section" aria-label="Active Identity Spotlight">
    <div class="pedestal-card">
      <div class="pedestal-glow" aria-hidden="true"></div>

      <div class="pedestal-layout">
        <!-- 3D Avatar Stage & Ring -->
        <div class="pedestal-stage">
          <div class="pedestal-ring" aria-hidden="true"></div>
          <div class="pedestal-avatar-wrapper">
            <MonogramTile
              name={activeAccount?.username || '?'}
              hue={activeAccount?.avatar_color ?? undefined}
              avatarUrl={activeAccount && activeAccount.has_avatar ? avatarSrc(activeAccount.username, true) : undefined}
              size={104}
              shape="rounded"
              className="pedestal-avatar"
            />
          </div>
          <div class="pedestal-floor" aria-hidden="true"></div>
        </div>

        <!-- Pedestal Identity Details -->
        <div class="pedestal-info">
          <div class="pedestal-info__top">
            <Badge variant="ok" dot={true} size="md">
              {t('vault.heroActiveBadge')}
            </Badge>

            {#if activeAccount?.token_kind === 'msa'}
              <Badge variant="accent" size="md">
                {t('vault.cardKindMsa')}
              </Badge>
            {:else}
              <Badge variant="neutral" size="md">
                {t('vault.cardKindOffline')}
              </Badge>
            {/if}
          </div>

          <div class="pedestal-username" title={activeAccount?.username || '—'}>
            {activeAccount?.username || t('vault.heroNoActive')}
          </div>

          <p class="pedestal-signature">
            <span class="shield-dot">✦</span>
            {t('vault.heroSkinHint')}
          </p>

          <!-- Pedestal Metadata Grid -->
          {#if activeAccount}
            <div class="pedestal-meta-grid">
              <div class="meta-item">
                <span class="meta-item__label">{t('account.lastUsed')}</span>
                <span class="meta-item__val">
                  {activeAccount.last_used ? timeAgo(activeAccount.last_used) : t('vault.heroNever')}
                </span>
              </div>

              <div class="meta-item">
                <span class="meta-item__label">{t('account.created')}</span>
                <span class="meta-item__val">
                  {activeAccount.created_at ? timeAgo(activeAccount.created_at) : '—'}
                </span>
              </div>

              <div class="meta-item meta-item--uuid">
                <span class="meta-item__label">UUID</span>
                <div class="uuid-copy-wrap">
                  <code class="uuid-code" title={activeAccount.uuid}>
                    {activeAccount.uuid.slice(0, 14)}…
                  </code>
                  <button
                    type="button"
                    class="uuid-copy-btn"
                    onclick={() => uuidCopier.copy(activeAccount?.uuid || '')}
                    title={t('vault.heroCopyUuid')}
                    aria-label={t('vault.heroCopyUuid')}
                  >
                    {#if uuidCopier.copied}
                      <span class="text-ok font-mono">✓</span>
                    {:else}
                      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                      </svg>
                    {/if}
                  </button>
                </div>
              </div>
            </div>
          {:else}
            <p class="pedestal-prompt">{t('vault.heroSelectPrompt')}</p>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <!-- Skin Atelier: 3D visualizer + named skin gallery (vanilla workflow) -->
  <section class="skin-atelier-section" aria-label="Skin Atelier">
    <GlassCard title={t('vault.skin.title')} subtitle={t('vault.skin.subtitle')} elevation="md">
      {#if !activeAccount}
        <p class="skin-atelier__note">{t('vault.heroSelectPrompt')}</p>
      {:else}
        <div class="skin-atelier__layout">
          <div class="skin-atelier__stage" bind:this={skinStageEl}>
            {#if skinLoading && !skinInfo && !viewerUrl}
              <div class="roster-loading">
                <span class="spinner"></span>
                <span>{t('vault.skin.viewerLoading')}</span>
              </div>
            {:else if viewerUrl}
              <SkinViewer skinUrl={viewerUrl} variant={viewerVariant} height={340} />
              {#if stagedSkin}
                <p class="skin-atelier__note">{t('vault.skin.galleryStaged')}</p>
              {:else if previewEntry}
                <Badge variant="accent" size="sm">
                  {t('vault.skin.galleryPreviewing')}: {previewEntry.name}
                </Badge>
              {:else if skinInfo?.cape}
                <Badge variant="accent" size="sm">{t('vault.skin.capeBadge')}</Badge>
              {/if}
            {:else}
              <p class="skin-atelier__note">{t('vault.skin.noSkin')}</p>
            {/if}
          </div>
          <div class="skin-atelier__controls">
            <label class="avatar-edit__upload">
              <input
                type="file"
                accept="image/*"
                class="icon-file-input"
                onchange={handleSkinFile}
                disabled={skinBusy}
              />
              {t('vault.skin.uploadLabel')}
            </label>
            {#if stagedSkin}
              <Field label={t('vault.skin.stageNameLabel')} required={true}>
                <input
                  type="text"
                  class="input text-input"
                  bind:value={stagedName}
                  placeholder={t('vault.skin.stageNamePlaceholder')}
                  maxlength="40"
                  autocomplete="off"
                  spellcheck="false"
                />
              </Field>
            {/if}
            <div class="skin-atelier__variants" role="radiogroup">
              <label class="skin-atelier__variant">
                <input
                  type="radio"
                  name="skin-variant"
                  value="classic"
                  checked={skinVariant === 'classic'}
                  onchange={() => (skinVariant = 'classic')}
                  disabled={skinBusy}
                />
                {t('vault.skin.variantClassic')}
              </label>
              <label class="skin-atelier__variant">
                <input
                  type="radio"
                  name="skin-variant"
                  value="slim"
                  checked={skinVariant === 'slim'}
                  onchange={() => (skinVariant = 'slim')}
                  disabled={skinBusy}
                />
                {t('vault.skin.variantSlim')}
              </label>
            </div>
            <div class="skin-atelier__actions">
              {#if stagedSkin}
                <Btn
                  variant="primary"
                  size="sm"
                  loading={skinBusy}
                  disabled={!stagedName.trim()}
                  onclick={() => void handleSaveToGallery()}
                >
                  {t('vault.skin.saveToGallery')}
                </Btn>
                <Btn variant="ghost" size="sm" disabled={skinBusy} onclick={() => clearStagedSkin()}>
                  {t('vault.cardLogoutCancel')}
                </Btn>
              {/if}
              {#if !isOfflineActive}
                {#if !confirmingSkinReset}
                  <Btn
                    variant="danger"
                    size="sm"
                    disabled={skinBusy || !skinInfo?.has_skin}
                    onclick={(e) => promptSkinReset(e)}
                  >
                    {t('vault.skin.reset')}
                  </Btn>
                {:else}
                  <div class="inline-confirm__actions">
                    <Btn
                      variant="danger"
                      size="sm"
                      loading={skinBusy}
                      onclick={(e) => void confirmSkinReset(e)}
                    >
                      {t('vault.skin.reset')}
                    </Btn>
                    <Btn
                      variant="ghost"
                      size="sm"
                      disabled={skinBusy}
                      onclick={(e) => cancelSkinReset(e)}
                    >
                      {t('vault.cardLogoutCancel')}
                    </Btn>
                  </div>
                {/if}
                <Btn
                  variant="ghost"
                  size="sm"
                  disabled={skinLoading || skinBusy}
                  onclick={() => void handleRefreshSkin()}
                >
                  {t('vault.skin.refresh')}
                </Btn>
              {:else}
                <p class="skin-atelier__hint">{t('vault.skin.offlineCantApply')}</p>
              {/if}
            </div>
            <div class="skin-atelier__editor">
              <a
                class="btn btn--ghost btn--sm skin-atelier__editor-link"
                href="https://minecraft.novaskin.me/"
                target="_blank"
                rel="noopener"
              >
                <span aria-hidden="true">🎨</span>
                {t('vault.skin.editorLink')}
              </a>
              <p class="skin-atelier__hint">{t('vault.skin.editorHint')}</p>
            </div>
          </div>
        </div>
        <!-- Gallery -->
        <div class="skin-gallery">
          <div class="skin-gallery__head">
            <div class="skin-gallery__title-group">
              <h3 class="skin-gallery__title">{t('vault.skin.galleryTitle')}</h3>
              <Badge variant="neutral" size="sm">{gallery.length}</Badge>
            </div>
            <div class="skin-gallery__head-actions">
              <Btn
                variant="ghost"
                size="sm"
                loading={importingVanilla}
                disabled={galleryLoading}
                onclick={() => void handleImportVanilla()}
              >
                {t('vault.skin.importVanilla')}
              </Btn>
              <Btn
                variant="ghost"
                size="sm"
                disabled={galleryLoading || skinBusy}
                onclick={() => void loadGallery()}
              >
                {t('vault.skin.refresh')}
              </Btn>
            </div>
          </div>
          {#if galleryLoading && gallery.length === 0}
            <div class="roster-loading">
              <span class="spinner"></span>
              <span>{t('vault.skin.viewerLoading')}</span>
            </div>
          {:else if gallery.length === 0}
            <p class="skin-atelier__note">{t('vault.skin.galleryEmpty')}</p>
          {:else}
            <div class="skin-gallery__grid">
              {#each gallery as entry (entry.id)}
                <div
                  class="skin-card"
                  class:skin-card--previewing={previewEntryId === entry.id}
                >
                  <!-- Frozen 3D render (still user-draggable). A div, not a button:
                       SkinViewer owns a spin-toggle button and buttons cannot nest. -->
                  <div
                    class="skin-card__art"
                    role="button"
                    tabindex="0"
                    onclick={() => previewSkin(entry.id)}
                    onkeydown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        previewSkin(entry.id);
                      }
                    }}
                    title={t('vault.skin.galleryPreview')}
                    aria-label={`${t('vault.skin.galleryPreview')}: ${entry.name}`}
                  >
                    <SkinViewer
                      skinUrl={`${librarySkinPngUrl(entry.id)}${galleryBust ? `?v=${galleryBust}` : ''}`}
                      variant={entry.variant}
                      height={120}
                      autoplay={false}
                      lazy
                    />
                  </div>
                  <div class="skin-card__body">
                    {#if renamingId === entry.id}
                      <form
                        class="skin-card__rename"
                        onsubmit={(e) => {
                          e.preventDefault();
                          void confirmRenameEntry(entry.id, e);
                        }}
                      >
                        <input
                          type="text"
                          class="input text-input"
                          bind:value={renameValue}
                          maxlength="40"
                          autocomplete="off"
                          spellcheck="false"
                        />
                        <div class="skin-card__row">
                          <Btn variant="primary" size="sm" type="submit" disabled={!renameValue.trim() || skinBusy}>
                            {t('vault.skin.galleryRenameSave')}
                          </Btn>
                          <Btn variant="ghost" size="sm" onclick={(e) => cancelRenameEntry(e)}>
                            {t('vault.cardLogoutCancel')}
                          </Btn>
                        </div>
                      </form>
                    {:else}
                      <p class="skin-card__name" title={entry.name}>{entry.name}</p>
                      <p class="skin-card__meta">
                        {entry.variant === 'slim' ? t('vault.skin.variantSlim') : t('vault.skin.variantClassic')}
                      </p>
                    {/if}
                    {#if renamingId !== entry.id}
                      <div class="skin-card__row">
                        {#if !isOfflineActive}
                          <Btn
                            variant="primary"
                            size="sm"
                            loading={applyingId === entry.id}
                            disabled={applyingId !== null || skinBusy}
                            onclick={() => void handleApplyEntry(entry.id)}
                          >
                            {applyingId === entry.id ? t('vault.skin.galleryApplying') : t('vault.skin.galleryUse')}
                          </Btn>
                        {/if}
                        <Btn
                          variant="secondary"
                          size="sm"
                          disabled={skinBusy}
                          onclick={() => previewSkin(entry.id)}
                        >
                          {t('vault.skin.galleryTry')}
                        </Btn>
                        <Btn
                          variant="ghost"
                          size="sm"
                          disabled={skinBusy}
                          onclick={(e) => promptRenameEntry(entry, e)}
                        >
                          {t('vault.skin.galleryRename')}
                        </Btn>
                        {#if confirmingDeleteId === entry.id}
                          <Btn
                            variant="danger"
                            size="sm"
                            loading={deletingId === entry.id}
                            onclick={() => void handleDeleteEntry(entry.id)}
                          >
                            {t('vault.skin.galleryDelete')}
                          </Btn>
                          <Btn variant="ghost" size="sm" onclick={(e) => cancelDeleteEntry(e)}>
                            {t('vault.cardLogoutCancel')}
                          </Btn>
                        {:else}
                          <Btn
                            variant="ghost"
                            size="sm"
                            disabled={skinBusy}
                            onclick={(e) => promptDeleteEntry(entry.id, e)}
                          >
                            {t('vault.skin.galleryDelete')}
                          </Btn>
                        {/if}
                      </div>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </GlassCard>
  </section>

  <!-- Main Split Workbench: Directory on Left, Identity Forge on Right -->
  <div class="vault-workbench">
    <!-- Left Column: Synchronized Roster Grid -->
    <section class="workbench-col workbench-col--roster" aria-label="Account Directory">
      <div class="section-head">
        <div class="section-head__title-group">
          <h2 class="section-head__title">{t('vault.rosterTitle')}</h2>
          <span class="section-head__sub">{t('vault.rosterSubtitle')}</span>
        </div>
        <Badge variant="neutral" size="sm">{accounts.length}</Badge>
      </div>

      {#if loading && accounts.length === 0}
        <div class="roster-loading">
          <span class="spinner"></span>
          <span>{t('vault.loading')}</span>
        </div>
      {:else if accounts.length === 0}
        <div class="roster-empty">
          <div class="roster-empty__icon">👤</div>
          <h3 class="roster-empty__title">{t('vault.rosterEmpty')}</h3>
          <p class="roster-empty__desc">{t('vault.rosterEmptyHint')}</p>
        </div>
      {:else}
        <div class="roster-grid">
          {#each sortedAccounts as acc (acc.username)}
            {@const isActive = acc.username === activeUsername}
            {@const isConfirmingLogout = confirmingLogoutUser === acc.username}

            <div
              class="account-card"
              class:account-card--active={isActive}
              tabindex="0"
              role="button"
              aria-pressed={isActive}
              onclick={() => handleSetActive(acc.username)}
              onkeydown={(e) => {
                if (e.target !== e.currentTarget) return;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  handleSetActive(acc.username);
                }
              }}
            >
              <div class="account-card__head">
                <div class="account-card__avatar-wrap">
                  <MonogramTile
                    name={acc.username}
                    hue={acc.avatar_color ?? undefined}
                    avatarUrl={acc.has_avatar ? avatarSrc(acc.username, true) : undefined}
                    size={46}
                    shape="rounded"
                  />
                  {#if isActive}
                    <span class="active-pulse-dot" title={t('vault.cardActive')}></span>
                  {/if}
                </div>

                <div class="account-card__title-wrap">
                  <div class="account-card__name-row">
                    <span class="account-card__name">{acc.username}</span>
                    {#if isActive}
                      <Badge variant="ok" size="sm">{t('vault.cardActive')}</Badge>
                    {/if}
                  </div>

                  <div class="account-card__tags">
                    {#if acc.token_kind === 'msa'}
                      <Badge variant="accent" size="sm">{t('vault.cardKindMsa')}</Badge>
                    {:else}
                      <Badge variant="neutral" size="sm">{t('vault.cardKindOffline')}</Badge>
                    {/if}
                    <code class="account-card__uuid" title={acc.uuid}>{acc.uuid.slice(0, 8)}…</code>
                  </div>
                </div>
              </div>

              <!-- Card Timestamps & Metadata -->
              <div class="account-card__footer">
                <div class="account-card__timestamps">
                  <span class="card-ts">
                    {t('vault.cardLastUsed', {
                      time: acc.last_used ? timeAgo(acc.last_used) : t('vault.cardNever'),
                    })}
                  </span>
                </div>

                <div class="account-card__actions">
                  {#if !isActive}
                    <Btn
                      variant="ghost"
                      size="sm"
                      onclick={(e) => {
                        e.stopPropagation();
                        handleSetActive(acc.username);
                      }}
                    >
                      {t('vault.cardSetActive')}
                    </Btn>
                  {/if}

                  {#if acc.token_kind === 'msa'}
                    <Btn
                      variant="danger"
                      size="sm"
                      onclick={(e) => promptLogout(acc.username, e)}
                    >
                      {t('vault.cardLogoutMsa')}
                    </Btn>
                  {/if}
                </div>
              </div>

              <!-- Avatar customization: color swatches + image -->
              <div
                class="account-card__avatar-edit"
                onclick={(e) => e.stopPropagation()}
                onkeydown={(e) => e.stopPropagation()}
                role="presentation"
              >
                <span class="avatar-edit__label">{t('vault.avatarTitle')}</span>
                <div class="avatar-edit__swatches">
                  {#each AVATAR_HUES as sw (sw)}
                    <button
                      type="button"
                      class="avatar-swatch"
                      class:avatar-swatch--active={acc.avatar_color === sw}
                      style="background: hsl({sw}, 70%, 50%);"
                      title={`${sw}°`}
                      aria-label={`${t('vault.colorTitle')}: ${sw}°`}
                      aria-pressed={acc.avatar_color === sw}
                      disabled={avatarBusy === acc.username}
                      onclick={() => handleAvatarColor(acc.username, acc.avatar_color === sw ? null : sw)}
                    ></button>
                  {/each}
                </div>
                <div class="avatar-edit__actions">
                  <label class="avatar-edit__upload">
                    <input
                      type="file"
                      accept="image/*"
                      class="icon-file-input"
                      onchange={(e) => handleAvatarFile(acc.username, e)}
                      disabled={avatarBusy === acc.username}
                    />
                    {t('vault.avatarUpload')}
                  </label>
                  {#if acc.has_avatar}
                    <button
                      type="button"
                      class="avatar-edit__remove"
                      disabled={avatarBusy === acc.username}
                      onclick={() => handleRemoveAvatar(acc.username)}
                    >
                      {t('vault.avatarRemove')}
                    </button>
                  {/if}
                </div>
              </div>

              <!-- Inline Confirmation Sheet for MSA Sign-Out -->
              {#if isConfirmingLogout}
                <div
                  class="inline-confirm-overlay"
                  transition:fade={{ duration: 120 }}
                  onclick={(e) => e.stopPropagation()}
                  role="presentation"
                >
                  <p class="inline-confirm__text">
                    {t('vault.cardLogoutConfirm', { name: acc.username })}
                  </p>
                  <div class="inline-confirm__actions">
                    <Btn
                      variant="danger"
                      size="sm"
                      loading={loggingOut}
                      onclick={(e) => confirmLogout(acc.username, e)}
                    >
                      {t('vault.cardLogoutBtn')}
                    </Btn>
                    <Btn
                      variant="ghost"
                      size="sm"
                      disabled={loggingOut}
                      onclick={(e) => cancelLogout(e)}
                    >
                      {t('vault.cardLogoutCancel')}
                    </Btn>
                  </div>
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <!-- Right Column: Identity Forge & Advanced Controls -->
    <section class="workbench-col workbench-col--forge" aria-label="Identity Forge">
      <!-- Identity Forge Container -->
      <GlassCard title={t('vault.addTitle')} subtitle={t('vault.addSubtitle')} elevation="md">
        <!-- Segmented Tab Navigation -->
        <div class="forge-tabs">
          <button
            type="button"
            class="forge-tab"
            class:forge-tab--active={forgeTab === 'offline'}
            onclick={() => (forgeTab = 'offline')}
          >
            <span class="tab-icon">⚡</span>
            <span>{t('vault.tabOffline')}</span>
          </button>
          <button
            type="button"
            class="forge-tab"
            class:forge-tab--active={forgeTab === 'msa'}
            onclick={() => (forgeTab = 'msa')}
          >
            <span class="tab-icon">🌐</span>
            <span>{t('vault.tabMicrosoft')}</span>
          </button>
        </div>

        <!-- Tab 1: Offline Account Forge -->
        {#if forgeTab === 'offline'}
          <div class="forge-pane" transition:fade={{ duration: 120 }}>
            <p class="forge-desc">{t('vault.offlineDesc')}</p>

            <form
              class="offline-form"
              onsubmit={(e) => {
                e.preventDefault();
                handleCreateOffline();
              }}
            >
              <Field
                label={t('vault.offlineLabel')}
                hint={t('vault.offlineValidationHint')}
                error={offlineError}
                required={true}
              >
                <div class="input-with-monogram">
                  <div class="input-monogram-preview">
                    <MonogramTile
                      name={newOfflineUsername.trim() || '?'}
                      size={32}
                      shape="rounded"
                    />
                  </div>
                  <input
                    type="text"
                    class="input text-input"
                    bind:value={newOfflineUsername}
                    placeholder={t('vault.offlinePlaceholder')}
                    maxlength="16"
                    autocomplete="off"
                    spellcheck="false"
                  />
                </div>
              </Field>

              <div class="forge-submit-row">
                <Btn
                  variant="primary"
                  size="md"
                  type="submit"
                  disabled={!nameValid || creatingOffline}
                  loading={creatingOffline}
                  block={true}
                >
                  {#snippet icon()}
                    <span class="btn-icon">⚡</span>
                  {/snippet}
                  {creatingOffline ? t('vault.offlineCreating') : t('vault.offlineCreateBtn')}
                </Btn>
              </div>
            </form>
          </div>

        <!-- Tab 2: Microsoft OAuth Device Flow -->
        {:else}
          <div class="forge-pane" transition:fade={{ duration: 120 }}>
            <p class="forge-desc">{t('vault.msDesc')}</p>

            {#if msFlow}
              <div class="ms-device-flow-card" transition:scalePop={{ start: 0.96, duration: 160 }}>
                <!-- Step 1: Verification URI -->
                <div class="flow-step">
                  <div class="flow-step__head">
                    <span class="flow-step__number">1</span>
                    <span class="flow-step__label">{t('vault.msStep1')}</span>
                  </div>
                  <div class="flow-step__body">
                    <div class="uri-pill">
                      <a
                        href={msFlow.verification_uri}
                        target="_blank"
                        rel="noopener"
                        class="uri-link"
                      >
                        {msFlow.verification_uri}
                        <span class="external-icon">↗</span>
                      </a>
                      <div class="uri-actions">
                        <Btn
                          variant="ghost"
                          size="sm"
                          onclick={() => uriCopier.copy(msFlow?.verification_uri || '')}
                        >
                          {uriCopier.copied ? t('vault.msLinkCopied') : t('vault.msCopyLink')}
                        </Btn>
                      </div>
                    </div>
                  </div>
                </div>

                <!-- Step 2: User Code -->
                <div class="flow-step">
                  <div class="flow-step__head">
                    <span class="flow-step__number">2</span>
                    <span class="flow-step__label">{t('vault.msStep2')}</span>
                  </div>
                  <div class="flow-step__body">
                    <div class="code-banner">
                      <span class="code-display font-pixel">{msFlow.user_code}</span>
                      <Btn
                        variant="primary"
                        size="sm"
                        onclick={() => codeCopier.copy(msFlow?.user_code || '')}
                      >
                        {#snippet icon()}
                          <span>📋</span>
                        {/snippet}
                        {codeCopier.copied ? t('vault.msCodeCopied') : t('vault.msCopyCode')}
                      </Btn>
                    </div>
                  </div>
                </div>

                <!-- Waiting state pulse -->
                <div class="flow-waiting">
                  <span class="pulse-ring"></span>
                  <span class="flow-waiting__text">{t('vault.msWaiting')}</span>
                </div>

                <div class="flow-cancel">
                  <Btn variant="ghost" size="sm" onclick={cancelMs}>
                    {t('vault.msCancelBtn')}
                  </Btn>
                </div>
              </div>
            {:else}
              <div class="ms-start-box">
                <Btn
                  variant="primary"
                  size="lg"
                  block={true}
                  loading={msPolling}
                  onclick={startMs}
                >
                  {#snippet icon()}
                    <span class="btn-icon">🌐</span>
                  {/snippet}
                  {msPolling ? t('vault.msStarting') : t('vault.msStartBtn')}
                </Btn>
              </div>
            {/if}

            {#if msPending}
              <div class="ms-alert ms-alert--pending" transition:fade={{ duration: 140 }}>
                <div class="ms-alert__head">
                  <span class="alert-icon">⏳</span>
                  <span class="alert-title">{t('vault.msaPending')}</span>
                </div>
                <p class="alert-desc">{t('vault.msaPendingHint')}</p>
              </div>
            {:else if msError}
              <div class="ms-alert ms-alert--error" transition:fade={{ duration: 140 }}>
                <span class="alert-icon">⚠️</span>
                <span class="alert-desc">{msError}</span>
              </div>
            {/if}
          </div>
        {/if}
      </GlassCard>

      <!-- Advanced Azure Client ID Configuration Accordion -->
      <div class="advanced-section">
        <button
          type="button"
          class="advanced-toggle"
          onclick={toggleAdvancedClientId}
          aria-expanded={showAdvancedClientId}
        >
          <div class="advanced-toggle__label">
            <span class="advanced-icon">⚙️</span>
            <span>{t('vault.clientIdToggle')}</span>
          </div>
          <span class="advanced-caret" class:advanced-caret--open={showAdvancedClientId}>▼</span>
        </button>

        {#if showAdvancedClientId}
          <div class="advanced-pane" transition:flyY={{ y: -8, duration: 150 }}>
            <Field
              label={t('vault.clientIdLabel')}
              hint={t('vault.clientIdHint')}
              error={clientIdError}
            >
              <input
                type="text"
                class="input text-input"
                bind:value={clientIdValue}
                placeholder={t('vault.clientIdPlaceholder')}
                disabled={loadingClientId || savingClientId}
              />
            </Field>

            <div class="advanced-actions">
              <Btn
                variant="secondary"
                size="sm"
                loading={savingClientId}
                disabled={loadingClientId}
                onclick={handleSaveClientId}
              >
                {savingClientId ? t('vault.clientIdSaving') : t('vault.clientIdSave')}
              </Btn>
              <Btn
                variant="ghost"
                size="sm"
                disabled={savingClientId || loadingClientId}
                onclick={() => {
                  clientIdValue = '';
                  void handleSaveClientId();
                }}
              >
                {t('vault.clientIdReset')}
              </Btn>
            </div>
          </div>
        {/if}
      </div>

      <!-- Lunar Token Shield & Migration Card -->
      <div class="lunar-shield-card">
        <div class="lunar-shield-card__head">
          <div class="lunar-shield-card__title-wrap">
            <span class="lunar-icon">🌙</span>
            <span class="lunar-title">{t('vault.lunarShieldTitle')}</span>
          </div>
          <Badge variant="purple" size="sm">{t('vault.lunarBadge')}</Badge>
        </div>

        <p class="lunar-shield-card__desc">
          {t('vault.lunarShieldDesc')}
        </p>

        <div class="lunar-shield-card__action">
          <Btn
            variant="ghost"
            size="sm"
            onclick={() => navigateTo('#/library/import')}
          >
            {#snippet icon()}
              <span>📦</span>
            {/snippet}
            {t('vault.lunarImportCta')}
          </Btn>
        </div>
      </div>
    </section>
  </div>
</div>

<style>
  /* ==========================================================================
     AccountVault Horizon Glass Styles
     ========================================================================== */

  .account-vault-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-6, 24px);
    padding: var(--space-6, 24px) var(--space-8, 32px) var(--space-12, 48px);
    max-width: 1400px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* --- Top Hero Header --- */
  .vault-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4, 16px);
    flex-wrap: wrap;
  }

  .vault-hero__meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .vault-hero__badge-row {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .vault-hero__count-pill {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    font-weight: 500;
  }

  .vault-hero__title {
    margin: 0;
    line-height: 1.1;
  }

  .vault-hero__subtitle {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    max-width: 680px;
  }

  .vault-hero__actions {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  /* --- Hero Pedestal Spotlight Section --- */
  .pedestal-section {
    position: relative;
    width: 100%;
  }

  .pedestal-card {
    position: relative;
    background: linear-gradient(135deg, rgba(16, 24, 40, 0.75), rgba(8, 14, 26, 0.85));
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-xl, 16px);
    padding: var(--space-6, 24px) var(--space-8, 32px);
    overflow: hidden;
    box-shadow: 0 16px 36px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.08);
  }

  .pedestal-glow {
    position: absolute;
    top: -50%;
    left: 20%;
    width: 600px;
    height: 300px;
    background: radial-gradient(ellipse at center, var(--ambient-1, rgba(16, 185, 129, 0.15)), transparent 70%);
    pointer-events: none;
    z-index: 0;
  }

  .pedestal-layout {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: var(--space-8, 32px);
    flex-wrap: wrap;
  }

  /* 3D Pedestal Stage */
  .pedestal-stage {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-width: 140px;
  }

  .pedestal-avatar-wrapper {
    position: relative;
    z-index: 2;
    filter: drop-shadow(0 12px 24px rgba(0, 0, 0, 0.6));
    transition: transform var(--dur-med, 200ms) var(--ease-spring-soft);
  }

  .pedestal-avatar-wrapper:hover {
    transform: translateY(-4px) scale(1.02);
  }

  .pedestal-ring {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 140px;
    height: 140px;
    border-radius: 50%;
    border: 1px dashed var(--accent, #10b981);
    opacity: 0.35;
    animation: ring-rotate 30s linear infinite;
    pointer-events: none;
  }

  @keyframes ring-rotate {
    from {
      transform: translate(-50%, -50%) rotate(0deg);
    }
    to {
      transform: translate(-50%, -50%) rotate(360deg);
    }
  }

  .pedestal-floor {
    position: absolute;
    bottom: -10px;
    width: 120px;
    height: 16px;
    background: radial-gradient(ellipse at center, rgba(16, 185, 129, 0.4), transparent 70%);
    border-radius: 50%;
    filter: blur(4px);
  }

  /* Pedestal Identity Details */
  .pedestal-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    flex: 1;
    min-width: 280px;
  }

  .pedestal-info__top {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .pedestal-username {
    font-size: var(--text-2xl, 1.75rem);
    font-weight: 700;
    color: var(--text, #f1f5f9);
    letter-spacing: -0.02em;
    line-height: 1.2;
  }

  .pedestal-signature {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    display: flex;
    align-items: center;
    gap: var(--space-1, 4px);
  }

  .shield-dot {
    color: var(--accent, #10b981);
  }

  .pedestal-meta-grid {
    display: flex;
    align-items: center;
    gap: var(--space-6, 24px);
    margin-top: var(--space-3, 12px);
    flex-wrap: wrap;
  }

  .meta-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .meta-item__label {
    font-size: var(--text-xs, 0.75rem);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim, #5c6b84);
    font-weight: 600;
  }

  .meta-item__val {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #f1f5f9);
    font-weight: 500;
  }

  .meta-item--uuid {
    min-width: 180px;
  }

  .uuid-copy-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .uuid-code {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    background: rgba(0, 0, 0, 0.3);
    padding: 2px 6px;
    border-radius: var(--radius-sm, 4px);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }

  .uuid-copy-btn {
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm, 4px);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color var(--dur-fast, 120ms), background var(--dur-fast, 120ms);
  }

  .uuid-copy-btn:hover {
    color: var(--text, #f1f5f9);
    background: rgba(255, 255, 255, 0.08);
  }

  .pedestal-prompt {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    margin: 0;
  }

  /* --- Main Split Workbench --- */
  .vault-workbench {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(360px, 1fr);
    gap: var(--space-6, 24px);
    align-items: start;
  }

  @media (max-width: 1024px) {
    .vault-workbench {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .workbench-col {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .section-head__title-group {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .section-head__title {
    margin: 0;
    font-size: var(--text-lg, 1.125rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .section-head__sub {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  /* --- Roster Grid Cards --- */
  .roster-loading,
  .roster-empty {
    padding: var(--space-8, 32px);
    text-align: center;
    background: rgba(16, 24, 40, 0.4);
    border: 1px dashed var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2, 8px);
    color: var(--text-muted, #8b9bb4);
  }

  .roster-empty__icon {
    font-size: 2rem;
    opacity: 0.5;
  }

  .roster-empty__title {
    margin: 0;
    color: var(--text, #f1f5f9);
    font-size: var(--text-base, 1rem);
  }

  .roster-empty__desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    max-width: 360px;
  }

  .roster-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .account-card {
    position: relative;
    background: rgba(16, 24, 40, 0.55);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    cursor: pointer;
    outline: none;
    transition: transform var(--dur-fast, 120ms) var(--ease-spring-soft),
      border-color var(--dur-fast, 120ms),
      background var(--dur-fast, 120ms),
      box-shadow var(--dur-fast, 120ms);
  }

  .account-card:hover {
    background: rgba(22, 34, 56, 0.7);
    border-color: rgba(255, 255, 255, 0.16);
    transform: translateY(-2px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  }

  .account-card:focus-visible {
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 0 2px rgba(16, 185, 129, 0.4);
  }

  .account-card--active {
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.08), rgba(16, 24, 40, 0.7));
    border-color: var(--accent, #10b981);
    box-shadow: 0 0 20px rgba(16, 185, 129, 0.12);
  }

  .account-card__head {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
  }

  .account-card__avatar-wrap {
    position: relative;
  }

  .active-pulse-dot {
    position: absolute;
    bottom: -2px;
    right: -2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    box-shadow: 0 0 8px var(--accent, #10b981);
    border: 2px solid #0b1320;
  }

  .account-card__title-wrap {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }

  .account-card__name-row {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .account-card__name {
    font-size: var(--text-base, 1rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .account-card__tags {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .account-card__uuid {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-dim, #5c6b84);
  }

  .account-card__footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    padding-top: var(--space-2, 8px);
    margin-top: 2px;
  }

  .card-ts {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  .account-card__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* Avatar customization (color swatches + image) */
  .account-card__avatar-edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    padding-top: var(--space-2, 8px);
  }

  .avatar-edit__label {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  .avatar-edit__swatches {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .avatar-swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }

  .avatar-swatch:hover {
    transform: scale(1.12);
  }

  .avatar-swatch--active {
    border-color: #fff;
    box-shadow: 0 0 8px rgba(255, 255, 255, 0.4);
  }

  .avatar-edit__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .avatar-edit__upload {
    display: inline-flex;
    align-items: center;
    padding: 0.3rem 0.7rem;
    border-radius: 0.5rem;
    border: 1px solid var(--border, rgba(40, 58, 96, 0.45));
    background: rgba(255, 255, 255, 0.05);
    font-size: 0.75rem;
    cursor: pointer;
  }

  .avatar-edit__upload:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .avatar-edit__remove {
    background: none;
    border: none;
    color: var(--danger, #f87171);
    font-size: 0.75rem;
    cursor: pointer;
    padding: 0.3rem 0.4rem;
  }

  .icon-file-input {
    display: none;
  }

  /* Inline Logout Confirmation Overlay */
  .inline-confirm-overlay {
    position: absolute;
    inset: 0;
    background: rgba(12, 18, 30, 0.95);
    backdrop-filter: blur(6px);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    z-index: 10;
  }

  .inline-confirm__text {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #f1f5f9);
    font-weight: 500;
  }

  .inline-confirm__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* --- Identity Forge Pane & Tabs --- */
  .forge-tabs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    background: rgba(0, 0, 0, 0.3);
    padding: 4px;
    border-radius: var(--radius-md, 8px);
    margin-bottom: var(--space-4, 16px);
  }

  .forge-tab {
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2, 8px);
    transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
  }

  .forge-tab:hover {
    color: var(--text, #f1f5f9);
  }

  .forge-tab--active {
    background: var(--surface-raised, rgba(255, 255, 255, 0.08));
    color: var(--text, #f1f5f9);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
  }

  .forge-pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .forge-desc {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    line-height: 1.4;
  }

  .offline-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .input-with-monogram {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .input-monogram-preview {
    flex-shrink: 0;
  }

  .text-input {
    width: 100%;
    box-sizing: border-box;
  }

  .forge-submit-row {
    margin-top: var(--space-2, 8px);
  }

  /* Microsoft Device Flow */
  .ms-device-flow-card {
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-md, 8px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4, 16px);
  }

  .flow-step {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
  }

  .flow-step__head {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .flow-step__number {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    color: #060a14;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .flow-step__label {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .uri-pill {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
    background: rgba(0, 0, 0, 0.4);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    border-radius: var(--radius-sm, 6px);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }

  .uri-link {
    color: var(--accent, #10b981);
    font-size: var(--text-sm, 0.875rem);
    text-decoration: none;
    display: flex;
    align-items: center;
    gap: 4px;
    word-break: break-all;
  }

  .uri-link:hover {
    text-decoration: underline;
  }

  .code-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 12px);
    background: linear-gradient(135deg, rgba(16, 185, 129, 0.15), rgba(6, 182, 212, 0.1));
    border: 1px solid var(--accent, #10b981);
    border-radius: var(--radius-md, 8px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
  }

  .code-display {
    font-size: var(--text-xl, 1.25rem);
    font-weight: 700;
    color: #fff;
    letter-spacing: 0.15em;
  }

  .flow-waiting {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-2, 8px) 0;
  }

  .pulse-ring {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent, #10b981);
    animation: pulse 1.5s infinite;
  }

  @keyframes pulse {
    0% {
      transform: scale(0.95);
      box-shadow: 0 0 0 0 rgba(16, 185, 129, 0.7);
    }
    70% {
      transform: scale(1);
      box-shadow: 0 0 0 8px rgba(16, 185, 129, 0);
    }
    100% {
      transform: scale(0.95);
      box-shadow: 0 0 0 0 rgba(16, 185, 129, 0);
    }
  }

  .flow-waiting__text {
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
  }

  .flow-cancel {
    display: flex;
    justify-content: flex-end;
  }

  .ms-alert {
    border-radius: var(--radius-md, 8px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    font-size: var(--text-sm, 0.875rem);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .ms-alert--pending {
    background: rgba(245, 158, 11, 0.12);
    border: 1px solid rgba(245, 158, 11, 0.35);
    color: #fbbf24;
  }

  .ms-alert--error {
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.35);
    color: #f87171;
  }

  .ms-alert__head {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
    font-weight: 600;
  }

  .alert-desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    line-height: 1.4;
  }

  /* --- Advanced Client ID Accordion --- */
  .advanced-section {
    background: rgba(16, 24, 40, 0.4);
    border: 1px solid var(--glass-border, rgba(255, 255, 255, 0.08));
    border-radius: var(--radius-lg, 12px);
    overflow: hidden;
  }

  .advanced-toggle {
    width: 100%;
    background: transparent;
    border: none;
    color: var(--text-muted, #8b9bb4);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    font-size: var(--text-sm, 0.875rem);
    font-weight: 500;
    display: flex;
    align-items: center;
    justify-content: space-between;
    cursor: pointer;
    transition: background var(--dur-fast, 120ms), color var(--dur-fast, 120ms);
  }

  .advanced-toggle:hover {
    background: rgba(255, 255, 255, 0.03);
    color: var(--text, #f1f5f9);
  }

  .advanced-toggle__label {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .advanced-caret {
    font-size: 0.65rem;
    transition: transform var(--dur-fast, 120ms);
  }

  .advanced-caret--open {
    transform: rotate(180deg);
  }

  .advanced-pane {
    padding: var(--space-4, 16px);
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
    background: rgba(0, 0, 0, 0.2);
  }

  .advanced-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  /* --- Lunar Token Shield Card --- */
  .lunar-shield-card {
    background: linear-gradient(135deg, rgba(168, 85, 247, 0.08), rgba(16, 24, 40, 0.5));
    border: 1px solid rgba(168, 85, 247, 0.25);
    border-radius: var(--radius-lg, 12px);
    padding: var(--space-4, 16px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .lunar-shield-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2, 8px);
  }

  .lunar-shield-card__title-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .lunar-icon {
    font-size: 1.1rem;
  }

  .lunar-title {
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    color: var(--text, #f1f5f9);
  }

  .lunar-shield-card__desc {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    line-height: 1.4;
  }

  .lunar-shield-card__action {
    display: flex;
    justify-content: flex-end;
  }

  /* Helper Spinner */
  .spinner {
    width: 18px;
    height: 18px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, #10b981);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .text-ok {
    color: var(--accent, #10b981);
  }

  /* --- Skin Atelier --- */
  .skin-atelier-section {
    margin-top: var(--space-4, 16px);
  }

  .skin-atelier__layout {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(220px, 280px);
    gap: var(--space-4, 16px);
    align-items: start;
  }

  @media (max-width: 720px) {
    .skin-atelier__layout {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .skin-atelier__stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2, 8px);
    min-height: 200px;
    justify-content: center;
  }

  .skin-atelier__note {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    color: var(--text-muted, #8b9bb4);
    text-align: center;
    line-height: 1.5;
  }

  .skin-atelier__controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .skin-atelier__variants {
    display: flex;
    gap: var(--space-3, 12px);
  }

  .skin-atelier__variant {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1, 4px);
    font-size: var(--text-sm, 0.875rem);
    color: var(--text, #f1f5f9);
    cursor: pointer;
  }

  .skin-atelier__variant input {
    accent-color: var(--accent, #10b981);
  }

  .skin-atelier__actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .skin-atelier__editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
    align-items: flex-start;
  }

  .skin-atelier__editor-link {
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1, 4px);
  }

  .skin-atelier__hint {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
    line-height: 1.4;
  }

  .skin-gallery {
    margin-top: var(--space-4, 16px);
    padding-top: var(--space-4, 16px);
    border-top: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 12px);
  }

  .skin-gallery__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-2, 8px);
  }

  .skin-gallery__title-group {
    display: flex;
    align-items: center;
    gap: var(--space-2, 8px);
  }

  .skin-gallery__title {
    margin: 0;
    font-size: var(--text-md, 1rem);
    font-weight: 600;
  }

  .skin-gallery__head-actions {
    display: flex;
    gap: var(--space-2, 8px);
    flex-wrap: wrap;
  }

  .skin-gallery__grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-3, 12px);
  }

  .skin-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2, 8px);
    padding: var(--space-3, 12px);
    border-radius: var(--radius-md, 8px);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.08));
    background: rgba(255, 255, 255, 0.02);
  }

  .skin-card--previewing {
    border-color: var(--accent, #10b981);
  }

  .skin-card__art {
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
    border-radius: var(--radius-sm, 6px);
    overflow: hidden;
    align-self: center;
  }

  .skin-card__art:focus-visible {
    outline: 2px solid var(--accent, #10b981);
    outline-offset: 2px;
  }

  /* Frozen 3D card art: keep cards compact — the viewer's 200px stage
     min-height would otherwise stretch every card. */
  .skin-card__art :global(.skin-viewer) {
    min-height: 0;
  }

  .skin-card__art :global(.skin-viewer__canvas) {
    max-width: 150px;
  }

  .skin-card__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
    min-width: 0;
  }

  .skin-card__name {
    margin: 0;
    font-size: var(--text-sm, 0.875rem);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .skin-card__meta {
    margin: 0;
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-muted, #8b9bb4);
  }

  .skin-card__row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1, 4px);
    margin-top: var(--space-1, 4px);
  }

  .skin-card__rename {
    display: flex;
    flex-direction: column;
    gap: var(--space-1, 4px);
  }
</style>
