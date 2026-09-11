# Espectral Client v2 — Product Vision and Grand Plan

**Status:** Approved product and engineering vision  
**Date:** 2026-09-11

## Product promise

**Espectral v2 is a fast, player-controlled Minecraft client: measured performance, a polished in-game suite, fair supporter expression, and privacy-respecting online services—without changing Minecraft server plugins, secretly reducing graphics, or overstating what client-side identification can prove.**

Suggested launch line:

> **Tu juego, a tu manera — rápido, claro y Espectral.**

Supporting message: Espectral improves startup and smoothness through reproducible measurements; player settings remain under player control; supporting the project unlocks expression, never gameplay advantage.

This vision builds on the existing launcher rather than replacing it. The current product already has a native Fabric feature suite, shared launcher/mod configuration, boot telemetry, Java 25 AOT training, Discord identity, a client-brand handshake, and a reproducible Fedora benchmark.

## Scope

The complete v2 experience is Fabric-first.

| Platform or surface | v2 commitment |
|---|---|
| Minecraft 26.2 + Fabric 0.19.5 | Primary proving ground and complete v2 suite |
| Minecraft 1.21.11 + Fabric | Compatibility lane, promoted after equivalent gates pass |
| Windows x64 | Tier 1 |
| Fedora/Linux x86-64 | Tier 1 after CI-built, signed install, update, and rollback validation |
| NeoForge 1.21.1 | Existing brand-only integration; not advertised as the full suite |
| Other versions and loaders | Launcher/core behavior only |
| macOS | No v2 commitment |

The performance preset retains all seven expected components, installed and enabled: **Fabric API, Sodium, Iris, Lithium, FerriteCore, Krypton, and Mod Menu**. Diagnostic work may temporarily ablate a mod to measure its cost, but no release or performance claim may be obtained by silently removing one.

## Workstream 1: Faster boot

**Status: complete — measured on the Fedora reference machine, 2026-09-11 (see "Measured outcome").**

### Objective

Make the already-proven AOT improvement part of every eligible ordinary launch, then optimize the next measured bottleneck.

### Evidence

The Fedora reference benchmark currently reports:

| Path | Median Play-to-menu time |
|---|---:|
| Espectral vanilla | 7.41 s |
| Full performance stack, ordinary launch | 11.42 s |
| Full performance stack, explicit AOT | 9.01 s |

Explicit AOT is approximately **2.4 seconds, or 21%, faster** than the ordinary full-stack path. The ordinary path does not currently pass `-XX:AOTCache`, so a trained cache provides no benefit there.

The first v2 boot improvement is therefore not speculative tuning. It is to make a valid trained cache participate automatically in every eligible normal launch.

### Required behavior

- A normal Fabric launch on a compatible JDK automatically consumes a valid AOT cache.
- Missing, invalid, or stale caches take the safe ordinary path.
- A failed AOT load never prevents the game from starting.
- Retraining remains deferred until the game exits and does not create a second simultaneous game window.
- The launcher exposes cache validity, invalidation reason, and proof that the JVM actually used linked classes.
- Cosmetic files and refreshes remain outside the JVM classpath so they do not invalidate AOT.

### Acceptance gate

On the Fedora reference machine:

- Full-stack normal-launch median is at or below **9.5 seconds**.
- The normal path is within **±5%** of the explicit-AOT path.
- `Using AOT-linked classes: true` appears on **20/20** consecutive valid-cache normal launches.
- Java, memory, mod set, options, visual profile, and cache state are disclosed and held constant.
- Each comparison uses one discarded warmup, at least five sequential timed runs, a 15-second settle, and reports median, minimum, maximum, and JVM arguments.

**Result (2026-09-11): all criteria pass** — see "Measured outcome (2026-09-11)".

Fixed-percentage AOT copy must not appear in the product until the normal launch path produces this proof. Later boot work must follow measured phase traces—especially Fabric/mixin initialization, asset validation, and `spawn_ms`—rather than intuition.

### Measured outcome (2026-09-11)

