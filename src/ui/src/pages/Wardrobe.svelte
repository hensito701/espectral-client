<!--
  ============================================================================
  Wardrobe.svelte — Horizon Glass Identity & Cosmetics Armario (#/armario)
  ============================================================================
  Per-account identity panel: Mojang UUID verification, Discord link/unlink
  (system-browser OAuth + poll), ⚛/⚛★/MVP flag display, affirmative "share my
  cosmetics" consent, MVP-gated cosmetic slot pickers with exclusivity
  enforcement, local-only emotes toggle, and the per-viewer hidden players
  list. All state flows through the local engine (GET/PUT /api/cosmetics,
  /api/identity/*) — the UI never talks to Discord or Mojang directly.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import {
    getAccounts,
    getIdentity,
    verifyIdentity,
    startIdentityDiscordLink,
    unlinkIdentityDiscord,
    refreshIdentity,
    getCosmetics,
    putCosmetics,
    avatarUrl,
  } from '../lib/api';
  import { isTauri } from '../lib/tauri';
  import type {
    Account,
    IdentityAccount,
    IdentityState,
    CosmeticsState,
    CosmeticsAccountState,
    CosmeticsPatch,
    CosmeticItem,
  } from '../lib/types';
  import GlassCard from '../components/GlassCard.svelte';
  import Btn from '../components/Btn.svelte';
  import Badge from '../components/Badge.svelte';
  import GradientText from '../components/GradientText.svelte';
  import { pushToast } from '../lib/toast.svelte';
  import { t, lang } from '../lib/i18n.svelte';

  const STORAGE_KEY_ACCOUNT = 'horizon:wardrobe-account';
  const MC_NAME_RE = /^[A-Za-z0-9_]{3,16}$/;
  const LINK_POLL_MS = 2_000;
  const LINK_POLL_MAX = 90; // ~3 min budget, matching the Discord login gate

  // --- Server state ---
  let accounts = $state<Account[]>([]);
  let identity = $state<IdentityState | null>(null);
  let cosmetics = $state<CosmeticsState | null>(null);
  let loading = $state(true);
  let loadError = $state('');

  // --- Selection ---
  let selectedUuid = $state(
    (typeof localStorage !== 'undefined' && localStorage.getItem(STORAGE_KEY_ACCOUNT)) || '',
  );

  // --- Editable cosmetics draft (per selected account) ---
  let draftShare = $state(false);
  let draftEmotes = $state(false);
  let draftLoadout = $state<Record<string, string | null>>({});
  let draftHidden = $state<string[]>([]);
  let saving = $state(false);

  // --- Action state ---
  let verifying = $state(false);
  let refreshing = $state(false);
  let linking = $state(false);
  let unlinking = $state(false);
  let hiddenDraft = $state('');
  let hiddenError = $state('');

  const account = $derived<IdentityAccount | null>(
    identity?.accounts.find((a) => a.uuid === selectedUuid)
      ?? identity?.accounts[0]
      ?? null,
  );

  const accountCosmetics = $derived<CosmeticsAccountState | null>(
    account && cosmetics ? (cosmetics.accounts[account.uuid] ?? null) : null,
  );

  const isMvp = $derived(!!account?.flags?.includes('mvp'));
  const isSupporter = $derived(!!account?.flags?.includes('supporter'));
  const normUuid = (u: string): string => u.replace(/-/g, '').toLowerCase();
  const selectedAccount = $derived<Account | null>(
    account ? (accounts.find((a) => normUuid(a.uuid) === account.uuid) ?? null) : null,
  );
  const isOfflineAccount = $derived(selectedAccount?.token_kind === 'offline');

  /* Hydrate the editable draft whenever the selected account or the server
     state object changes (fresh GET or PUT response). */
  $effect(() => {
    const st = accountCosmetics;
    if (!st) return;
    draftShare = st.share;
    draftEmotes = !!st.emotes;
    draftHidden = [...st.hidden_players];
    const loadout: Record<string, string | null> = {};
    for (const slot of cosmetics?.catalog.slots ?? []) {
      loadout[slot] = st.loadout?.[slot] ?? null;
    }
    draftLoadout = loadout;
  });

  // Persist the account selection.
  $effect(() => {
    if (typeof localStorage !== 'undefined' && account?.uuid) {
      selectedUuid = account.uuid;
      localStorage.setItem(STORAGE_KEY_ACCOUNT, account.uuid);
    }
  });

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    loadError = '';
    try {
      const [accountsRes, identityRes, cosmeticsRes] = await Promise.all([
        getAccounts(),
        getIdentity(),
        getCosmetics(),
      ]);
      accounts = accountsRes;
      identity = identityRes;
      cosmetics = cosmeticsRes;
    } catch (err) {
      loadError = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  async function reloadIdentity() {
    try {
      identity = await getIdentity();
    } catch {
      /* transient — keep the last good snapshot */
    }
  }

  /* ---------- verification ---------- */

  async function verify() {
    if (!account || verifying) return;
    verifying = true;
    try {
      await verifyIdentity(account.uuid);
      await reloadIdentity();
      pushToast({ kind: 'ok', text: t('wardrobe.identity.verified') });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('wardrobe.saveError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      verifying = false;
    }
  }

  async function refreshFlags() {
    if (!account || refreshing) return;
    refreshing = true;
    try {
      await refreshIdentity(account.uuid);
      await reloadIdentity();
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('wardrobe.saveError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      refreshing = false;
    }
  }

  /* ---------- Discord link ----------
     Opens the authorize URL in the system browser (Tauri shell open; popup
     fallback for browser dev) — same pattern as lib/discord.svelte.ts —
     then polls /api/identity until the account shows a linked Discord. */

  async function openExternal(url: string): Promise<boolean> {
    if (isTauri()) {
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        await (window as any).__TAURI_INTERNALS__.invoke('plugin:shell|open', { path: url });
        return true;
      } catch {
        /* fall through to the popup */
      }
    }
    if (typeof window !== 'undefined') {
      const width = 540;
      const height = 800;
      const left = window.screenX + (window.outerWidth - width) / 2;
      const top = window.screenY + (window.outerHeight - height) / 2;
      const popup = window.open(url, 'discord-oauth', `width=${width},height=${height},left=${left},top=${top},scrollbars=yes,status=1`);
      return !!popup;
    }
    return false;
  }

  async function linkDiscord() {
    if (!account || linking) return;
    linking = true;
    try {
      const { url } = await startIdentityDiscordLink(account.uuid);
      const opened = await openExternal(url);
      if (!opened) {
        linking = false;
        return;
      }
      // Poll until the engine records the link (the OAuth callback lands on
      // /api/discord/callback which completes the bind server-side).
      for (let attempt = 0; attempt < LINK_POLL_MAX && linking; attempt++) {
        await new Promise((r) => setTimeout(r, LINK_POLL_MS));
        if (!linking) return; // cancelled while sleeping
        await reloadIdentity();
        const linked = identity?.accounts.find((a) => a.uuid === account.uuid)?.discord;
        if (linked) {
          linking = false;
          pushToast({ kind: 'ok', text: t('wardrobe.discord.linked') });
          return;
        }
      }
      linking = false;
    } catch (err) {
      linking = false;
      pushToast({
        kind: 'err',
        text: t('wardrobe.saveError', { error: err instanceof Error ? err.message : String(err) }),
      });
    }
  }

  function cancelLink() {
    linking = false;
  }

  async function unlinkDiscord() {
    if (!account || unlinking) return;
    unlinking = true;
    try {
      await unlinkIdentityDiscord(account.uuid);
      await reloadIdentity();
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('wardrobe.saveError', { error: err instanceof Error ? err.message : String(err) }),
      });
    } finally {
      unlinking = false;
    }
  }

  /* ---------- cosmetics persistence ---------- */

  // Edits made while a PUT is in flight are merged here (last write wins
  // per field) and flushed once the in-flight request resolves — a change
  // is never silently dropped.
  let pendingPatch: CosmeticsPatch | null = null;

  async function save(patch: CosmeticsPatch) {
    if (!account) return;
    if (saving) {
      pendingPatch = { ...pendingPatch, ...patch };
      return;
    }
    const uuid = account.uuid;
    saving = true;
    try {
      let next: CosmeticsPatch | null = patch;
      while (next) {
        const resp = await putCosmetics(uuid, next);
        next = pendingPatch;
        pendingPatch = null;
        // The PUT may return the account slice or the full state; absorb either.
        const asState = resp as unknown as CosmeticsState;
        if (asState && typeof asState === 'object' && asState.accounts && asState.catalog) {
          cosmetics = asState;
        } else if (cosmetics && resp && typeof resp === 'object') {
          cosmetics = {
            ...cosmetics,
            accounts: { ...cosmetics.accounts, [uuid]: resp as CosmeticsAccountState },
          };
        }
      }
      pushToast({ kind: 'ok', text: t('wardrobe.saved') });
    } catch (err) {
      pushToast({
        kind: 'err',
        text: t('wardrobe.saveError', { error: err instanceof Error ? err.message : String(err) }),
      });
      // Revert the optimistic draft to the last known server state.
      try {
        cosmetics = await getCosmetics();
      } catch {
        /* keep the draft — the error toast already explains */
      }
    } finally {
      saving = false;
      // Flush anything queued while the last PUT/revert was in flight.
      const queued = pendingPatch;
      pendingPatch = null;
      if (queued) void save(queued);
    }
  }

  function toggleShare(on: boolean) {
    draftShare = on;
    void save({ share: on });
  }

  function toggleEmotes(on: boolean) {
    draftEmotes = on;
    void save({ emotes: on });
  }

  /* ---------- loadout & exclusivity ---------- */

  /** Equipped items that would conflict with equipping `itemId` into `slot`.
      Rule ["cuello", ids]: any item worn in `cuello` excludes the listed ids
      (worn in their own slot), and vice-versa. */
  function conflictingEquipped(slot: string, itemId: string): { slot: string; itemId: string }[] {
    const out: { slot: string; itemId: string }[] = [];
    for (const [exclusiveSlot, ids] of cosmetics?.catalog.exclusive ?? []) {
      if (slot === exclusiveSlot) {
        for (const [otherSlot, equipped] of Object.entries(draftLoadout)) {
          if (otherSlot !== slot && equipped && ids.includes(equipped)) {
            out.push({ slot: otherSlot, itemId: equipped });
          }
        }
      } else if (ids.includes(itemId)) {
        const equippedInExclusive = draftLoadout[exclusiveSlot];
        if (equippedInExclusive) {
          out.push({ slot: exclusiveSlot, itemId: equippedInExclusive });
        }
      }
    }
    return out;
  }

  /** Equipped catalog item that blocks equipping `itemId`, for chip hints. */
  function conflictedBy(itemId: string, slot: string): CosmeticItem | null {
    const first = conflictingEquipped(slot, itemId)[0];
    return first ? (cosmetics?.catalog.items.find((i) => i.id === first.itemId) ?? null) : null;
  }

  function itemName(item: CosmeticItem): string {
    return item.name?.[lang.value as 'es' | 'en'] ?? item.id;
  }

  function equip(slot: string, itemId: string | null) {
    if (!itemId) {
      draftLoadout = { ...draftLoadout, [slot]: null };
      void save({ loadout: draftLoadout });
      return;
    }
    const next = { ...draftLoadout, [slot]: itemId };
    // Enforce exclusivity in the UI: unequip conflicting items and say why.
    for (const c of conflictingEquipped(slot, itemId)) {
      next[c.slot] = null;
      const dropped = cosmetics?.catalog.items.find((i) => i.id === c.itemId);
      pushToast({
        kind: 'info',
        text: t('wardrobe.slots.unequipped', { name: dropped ? itemName(dropped) : c.itemId }),
      });
    }
    draftLoadout = next;
    void save({ loadout: next });
  }

  function itemsForSlot(slot: string): CosmeticItem[] {
    return cosmetics?.catalog.items.filter((i) => i.slot === slot) ?? [];
  }

  /* ---------- hidden players ---------- */

  function addHidden() {
    const name = hiddenDraft.trim();
    if (!MC_NAME_RE.test(name)) {
      hiddenError = t('wardrobe.hidden.invalid');
      return;
    }
    hiddenError = '';
    if (draftHidden.some((h) => h.toLowerCase() === name.toLowerCase())) {
      hiddenDraft = '';
      return;
    }
    const next = [...draftHidden, name];
    draftHidden = next;
    hiddenDraft = '';
    void save({ hidden_players: next });
  }

  function removeHidden(name: string) {
    const next = draftHidden.filter((h) => h !== name);
    draftHidden = next;
    void save({ hidden_players: next });
  }

  function grantExpDate(acc: IdentityAccount): string {
    if (!acc.grant_exp) return '';
    return new Date(acc.grant_exp * 1000).toLocaleString(lang.value === 'en' ? 'en-US' : 'es-ES');
  }

  function discordAvatarUrl(discord: { id: string; avatar: string | null }): string | null {
    return discord.avatar
      ? `https://cdn.discordapp.com/avatars/${discord.id}/${discord.avatar}.png?size=64`
      : null;
  }
</script>

<svelte:head>
  <title>{t('wardrobe.title')} — Espectral</title>
</svelte:head>

<div class="wardrobe-page">
  <!-- Hero -->
  <header class="wardrobe-hero">
    <div class="wardrobe-hero__badge-row">
      <Badge variant="gold" size="sm" dot={true}>{t('wardrobe.tag')}</Badge>
    </div>
    <h1 class="wardrobe-hero__title">
      <GradientText as="span">{t('wardrobe.title')}</GradientText>
    </h1>
    <p class="wardrobe-hero__subtitle">{t('wardrobe.subtitle')}</p>
  </header>

  {#if loading}
    <div class="wardrobe-loading" aria-busy="true">
      <span class="spinner" aria-hidden="true"></span>
    </div>
  {:else if loadError}
    <GlassCard className="wardrobe-error" elevation="md">
      <p class="wardrobe-error__text">{t('wardrobe.loadError', { error: loadError })}</p>
      <Btn variant="secondary" size="sm" onclick={() => void load()}>{t('wardrobe.retry')}</Btn>
    </GlassCard>
  {:else if !account}
    <GlassCard className="wardrobe-error" elevation="md">
      <p class="wardrobe-error__text">{t('wardrobe.account.none')}</p>
      <Btn variant="secondary" size="sm" onclick={() => (window.location.hash = '#/account')}>
        {t('wardrobe.account.openVault')}
      </Btn>
    </GlassCard>
  {:else}
    <!-- Account picker -->
    <div class="account-picker" role="tablist" aria-label={t('wardrobe.account.picker')}>
      {#each identity?.accounts ?? [] as acc (acc.uuid)}
        <button
          type="button"
          role="tab"
          class="account-chip"
          class:account-chip--active={acc.uuid === account.uuid}
          aria-selected={acc.uuid === account.uuid}
          onclick={() => (selectedUuid = acc.uuid)}
        >
          <img class="account-chip__avatar" src={avatarUrl(acc.username)} alt="" aria-hidden="true" />
          <span class="account-chip__name">{acc.username}</span>
          {#if acc.flags.includes('supporter')}
            <span class="account-chip__flag" aria-hidden="true">⚛★</span>
          {:else}
            <span class="account-chip__flag" aria-hidden="true">⚛</span>
          {/if}
        </button>
      {/each}
    </div>

    <!-- Identity card: verification + Discord + flags -->
    <GlassCard className="wardrobe-card" elevation="md">
      <div class="wardrobe-card__head">
        <div>
          <h2 class="section-title">{t('wardrobe.identity.title')}</h2>
          <p class="section-subtitle">{t('wardrobe.identity.subtitle')}</p>
        </div>
        <div class="flag-badges">
          {#if isSupporter}
            <Badge variant="gold" size="md">{t('wardrobe.flags.supporter')}</Badge>
          {:else}
            <Badge variant="accent" size="md">{t('wardrobe.flags.none')}</Badge>
          {/if}
          {#if isMvp}
            <Badge variant="purple" size="md">{t('wardrobe.flags.mvp')}</Badge>
          {/if}
        </div>
      </div>
      <p class="flag-legend">{t('wardrobe.flags.legend')}</p>

      <div class="identity-rows">
        <!-- Minecraft verification -->
        <div class="identity-row">
          <div class="identity-row__info">
            <span class="identity-row__label">{account.username}</span>
            <span class="identity-row__status" class:identity-row__status--ok={account.verified}>
              {account.verified ? t('wardrobe.identity.verified') : t('wardrobe.identity.notVerified')}
            </span>
            {#if account.grant_exp}
              <span class="identity-row__hint">{t('wardrobe.identity.grantExp', { time: grantExpDate(account) })}</span>
            {/if}
          </div>
          <div class="identity-row__actions">
            {#if !account.verified}
              <Btn
                variant="primary"
                size="sm"
                loading={verifying}
                disabled={isOfflineAccount}
                title={isOfflineAccount ? t('wardrobe.identity.verifyHint') : undefined}
                onclick={() => void verify()}
              >
                {verifying ? t('wardrobe.identity.verifying') : t('wardrobe.identity.verify')}
              </Btn>
            {:else}
              <Btn variant="ghost" size="sm" loading={refreshing} onclick={() => void refreshFlags()}>
                {refreshing ? t('wardrobe.identity.refreshing') : t('wardrobe.identity.refresh')}
              </Btn>
            {/if}
          </div>
        </div>
        {#if !account.verified}
          <p class="identity-note">{t('wardrobe.identity.verifyHint')}</p>
        {/if}

        <!-- Discord link -->
        <div class="identity-row">
          <div class="identity-row__info">
            {#if account.discord}
              {#if discordAvatarUrl(account.discord)}
                <img class="discord-avatar" src={discordAvatarUrl(account.discord)} alt="" aria-hidden="true" />
              {/if}
              <span class="identity-row__label">@{account.discord.username}</span>
              <span class="identity-row__status identity-row__status--ok">{t('wardrobe.discord.linked')}</span>
            {:else}
              <span class="identity-row__label">Discord</span>
              <span class="identity-row__status">{t('wardrobe.discord.notLinked')}</span>
            {/if}
          </div>
          <div class="identity-row__actions">
            {#if linking}
              <span class="identity-row__hint">{t('wardrobe.discord.linking')}</span>
              <Btn variant="ghost" size="sm" onclick={cancelLink}>{t('wardrobe.discord.cancel')}</Btn>
            {:else if account.discord}
              <Btn variant="ghost" size="sm" loading={unlinking} onclick={() => void unlinkDiscord()}>
                {t('wardrobe.discord.unlink')}
              </Btn>
            {:else}
              <Btn
                variant="secondary"
                size="sm"
                disabled={!account.verified}
                title={!account.verified ? t('wardrobe.share.needsVerify') : undefined}
                onclick={() => void linkDiscord()}
              >
                {t('wardrobe.discord.link')}
              </Btn>
            {/if}
          </div>
        </div>
        {#if !account.discord && account.verified}
          <p class="identity-note">{t('wardrobe.discord.linkHint')}</p>
        {/if}
      </div>
    </GlassCard>

    <!-- Consent -->
    <GlassCard className="wardrobe-card" elevation="sm">
      <div class="consent-row">
        <div class="consent-row__info">
          <h2 class="section-title section-title--sm">{t('wardrobe.share.title')}</h2>
          <p class="section-subtitle">{t('wardrobe.share.desc')}</p>
          {#if !account.verified}
            <p class="identity-note">{t('wardrobe.share.needsVerify')}</p>
          {/if}
        </div>
        <button
          type="button"
          class="switch"
          class:on={draftShare}
          role="switch"
          aria-checked={draftShare}
          aria-label={t('wardrobe.share.title')}
          disabled={!account.verified || saving}
          onclick={() => toggleShare(!draftShare)}
        >
          <span class="switch__thumb"></span>
        </button>
      </div>
    </GlassCard>

    <!-- Slot pickers -->
    <GlassCard className="wardrobe-card" elevation="md">
      <div class="wardrobe-card__head">
        <div>
          <h2 class="section-title">{t('wardrobe.slots.title')}</h2>
          <p class="section-subtitle">{t('wardrobe.slots.subtitle')}</p>
        </div>
        {#if !isMvp}
          <Badge variant="warn" size="sm">MVP</Badge>
        {/if}
      </div>

      {#if !isMvp}
        <p class="locked-note">{t('wardrobe.slots.locked')}</p>
      {/if}

      <div class="slots-grid" class:slots-grid--locked={!isMvp}>
        {#each cosmetics?.catalog.slots ?? [] as slot (slot)}
          <div class="slot-group">
            <span class="slot-group__label">{t(`wardrobe.slot.${slot}`)}</span>
            <div class="slot-group__items">
              <button
                type="button"
                class="item-chip"
                class:item-chip--equipped={!draftLoadout[slot]}
                disabled={!isMvp}
                onclick={() => equip(slot, null)}
              >
                {t('wardrobe.slots.none')}
              </button>
              {#each itemsForSlot(slot) as item (item.id)}
                {@const conflict = conflictedBy(item.id, slot)}
                <button
                  type="button"
                  class="item-chip"
                  class:item-chip--equipped={draftLoadout[slot] === item.id}
                  class:item-chip--conflict={!!conflict}
                  disabled={!isMvp}
                  title={conflict ? t('wardrobe.slots.unequipped', { name: itemName(conflict) }) : undefined}
                  onclick={() => equip(slot, item.id)}
                >
                  {itemName(item)}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    </GlassCard>

    <!-- Emotes -->
    <GlassCard className="wardrobe-card" elevation="sm">
      <div class="consent-row">
        <div class="consent-row__info">
          <h2 class="section-title section-title--sm">{t('wardrobe.emotes.title')}</h2>
          <p class="section-subtitle">{t('wardrobe.emotes.desc')}</p>
          {#if !isMvp}
            <p class="locked-note">{t('wardrobe.slots.locked')}</p>
          {:else if cosmetics?.catalog.emotes?.length}
            <div class="emote-chips">
              {#each cosmetics.catalog.emotes as emote (emote.id)}
                <span class="emote-chip">{emote.name?.[lang.value as 'es' | 'en'] ?? emote.id}</span>
              {/each}
            </div>
          {/if}
        </div>
        <button
          type="button"
          class="switch"
          class:on={draftEmotes}
          role="switch"
          aria-checked={draftEmotes}
          aria-label={t('wardrobe.emotes.title')}
          disabled={!isMvp || saving}
          onclick={() => toggleEmotes(!draftEmotes)}
        >
          <span class="switch__thumb"></span>
        </button>
      </div>
    </GlassCard>

    <!-- Hidden players -->
    <GlassCard className="wardrobe-card" elevation="sm">
      <div class="wardrobe-card__head">
        <div>
          <h2 class="section-title">{t('wardrobe.hidden.title')}</h2>
          <p class="section-subtitle">{t('wardrobe.hidden.desc')}</p>
        </div>
      </div>

      <form class="hidden-form" onsubmit={(e) => { e.preventDefault(); addHidden(); }}>
        <input
          class="hidden-input"
          type="text"
          bind:value={hiddenDraft}
          placeholder={t('wardrobe.hidden.placeholder')}
          maxlength="16"
        />
        <Btn variant="secondary" size="sm" type="submit">{t('wardrobe.hidden.add')}</Btn>
      </form>
      {#if hiddenError}
        <p class="hidden-error">{hiddenError}</p>
      {/if}

      {#if draftHidden.length === 0}
        <p class="hidden-empty">{t('wardrobe.hidden.empty')}</p>
      {:else}
        <ul class="hidden-list">
          {#each draftHidden as name (name)}
            <li class="hidden-list__item">
              <span class="hidden-list__name">{name}</span>
              <button
                type="button"
                class="hidden-list__remove"
                aria-label={name}
                onclick={() => removeHidden(name)}
              >
                ×
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </GlassCard>

    <!-- Legal footer -->
    <p class="wardrobe-disclaimer">{t('wardrobe.disclaimer')}</p>
  {/if}
</div>

<style>
  .wardrobe-page {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
    padding: 2rem 2.5rem 4rem;
    max-width: 1100px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
  }

  /* Hero */
  .wardrobe-hero {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding-bottom: 1.25rem;
    border-bottom: 1px solid var(--border);
  }

  .wardrobe-hero__title {
    margin: 0;
    font-size: var(--text-hero);
    font-weight: 700;
    line-height: var(--leading-tight);
  }

  .wardrobe-hero__subtitle {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-md);
    max-width: 60ch;
    line-height: var(--leading-body);
  }

  .wardrobe-loading {
    display: flex;
    justify-content: center;
    padding: 4rem 0;
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: wardrobe-spin 0.8s linear infinite;
  }

  @keyframes wardrobe-spin {
    to { transform: rotate(360deg); }
  }

  .wardrobe-error__text {
    color: var(--muted-strong);
    margin: 0 0 0.75rem;
  }

  /* Account picker */
  .account-picker {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .account-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.375rem 0.75rem 0.375rem 0.375rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--text-sm);
    cursor: pointer;
    transition: border-color 0.15s ease, background-color 0.15s ease;
  }

  .account-chip:hover {
    border-color: var(--hover-border);
  }

  .account-chip--active {
    border-color: var(--accent);
    background: rgba(var(--accent-rgb), 0.12);
  }

  .account-chip__avatar {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--surface-up-solid);
  }

  .account-chip__flag {
    color: var(--accent-gold);
    font-size: var(--text-sm);
  }

  /* Cards */
  :global(.wardrobe-card) {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .wardrobe-card__head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
  }

  .section-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--text);
  }

  .section-title--sm {
    font-size: var(--text-base);
  }

  .section-subtitle {
    margin: 0.25rem 0 0;
    font-size: var(--text-sm);
    color: var(--muted);
    line-height: var(--leading-body);
  }

  .flag-badges {
    display: flex;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .flag-legend {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--muted);
  }

  /* Identity rows */
  .identity-rows {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .identity-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.75rem 1rem;
    background: var(--card-bg-soft);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .identity-row__info {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    flex-wrap: wrap;
  }

  .identity-row__label {
    font-weight: 600;
    font-size: var(--text-md);
    color: var(--text);
  }

  .identity-row__status {
    font-size: var(--text-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--accent-alt);
  }

  .identity-row__status--ok {
    color: var(--accent-green);
  }

  .identity-row__hint {
    font-size: var(--text-xs);
    color: var(--muted);
  }

  .identity-row__actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .identity-note {
    margin: -0.25rem 0 0;
    font-size: var(--text-xs);
    color: var(--muted);
    line-height: var(--leading-body);
  }

  .discord-avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
  }

  /* Consent rows + switch (pattern mirrors ClientSuite) */
  .consent-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1.5rem;
  }

  .consent-row__info {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    min-width: 0;
  }

  .switch {
    position: relative;
    display: inline-flex;
    align-items: center;
    width: 42px;
    height: 24px;
    padding: 2px;
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid var(--border);
    border-radius: 9999px;
    cursor: pointer;
    transition: background-color 0.2s cubic-bezier(0.16, 1, 0.3, 1), border-color 0.2s ease;
    flex-shrink: 0;
  }

  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .switch.on {
    background: var(--accent);
    border-color: rgba(var(--accent-rgb), 0.8);
    box-shadow: 0 0 12px rgba(var(--accent-rgb), 0.35);
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

  /* Slot pickers */
  .locked-note {
    margin: 0;
    padding: 0.625rem 0.875rem;
    font-size: var(--text-sm);
    color: var(--accent-alt);
    background: rgba(var(--accent-alt-rgb), 0.08);
    border: 1px solid rgba(var(--accent-alt-rgb), 0.25);
    border-radius: var(--radius-md);
  }

  .slots-grid {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .slots-grid--locked {
    opacity: 0.55;
  }

  .slot-group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .slot-group__label {
    font-size: var(--text-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--muted);
  }

  .slot-group__items {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .item-chip {
    padding: 0.375rem 0.875rem;
    font-family: var(--font-body);
    font-size: var(--text-sm);
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
    cursor: pointer;
    transition: border-color 0.15s ease, background-color 0.15s ease, color 0.15s ease;
  }

  .item-chip:hover:not(:disabled) {
    border-color: var(--hover-border);
  }

  .item-chip--equipped {
    background: rgba(var(--accent-gold-rgb), 0.14);
    border-color: var(--accent-gold);
    color: var(--accent-gold);
    font-weight: 600;
  }

  .item-chip--conflict {
    border-style: dashed;
    color: var(--muted);
  }

  .item-chip:disabled {
    cursor: not-allowed;
  }

  /* Emote chips */
  .emote-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    margin-top: 0.375rem;
  }

  .emote-chip {
    padding: 0.25rem 0.625rem;
    font-size: var(--text-xs);
    color: var(--muted-strong);
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: 999px;
  }

  /* Hidden players */
  .hidden-form {
    display: flex;
    gap: 0.5rem;
  }

  .hidden-input {
    flex: 1;
    min-width: 0;
    padding: 0.5rem 0.75rem;
    font-family: var(--font-body);
    font-size: var(--text-sm);
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    outline: none;
  }

  .hidden-input:focus {
    border-color: var(--border-focus);
  }

  .hidden-error {
    margin: 0.25rem 0 0;
    font-size: var(--text-xs);
    color: var(--accent-red);
  }

  .hidden-empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .hidden-list {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .hidden-list__item {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.25rem 0.375rem 0.25rem 0.75rem;
    background: var(--surface-up);
    border: 1px solid var(--border);
    border-radius: 999px;
  }

  .hidden-list__name {
    font-size: var(--text-sm);
    color: var(--text);
  }

  .hidden-list__remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    font-size: 0.875rem;
    line-height: 1;
    color: var(--muted);
    background: transparent;
    border: none;
    border-radius: 50%;
    cursor: pointer;
  }

  .hidden-list__remove:hover {
    color: var(--accent-red);
    background: rgba(var(--accent-red-rgb), 0.12);
  }

  /* Legal */
  .wardrobe-disclaimer {
    margin: 0;
    padding: 0.875rem 1rem;
    font-size: var(--text-xs);
    text-align: center;
    letter-spacing: 0.02em;
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--card-bg-soft);
  }
</style>