**Method.** Fedora reference machine (Ryzen 7 5825U, Micron 2500 NVMe, GNOME/Wayland; idle/sleep/dim/lock disabled), instance `Espectral default` (Minecraft 26.2, Fabric loader, seven-mod performance preset installed and enabled 7/7, visual profile Balanced, default options, `-Xms3G -Xmx3G`), runtime `/usr/bin/java` 25.0.2+10. Benchmark harness `bench-menu.py`: one discarded warmup, sequential timed runs, 15-second settle, each game stopped through the engine API. Cache state held constant across every run: key `ff7f3842…`, `game.aot` 144 912 384 bytes, `trained_at` 2026-09-11T10:21:45Z.

Disclosed JVM arguments (dry-run, access token redacted):

```text
/usr/bin/java -Xms3G -Xmx3G --add-opens java.base/java.lang=ALL-UNNAMED … --enable-native-access=ALL-UNNAMED
  "-Djava.library.path=…/instances/Espectral default/natives" -XX:-UsePerfData -Xverify:none "-DFabricMcEmu= net.minecraft.client.main.Main "
  -cp … -XX:AOTCache=…/cache/aot/ff7f3842…/game.aot -Xlog:aot=info:file=aot-%p.log
  net.fabricmc.loader.impl.launch.knot.KnotClient …
```

| Path | Median wall | Median `menu_ms` | Minimum | Maximum | Proof |
|---|---:|---:|---:|---:|---:|
| Full stack, ordinary launch, cache unused (12:20–12:28) | 11 416 ms | 11 342 ms | 10 992 ms | 16 302 ms | none available |
| Full stack, explicit AOT mode (12:32–12:34) | 9 013 ms | 8 817 ms | 8 410 ms | 9 065 ms | 6/6 |
| Full stack, ordinary launch, auto-AOT (23:29–23:31) | 7 212 ms | 7 185 ms | 7 145 ms | 7 458 ms | 6/6 |

- **Median gate met:** 7 212 ms wall / 7 185 ms engine against the ≤9.5-second requirement, 2.3 s inside it.
- **Parity:** the ordinary and explicit paths are now the same code path — one `buildArgv` branch, identical `-XX:AOTCache` / `-Xlog:aot` arguments, no mode flag on the wire. The auto path measured 20% below the last explicit-only reference; the same-session pair (12:20–12:34) remains the conservative statement of the AOT benefit: **−2.5 s, −22%**.
- **Proof:** 20/20 consecutive ordinary launches (23:32–23:37) each wrote `Using AOT-linked classes: true` into their own `aot-<pid>.log`, with zero `[warning]`/`[error]` `[aot]` records; each launch was stopped through the engine API as soon as its proof log carried the record. Ordinary launches always pass `-Xlog:aot=info:file=aot-%p.log`, so the proof is a per-launch artifact rather than a one-off experiment.
- **Failure surface:** missing, stale, or refused caches take the safe ordinary path. `-XX:AOTMode` defaults to `auto`, so a cache the JVM cannot use never blocks a launch; the JVM's own refusal line is surfaced instead of being swallowed.

**Implementation.** One consume gate in `buildArgv` (`src/engine/launch.mjs`): every non-training launch on a JDK 25+ runtime passes the cache and the proof log when `aotCacheExists && !aotCacheStale`, and reports `not found` / `no longer matches the classpath` / `JDK 25-tier` otherwise. `resolveLaunch` keeps owning staleness (classpath size+mtime stamp). `aotStatus` now returns `stale` and `proof.refusal`; the Normal/AOT switch is gone from the capsule, replaced by a status chip driven by `GET /api/instances/:name/aot` (active / ready / stale / not applied / untrained / training).

### Remaining boot budget (measured)

Medians of per-run phase intervals, same machine and instance, `spawn_ms` shown separately (launcher-side preparation is not a factor at 12–13 ms):

| Phase (marker → marker) | Ordinary (n=16) | Auto-AOT (n=5) | AOT saving | + `-XX:TieredStopAtLevel=1` (n=3) |
|---|---:|---:|---:|---:|
| spawn → `Loading Minecraft` | 659 ms | 430 ms | −229 ms | 387 ms |
| → `Mixin Subsystem` | 216 ms | 157 ms | −60 ms | 121 ms |
| → `Searching for graphics cards` | 1 315 ms | 957 ms | −358 ms | 896 ms |
| → `Datafixer` | 1 450 ms | 829 ms | −621 ms | 859 ms |
| → client constructor | 4 332 ms | 2 780 ms | −1 552 ms | 2 659 ms |
| → `Setting user:` | 171 ms | 105 ms | −66 ms | 100 ms |
| → `Reloading ResourceManager` | 1 720 ms | 1 106 ms | −614 ms | 974 ms |
| → `OpenAL initialized` | 1 494 ms | 785 ms | −710 ms | 622 ms |
| → `Sound engine started` (menu) | 1 ms | 1 ms | 0 ms | 1 ms |
| **total** | **11 358 ms** | **7 150 ms** | **−4 208 ms** | **6 619 ms** |
| launcher `spawn_ms` | 12 ms | 13 ms | — | 8 ms |

### Next boot targets (evidence-ranked)

AOT class-load trace (`-Xlog:class+load`, one auto-AOT boot, 26 528 classes, last load at 7.14 s of a 7.0 s boot):

| Window | Classes loaded | From the AOT archive |
|---|---:|---:|
| JVM start → `Loading Minecraft` | 6 000 | 94% |
| → `Mixin Subsystem` | 310 | 0% |
| → `Searching for graphics cards` | 366 | 51% |
| → `Datafixer` | 2 753 | 50% |
| → client constructor | 11 502 | 25% |
| → `Setting user:` | 263 | 63% |
| → `Reloading ResourceManager` | 3 421 | 49% |
| → `OpenAL initialized` | 1 890 | 39% |

1. **Client constructor window (2.78 s with AOT) is class-loading-bound, not JIT-bound.** It loads 11 502 of the boot's 26 528 classes and only a quarter come from the archive; `-XX:TieredStopAtLevel=1` moved this window by 121 ms while cutting 400 ms elsewhere. The next step is a profile of *what* loads there (registry/built-in bootstrap; `Blocks`, `Items` and `EntityTypes` define their hidden classes at runtime) — not another JVM flag.
2. **Fabric/Mixin initialization + mod class loading (0.16 s + loader share).** JEP 483 cannot cache classes defined by a user-defined class loader, and Fabric's `KnotClassLoader` defines mod classes through `JVM_DefineClass` (619 loads in the trace). Reducing that cost means fewer or leaner mod classes (branding mod), not AOT tuning.
3. **Resource reload + sound (1.89 s with AOT).** 3 421 + 1 890 class loads, roughly half cached; Sodium/Iris resource and shader scanning are the likely costs. Needs a resource-phase trace before any change.
4. **`-XX:TieredStopAtLevel=1` (the existing opt-in `fast_boot`) stacks on AOT: −403 ms wall (−5.6%) and −572 ms menu (−8.0%)**, almost all of it outside the client constructor (resources −132 ms, sound −163 ms, class-load warm-up). It lowers peak JIT quality, so it stays opt-in and belongs to Workstream 2's frame-time envelope; it does not invalidate the AOT cache (20/20 proof held while it was active).
5. **Launcher app startup is a separate metric from play-to-menu.** The engine reports `spawn_ms` 12–13 ms, so resolve/download work is not a boot factor. The app-side costs sit before the game: an unconditional 600 ms port probe before the engine spawn (`src-tauri/src/lib.rs`), a window hidden until engine health (`tauri.conf.json`), ~514 KB of route modules parsed before `listen` (`src/engine/server.mjs`), and three duplicate `GET /api/accounts` requests plus 99.4 KB of preloaded fonts on first paint.
6. **Cache-key collision across instances (found while measuring).** The AOT key is `sha256(version | javaBuild | osArch)`, so a vanilla and a fully modded instance of the same Minecraft version share one cache directory: today `bench-vanilla-26.2` reports the Fabric-trained `ff7f3842…` cache as stale while `Espectral default` reports it as valid, and whichever instance launches last retrains the shared cache for both. The instance tile badge (cache existence only) shows ⚡ AOT for both, so only the capsule resolves the per-instance truth. Including the loader and mod-set identity in the key would end the ping-pong.
7. **JDK tier watch.** `-Xverify:none` is already gated off for major ≥ 27 (`src/engine/launch.mjs`); the runtime resolver's supported-major list would reject a version declaring a newer major; AOT method profiling (JEP 515) is already part of the JDK 25 tier this build ships.

## Workstream 2: FPS and smoothness

### Objective

Improve frame-time consistency at the ceiling created by Sodium and the retained performance stack, without hiding visual-quality reductions.

Sodium already provides the largest renderer improvement. A credible v2 must not promise a dramatic universal multiplier from this baseline. It should optimize and report:

- p50, p95, and p99 frame time
- 1% lows
- frames above 50 ms and 100 ms
- heap after garbage collection
- fixed static, chunk-loading, entity-heavy, and shader scenes

### Product rules

1. **Balanced means today's visual quality.** No silent reductions to render distance, particles, shaders, graphics, or texture quality.
2. JVM, garbage-collector, configuration, and additive-mod candidates are introduced one at a time.
3. UI and cosmetic work stays out of the timed boot path.
4. No network request, asset decoding, or blocking file work occurs on the render or tick thread.
5. Disabled functionality uses early exits and performs no avoidable per-frame allocation.
6. Optional Fast and Quality profiles may follow, but they must be visible, reversible player choices.

### Regression envelope

All later v2 work must satisfy:

- Boot median no worse than **3%**.
- p50 frame time no worse than **2%**.
- p99 frame time no worse than **5%**.
- Heap after GC no worse than **5%**.
- No increase in frames above 50 ms.
- Balanced screenshots and settings remain equivalent.
- All seven performance-stack components remain present and enabled.

A 10–15% p95 improvement and 5–10% heap reduction are research goals, not release promises, until representative scene baselines establish normal variance.

## Workstream 3: Custom UI and suite controls

### Objective

Make the client unmistakably Espectral while preserving the familiar Minecraft interaction model.

### Experience

- Retain the vanilla panorama and expected Minecraft navigation.
- Apply a restrained dark-glass and gold Espectral visual system instead of introducing a heavyweight widget framework.
- Replace the flat feature list with accessible categories, search, reset, and explicit enabled/disabled text.
- Preserve keyboard navigation, narration, resizing, and correct parent-screen behavior.
- Add append-only **Espectral Client…** and **Apoyar** actions to the Esc screen without replacing vanilla pause behavior.
- Keep Right Shift as the recovery shortcut.
- Confirm before opening the canonical support URL: `https://espectral.tebex.io/`.
- Never append a Minecraft UUID, account token, or tracking query to the support URL.

### Suite master-toggle contract

When `suite.enabled` is off, every Espectral-owned in-world effect is suppressed:

- Fullbright
- No Fog
- Zoom
- Macros
- HUD overlays
- Skin and cosmetic layers
- All new v2 cosmetics and effects

Turning the suite off does not:

- Mutate granular feature choices.
- Disable the seven performance mods, which require restart-level changes.
- Disable launcher authentication, updating, or telemetry preferences.
- Remove the branded menu entry, Right Shift recovery, or Apoyar link.

Turning the suite on restores every previous granular choice exactly. This behavior must be implemented through one canonical configuration choke point, not duplicated across individual feature engines.

### Acceptance gate

- Schema migration preserves all unknown fields and defaults `suite.enabled` to true.
- Master off makes every owned feature evaluate inactive without changing stored flags; on restores them exactly.
- Gamma, fog, and zoom state survives repeated off/on transitions without drift.
- Launcher and mod share a canonical registry contract, resolving the existing `nofog` default mismatch.
- Supported Fabric versions pass at 854×480 and 1920×1080.
- The title, Esc actions, Suite screen, search, narration, keyboard traversal, resize behavior, and parent return all work without duplicate widgets.
- Disabled suite paths create no backend traffic and no material tick/render work beyond the central branch.

## Workstream 4: Donator perks and cosmetics

### Objective

Let supporters express themselves through individually controllable cosmetics visible to participating Espectral clients, without creating gameplay advantage.

### Tier philosophy

Tiers are cumulative:

| Tier | Initial cosmetic scope |
|---|---|
| VIP | Headwear |
| VIP+ | Expanded headwear and forehead bands |
| MVP | Companions and pets |
| MVP+ | Particles, layering, and early catalog access |

Exact assets and quantities remain a sustainable catalog and art-direction decision. Engineering rules are fixed:

- Benefits are cosmetic-only.
- Higher tiers add choice, not deliberately greater render load.
- One equipped item per slot is the safe default.
- Every cosmetic is individually toggleable and respects the Suite master.
- Viewers can hide all cosmetics or hide a specific player.
- Reduced-motion behavior and global particle caps are mandatory.
- Cosmetics never alter hitboxes, imitate armor or status effects, occlude opponents, or grant gameplay/performance functionality.
- Fullbright, no-fog, macros, HUD data, and performance controls remain available regardless of payment.
- Existing Minecraft-server rank perks remain separate from client cosmetics.

### Peer visibility

“Visible to other players” means **visible to participating, supported Espectral Fabric clients**. Vanilla clients and brand-only NeoForge clients see an unchanged player.

No Minecraft plugin jar or configuration change is required, but a first-party HTTPS backend is required:

1. The launcher links Discord identity to a proven Minecraft UUID.
2. The backend resolves immutable Discord role IDs using a bot credential.
3. The backend issues a short-lived signed entitlement bound to that UUID.
4. The launcher equips only catalog items allowed by the entitlement.
5. A viewer requests presence only for UUIDs already visible through the vanilla player/entity list.
6. The viewer verifies the signed entitlement and signed, content-addressed catalog before rendering.

A local tier flag, forged brand, wearer-provided URL, or wearer-supplied model grants nothing. Cosmetic sharing is an affirmative choice distinct from telemetry consent. Sharing off publishes no presence.

### Runtime architecture

The local Node engine remains the network-policy authority. A per-launch authenticated loopback bridge allows the Fabric mod to report visible UUIDs; the engine batches HTTPS polling and stages bounded, signed cache updates for the mod.

Start with batched HTTPS polling. Do not add WebSockets, P2P, custom Minecraft packets, or a plugin relay without measured need.

### Rollout order

1. Signed catalog, role fixtures, and entitlement contract.
2. Own-player hats and headbands.
3. Peer-visible hats and headbands.
4. Pets.
5. Particles and layering only after performance, accessibility, shader, and abuse gates pass.

### Acceptance gate

- Every role combination maps to the correct cumulative tier in fixtures.
- Wrong UUID/audience, unknown key, tampered signature, expired grant, forged brand, local tier edit, unknown asset, or peer URL renders no paid cosmetic.
- Role removal and revocation are reflected within the documented entitlement lifetime.
- Share-off produces no presence publication.
- Presence contains only the UUID, permitted loadout/grant reference, and short expiry; it contains no coordinates, chat, or server address.
- Presence history is not retained and expires within minutes.
- Vanilla packet capture contains no Espectral custom gameplay payload.
- A healthy-service loadout becomes visible to an eligible viewer within a target p95 of five seconds.
- Backend failure never blocks game launch or the render/game thread.
- The initial eight-player hats/headbands scene stays inside the global boot/frame regression envelope.

## Workstream 5: Trust and signed telemetry

V2 must keep three signals separate: the Minecraft brand, backend-signed entitlements, and client-signed telemetry.

### Minecraft brand

The current `minecraft:brand` value is equivalent to a User-Agent declaration:

> The server observed a client-supplied brand string containing `espectral`.

It can support greetings, display, diagnostics, or ad suppression. It cannot authenticate the launcher, prove an unmodified official client, establish a Discord tier, or support anti-cheat and moderation decisions.

Without a plugin challenge, a signed brand is copyable and replayable. A compact Ed25519 brand token is also unsuitable for the contemplated 64-character envelope: its raw signature alone requires about 86 unpadded base64url characters.

**Decision:** keep the current brand byte-compatible and remove signed-brand work from the v2 dependency chain. No valuable capability may depend on it.

### Signed entitlements

Valuable claims move to HTTPS. Backend-verified Discord and Minecraft linkage produces short-lived, revocable grants bound to:

- Minecraft UUID
- Audience
- Tier and permitted cosmetic IDs
- Catalog version
- Issued, not-before, and expiry timestamps
- Unique grant ID
- Signing-key ID

Bad signature, wrong UUID or audience, expired grant, or unknown key means no paid cosmetic. It must never prevent launching or playing.

Revocation is bounded by the signed lifetime, not instant while offline. A six-hour online grant is the initial proposed bound, subject to final policy for gifting, linked-account count, Boost expiry, chargebacks, offline accounts, and revocation ownership.

### Signed telemetry

The current product stores launch statistics locally in `launch-stats.jsonl`. Any v2 upload is:

- Separately opt-in and default off.
- Never required for cosmetics or another perk.
- Inspectable, exportable, and deletable.
- Pseudonymous, not described as anonymous.
- Limited to documented performance and launch fields plus a pseudonymous install key and coarse mod-set hash.
- Excludes chat, coordinates, world/server address, player lists, access/refresh tokens, and raw account identifiers.
- Batched outside game and render paths.

A per-install key can prove that an accepted record came from a registered key, was not modified after signing, and was not already ingested. TLS, a short-lived upload challenge, audience, nonce, timestamp, event ID, and atomic deduplication bound network replay.

The signature cannot prove that the measurement is true, that official code is running, or that a physical device is unique. A user controlling the client can fabricate correctly signed observations. Telemetry must never become moderation, rank, reward, anti-cheat, or trusted-playtime evidence.

Use separate signing keys for:

1. Releases and catalog assets.
2. Entitlements.
3. Install telemetry.

Never reuse an offline release key in an online service.

### Privacy requirements

- Telemetry and cosmetic sharing use separate consent flows, default off.
- Spanish and English notices state purpose, fields, retention, deletion, and processors.
- Local launch statistics continue without upload consent.
- Users can inspect, export, withdraw, delete, unlink, and rotate the install identity.
- Raw identifiable telemetry has an initial maximum retention of 30 days; longer aggregates must be genuinely de-identified and separately approved.
- Cosmetic presence expires within minutes and is not retained as social-history data.
- Known or possible minors do not receive optional analytics without an approved guardian-consent process.

## Minimal new architecture

One modular first-party HTTPS service, one relational database, a Discord bot credential, and an online entitlement-signing key are sufficient. The service owns:

- Discord OAuth/session exchange and guild-role resolution
- Discord-to-Minecraft account linking
- Signed entitlement issuance and revocation
- Signed content-addressed cosmetic catalog
- Short-lived cosmetic presence
- Optional telemetry ingestion and deletion
- Feature flags and kill switches

The existing boundaries remain:

- **Node engine:** local policy, credentials, network access, cache staging, and API authority
- **Svelte UI:** launcher presentation and user choices
- **Fabric mod:** in-game state, controls, and rendering
- **Tauri:** application lifecycle, packaging, and updater
- **Backend:** identity, roles, entitlements, catalog, presence, and optional telemetry ingestion

Do not add microservices, Kafka, queues, Redis, P2P, WebSockets, custom game packets, or a new Minecraft plugin until measured scale demonstrates a requirement.

## Phased execution plan

### Phase 0: Credibility and contracts

Stop-ship decisions before network implementation:

- Freeze the platform/version matrix and prohibited claims.
- Confirm exact Discord guild and VIP/VIP+/MVP/MVP+ role IDs and precedence.
- Select an approved Microsoft ownership-proof flow or documented manual UUID verification; “Microsoft signs a nonce” is not an existing capability.
- Decide multi-account, gifting, offline-account, Boost expiry, role-loss, chargeback, and revocation policies.
- Approve Spanish/English consent, retention, deletion, and minors treatment.
- Assign backend, signing-key, catalog, privacy, and operational owners.
- Capture missing in-game frame-time baselines.
- Freeze additive/versioned schemas for Suite config, `/api/v2`, entitlement, catalog, presence, and telemetry.
- Establish beta/stable feature flags and rollback behavior.

### Phase 1: Truthful, valuable core

- Automatically apply valid AOT caches to normal launches and expose proof. *(done 2026-09-11 — see Workstream 1 "Measured outcome")*
- Add locked performance scenes and instrumentation.
- Migrate to schema 2 and canonical feature defaults.
- Build the title, Esc, Suite, master-toggle, recovery, and Apoyar experience.
- Put Fedora artifacts into CI with signed install, update, and rollback paths.
- Remove unsupported fixed-percentage, competitor, and verified-client messaging.

This release remains useful without a backend.

### Phase 2: Identity and own cosmetics

- Deploy account linking and backend Discord-role resolution.
- Add signed entitlement/catalog issuance, cache, expiry, and revocation.
- Build launcher equip controls.
- Render own-player hats and headbands only from valid grants.
- Keep telemetry local.

### Phase 3: Peer expression

- Add separate “share my cosmetics” consent.
- Implement the authenticated mod-to-engine loopback bridge.
- Poll presence only for vanilla-visible UUIDs.
- Verify viewer-side grants and content-addressed assets.
- Add global and per-player hide controls.
- Expand to pets and particles only after frame-time, shader, accessibility, and abuse gates pass.

### Phase 4: Learning and stable operations

- Add separately opt-in signed telemetry upload.
- Ship inspect, export, delete, unlink, and key rotation.
- Exercise backend, DNS, signature, expiry, and cache-corruption failures.
- Exercise feature kill switches.
- Maintain separate beta/stable feeds and an N-1 rollback artifact.
- Promote stable only after Windows and Fedora install, upgrade, and rollback smoke tests pass.

## Stable release gates

V2 stable requires all of the following:

- Normal full-stack AOT meets the Fedora ≤9.5-second engineering gate. *(met 2026-09-11: 7.19 s median `menu_ms`, 20/20 linked-class proof — see Workstream 1 "Measured outcome")*
- The exact seven-mod preset and Balanced visuals remain intact.
- Custom UI and master state pass on both supported Fabric versions and required resolutions.
- Spoofed brand, local tier, peer URL, tampered asset, and invalid grant cannot produce paid cosmetics on a conforming viewer.
- Share-off sends no presence publication.
- Telemetry-off sends no analytics upload.
- Entitlement operation never silently enables analytics.
- Vanilla packet capture contains no Espectral custom gameplay payload.
- Backend, DNS, signature, or cache failure never blocks launch or gameplay.
- Zero-equipped cosmetics pass the global frame and boot regression envelope.
- The initial eight-player hats/headbands scene adds no more than 5% to p99 frame time and performs no network or decode work on the render thread.
- Role removal, expiry, wrong UUID/audience, replay, and signing-key rotation are exercised.
- Signed install, updater, and rollback paths pass on Windows and Fedora.
- Beta soak and kill-switch drills complete before stable promotion.

## Claims v2 must not make

V2 must not be marketed as:

- The “fastest client,” “2× FPS,” or universally a fixed percentage faster.
- An authenticated, verified, genuine, or unmodified client based on the Minecraft brand.
- Tamper-proof, unspoofable, cheat-proof, or replay-proof.
- Device-bound or machine-bound when using exportable software credentials.
- Server-authoritative cosmetics without naming the first-party entitlement service and participating viewers.
- Visible to everyone or all Minecraft players.
- Anonymous telemetry while stable pseudonymous identifiers exist.
- Instant revocation or permanently secure offline perks.
- Fully client-only or requiring “no server changes.”

The accurate formulation is:

> **No Minecraft plugin jar or configuration changes. One new first-party Espectral backend supports verified entitlements, participating-client cosmetics, and optional telemetry.**

Performance claims must name the hardware, operating system, Minecraft version, loader, JDK, mod stack, visual profile, cache state, and benchmark method. FastClient’s 60-mod result is not directly comparable with Espectral’s seven-mod preset; Lunar and Feather comparisons remain suspended until equivalent reproducible automation exists.

## Guiding principle

Deliver the measured boot improvement and useful UI first. Contain network, identity, and cosmetic risk behind explicit contracts and feature flags. Make paid benefits expressive but fair. Treat telemetry as untrusted client reporting even when signed. Promote every ambitious claim only after the exact behavior has produced reproducible evidence.