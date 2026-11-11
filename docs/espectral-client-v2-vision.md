# Espectral Client v2 — Product Vision and Grand Plan

> **Historical design note.** Written while the launcher was the Tauri shell plus a
> Node engine (both deleted in **1.7.0**). `src/engine/…`, `src-tauri/…` and the
> HTTP route names below are history; the live code is `native/engine` +
> `native/app`. Numbers and measurements are from the 1.6.x builds.

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

The performance preset retains all eight expected components, installed and enabled: **Fabric API, Sodium, Iris, ImmediatelyFast, Lithium, FerriteCore, Krypton, and Mod Menu**. Diagnostic work may temporarily ablate a mod to measure its cost, but no release or performance claim may be obtained by silently removing one.

*Set revision (2026-11-11):* ImmediatelyFast joined the preset — it batches immediate-mode GUI/text draws and is the only candidate that is open source (LGPL-3.0-or-later), ships stable releases for 1.21.11, 26.2 and 26.3, and touches none of the classes the branding mod rewrites. EntityCulling (non-free licence, entity renderer rewrite), ModernFix (no Fabric 26.x builds in the canonical project), Dynamic FPS (fights agent launches' no-throttle rule), More Culling (Cloth Config dependency, release gaps for 1.21.6–1.21.10), BadOptimizations (cancels the lightmap extraction our fullbright hooks) and ScalableLux/C2ME/Noisium/Sodium Extra were rejected. Measurements below marked "seven-mod" predate this change.

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
- **Proof:** 20/20 consecutive ordinary launches (23:32–23:37) each wrote `Using AOT-linked classes: true` into their own `aot-<pid>.log`, with zero `[warning]`/`[error]` `[aot]` records; each launch was stopped through the engine API as soon as its proof log carried the record. Ordinary launches always pass `-Xlog:aot=info:file=aot-%p.log`, so the proof is a per-launch artifact rather than a one-off experiment. *(Caveat added 2026-09-12: this marker fires before a second resource reload caused by a suite mixin defect — see "Boot optimization research". That defect is fixed as of 2026-09-12; the marker is a single event again, the gate metric itself is unchanged by the fix, and the "~8.16 s honest number" this caveat first carried was a reload retry, not a delayed menu.)*
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

1. **Client constructor window (2.78 s with AOT) is class-loading-bound, not JIT-bound.** It loads 11 502 of the boot's 26 528 classes and only a quarter come from the archive; `-XX:TieredStopAtLevel=1` moved this window by 121 ms while cutting 400 ms elsewhere. The next step is a profile of *what* loads there (registry/built-in bootstrap; `Blocks`, `Items` and `EntityTypes` define their hidden classes at runtime) — not another JVM flag. *(Refined 2026-09-12 in "Boot optimization research": the boot as a whole spends ~40 CPU-seconds in JIT, almost all of it C2, and only the tier limit moves it; the constructor window itself stays expensive under every JVM lever tried.)*
2. **Fabric/Mixin initialization + mod class loading (0.16 s + loader share).** JEP 483 cannot cache classes defined by a user-defined class loader, and Fabric's `KnotClassLoader` defines mod classes through `JVM_DefineClass` (619 loads in the trace). Reducing that cost means fewer or leaner mod classes (branding mod), not AOT tuning.
3. **Resource reload + sound (1.89 s with AOT).** 3 421 + 1 890 class loads, roughly half cached; Sodium/Iris resource and shader scanning are the likely costs. Needs a resource-phase trace before any change.
4. **`-XX:TieredStopAtLevel=1` (the existing opt-in `fast_boot`) stacks on AOT: −403 ms wall (−5.6%) and −572 ms menu (−8.0%)**, almost all of it outside the client constructor (resources −132 ms, sound −163 ms, class-load warm-up). It lowers peak JIT quality, so it stays opt-in and belongs to Workstream 2's frame-time envelope; it does not invalidate the AOT cache (20/20 proof held while it was active).
5. **Launcher app startup is a separate metric from play-to-menu.** The engine reports `spawn_ms` 12–13 ms, so resolve/download work is not a boot factor. The app-side costs sit before the game: an unconditional 600 ms port probe before the engine spawn (`src-tauri/src/lib.rs`), a window hidden until engine health (`tauri.conf.json`), ~514 KB of route modules parsed before `listen` (`src/engine/server.mjs`), and three duplicate `GET /api/accounts` requests plus 99.4 KB of preloaded fonts on first paint.
6. **Cache-key collision across instances (found while measuring).** The AOT key is `sha256(version | javaBuild | osArch)`, so a vanilla and a fully modded instance of the same Minecraft version share one cache directory: today `bench-vanilla-26.2` reports the Fabric-trained `ff7f3842…` cache as stale while `Espectral default` reports it as valid, and whichever instance launches last retrains the shared cache for both. The instance tile badge (cache existence only) shows ⚡ AOT for both, so only the capsule resolves the per-instance truth. Including the loader and mod-set identity in the key would end the ping-pong.
7. **JDK tier watch.** `-Xverify:none` is already gated off for major ≥ 27 (`src/engine/launch.mjs`); the runtime resolver's supported-major list would reject a version declaring a newer major; AOT method profiling (JEP 515) is already part of the JDK 25 tier this build ships.

### Boot optimization research (2026-09-12)

Follow-up research on the remaining 7.2 s, with the seven-mod preset and the suite kept intact. All numbers measured on the Fedora reference machine with the same harness and instance; same-session comparisons only, because boot time drifts by up to ~5% between sessions (7 212 ms at 23:29 vs 7 618 ms at 00:04 for identical configurations).

**Where the time actually goes.** The JVM consumes ~52–57 CPU-seconds during a ~8 s boot (≈6.1 cores busy; CPU boosts to 3.2–3.9 GHz throughout, so the `powersave` governor is not a limiter). Of that, **`-XX:+CITime` reports 39.9 s of JIT compilation** — C1 4.4 s, C2 35.5 s — for **16 055 methods** and 2.97 MB of bytecode, against ~17 000 compilation events in `-Xlog:jit+compilation`. Garbage collection is negligible (one young collection, 4.6 ms) and I/O is served entirely from page cache (108 MB `rchar`, 0 `read_bytes`, 89 k read syscalls). **Boot is a compilation problem, not an I/O, GC, or CPU-frequency problem.** After the menu marker the process drops to 0.1–0.25 cores, but only after a ~2.8–3.5 s tail (atlas creation and the rest of the reload): the marker is a *consistent* comparison point, not the exact moment the title screen paints.

**Highest-value finding — two Mixin target mistakes, fixed 2026-09-12.** Every launch logged

```text
[Render thread/ERROR]: Mixin apply for mod espectral-menu failed espectral-menu.mixins.json:AvatarRendererMixin
  -> ...AvatarRenderer: InvalidMixinException @Shadow field model was not located in the target class
     net.minecraft.client.renderer.entity.player.AvatarRenderer. No refMap loaded.
```

`branding-mod/mc262/.../AvatarRendererMixin.java` asked Mixin for two things its target cannot provide: a `@Shadow` field declared by the superclass (`LivingEntityRenderer#model`) and an `@Inject` into `submit`, also declared by `LivingEntityRenderer`. Mixin resolves both against its *own target class only* (the jar is right to ship no refmap: 26.2 runs named classes, so no remap is needed). The failed transform aborted the resource reload in flight (`EntityRenderDispatcher.onResourceManagerReload` → `EntityRenderers.createAvatarRenderers`), so the client ran a **second full reload**: in every captured pre-fix run (6 of 6 across both sessions) the log shows 1 failure, 2 `Reloading ResourceManager` and 2 `Sound engine started` records, the second marker **+960 ms** after the first (7 197 ms → 8 157 ms); a control launch with the suite jar removed shows 1 reload, 1 marker, 0 failures, 7 097 ms. Two further consequences: 3D skin layers — the feature those injections exist for — never rendered on 26.2, and AOT **training** launches escalated the same failure into a hard crash (`InvalidMixinException` → `MixinApplyError` → "Initializing game"; `crash-2026-09-11_23.52.52-client.txt`, `…23.54.47-client.txt`), so the launcher's own retrain flow could kill the JVM mid-training and write a partial cache.

**The fix (jar sha1 `cc188c5a…`).** The field is reached through a new `LivingEntityRendererAccessor` (`@Accessor("model")` get/set, declared on the class that owns the field); the third-person injections moved into a new `LivingEntityRendererSubmitMixin` targeting `LivingEntityRenderer`; both mixins share per-renderer state (saved model, constructor `slim` flag) through `es.spectral.menu.Skin3dSwapHost` + `Skin3dSwap`. Those two helpers had to sit *outside* `es.spectral.menu.mixin`: Mixin owns that package and refuses to transform any class in it that is not a registered mixin — the first attempt put the interface there and reproduced the identical symptom (class-load error inside `EntityRenderers.createAvatarRenderers` → aborted reload → second reload), which is also the cleanest confirmation of the mechanism above. Built with `sh gradlew :mc262:jar` (this Fedora box's JDK 25 ships no `lib/ct.sym`, so the `--release 25` option is dropped through a local init script; identical bytecode level).

**What the fix bought — and what it did not.** Verified by exporting the transformed classes (`-Dmixin.debug.export=true`): `LivingEntityRenderer` now implements the accessor and carries the injected `submit` handlers; `AvatarRenderer` implements the host interface with the `<init>` and hand-path handlers. Long runs after the fix (4/4 of ≥30 s, all three snapshotted) show exactly one reload and one marker with zero mixin records. **It does not move the gate metric**: 1 warmup + 5 timed runs measure wall median **7 613 ms** (min 7 412 / max 7 617), engine `menu_ms` median **7 428** — statistically identical to the same-session pre-fix control (7 618 / 7 492) and inside the known session drift (7 212 ms at 23:29). What the duplicate reload really cost was CPU work, and it is gone: **54.0 CPU-seconds per launch after the fix (n=3, 53.7–54.5) vs 59.1 before (n=2, 57.5–60.7)** over the same 32-second window, while the CPU-idle settling time is unchanged within noise (10.16 s before, 10.92 s median after). Read the fix as correctness (a working feature, a crash-free retrain, no client error) plus ~9% less CPU work — **not** as a faster boot; the earlier "+960 ms of real boot time" reading of the second marker over-claimed, because the second marker is a reload retry, not a delayed menu.

**Measured levers.**

| Lever | Measured effect | Verdict |
|---|---|---|
| Fix `AvatarRendererMixin` (done 2026-09-12) | error, duplicate reload, dead skin3d and the record-mode crash are gone; ~5 CPU-s (≈9%) less work per launch; **gate marker unchanged** (7 613 vs 7 618 ms same-session) | **done** — correctness and removed duplicate work, not a boot win |
| `-XX:TieredStopAtLevel=1` (existing opt-in `fast_boot`) | same-session control 7 618 ms wall / 7 492 ms menu → 6 810 / 6 667 ms; −808 ms wall (−10.6%), −825 ms menu (−11.0%); earlier session −403 ms wall. Runs also become far more reproducible (2 ms spread vs ±200 ms) | keep opt-in; the only JVM lever with real gains |
| `-XX:+AOTCompileEagerly` (experimental) + two-step training-data cache | cache replays profiles — 9 414 `Replay training:` records with a two-step cache vs **0** with the shipped one-step cache — but boot got **slower**: 8 573 / 8 877 ms vs 7 370 ms on the same cache, with more compilations (20.5 k / 20.9 k vs 19.1 k) | reject for boot |
| Two-step (record → create) training for the cache itself | 7 370 ms with the two-step cache vs 7 185 ms shipped — no boot gain; the one-step `-XX:AOTCacheOutput` flow stays | reject for boot |
| AOT compiled code (JEP 544) | not in this JDK: the cache's code region holds 569 adapter blobs (~700 KB) and **0 C1/C2 method blobs**; JEP 544 is still Candidate upstream | unavailable — the structural fix is a future JDK tier |
| ModernFix (only 26.2 Fabric build is the third-party `ModernFix-mVUS` 5.27.19-build.1; upstream dropped Fabric) | 7 815 ms wall vs 7 618 ms same-session control (+197 ms), LGPL-3.0 | reject |
| GC tuning (`UseSerialGC`/`ParallelGC`/heap flags) | one young GC of 4.6 ms in the whole boot | reject — nothing to win |
| CPU governor / `platform_profile` | `powersave` + `amd-pstate-epp`, yet 3.2–3.9 GHz during boot | not a limiter |
| Asset/classpath prefetch by the launcher | `spawn_ms` 7–16 ms; `fincore` shows the AOT cache 98.8% and the hot jars 100% resident; `read_bytes` 0 during boot | ≈0 on a warm machine; only relevant to cold boots |
| Suite/launcher side | one blocking 756 B config read + 2 `stat` calls on the render thread; Mod Menu/Iris update checks are off-thread; no network on the boot path | no meaningful budget to reclaim |

**Second in-house defect (not boot-speed, but player-visible):** `GammaEngine` writes `options.gamma().set(15.0)` every tick while fullbright is enabled, but vanilla's gamma range is 0–1, so the value is rejected (`Illegal option value 15.0 for Brightness`, 106–837 ERROR lines per session depending on how long the game runs) and fullbright silently does nothing — `options.txt` still holds the player's `gamma:0.5` after sessions with the feature on, and the ctor comment in `GammaEngine` ("vanilla slider caps at 1.0") shows the 15.0 was intended to escape the slider rather than to be a valid value.

**Ranked next actions.** 1) The 1.21.11 variant still carries the same pattern (`branding-mod/mc12111/.../AvatarRendererMixin.java` shadows `model` and injects into `submit` the same way); it needs the same treatment, and it can only be verified on a 1.21.11 rig — this box ships 26.2 only. 2) The gate cannot see this class of defect: the harness stops the client at the first marker, so a post-marker failure or duplicate reload never lands in the saved log. Keep the marker assertions (0 `FAILED during APPLY`, one `Reloading ResourceManager`) **and** run one long launch per release, checking `Reloading ResourceManager == 1` and `Sound engine started == 1`. 3) Surface `fast_boot` as a visible, reversible Fast profile once Workstream 2 bounds its frame-time cost. 4) Fix the fullbright gamma value — `options.txt` still holds the player's `gamma:0.5` after sessions with the feature on, which is direct evidence the 15.0 override is rejected and the feature is a no-op. 5) Watch the JDK tier: JEP 544 is the only lever that can remove the remaining ~40 s of boot-time compilation, and AOT method-profile caches (JEP 515) only help *after* boot. 6) Do not spend effort on GC flags, CPU governors, launcher prefetch, or additive boot mods — all measured or reasoned to ~zero. 7) Mod-jar rebuilds do not invalidate the AOT cache (`isCacheStale` and the JVM both accept it after a branding jar change: the mods dir is not on the JVM classpath), so the stale-cache warnings remain reserved for real launcher-classpath drift.

### 1.8.2 low-end pass (2026-11-10)

**Method.** Windows 11, Ryzen 5 3600 (6c/12t), 32 GB, RX 580, SSD, Temurin 25.0.4.1. Instance: `26.3` Fabric + the performance preset + espectral-menu, render distance 16, vsync off, 854×480. Game arms were run interleaved and compared by median; the launcher ran from a copy with its own data directory on a second Windows desktop, so nothing was measured through the installed client. The low-end PC was emulated, not owned: `-Xmx2G` (the automatic heap up to 8.5 GB of RAM), the game pinned to 4 logical CPUs (2 cores), and `ESPECTRAL_TOTAL_RAM_MB` for the launcher's own decisions. A slow disk, real memory pressure and an integrated GPU were **not** emulated, so every figure below is a same-machine comparison, not a prediction for a given low-end PC. A second agent session ran a Paper server on the box for about half an hour of the pass; runs in that window are noisier and no conclusion rests on them alone.

**AOT training took six minutes because of the signed client jar.** Training on this PC took 351 s, 352 s and 363 s. The trainer JVM spends about 160 s of one core at under 100 MB before the game starts to boot, and the JDK's cache-assembly child spends about as long again while the trainer waits at 1.5 GB (peak for the pair: 2.1 GB). Cause: Mojang signs the client jar, so its `META-INF/MANIFEST.MF` is 5.07 MB with 32 875 per-entry sections and 40 953 continuation lines, and JDK 25 reads the manifest of every classpath jar in AOT record and create mode with a routine that does not scale. `java -XX:AOTCacheOutput=x -cp 26.3.jar -version` runs for more than 300 s; the other 61 classpath jars together take 3.1 s; the same jar with the manifest cut down to its main section takes 0.8 s. Fabric does not need the game jar on the JVM classpath (Knot loads the game classes itself), so Fabric launches on an AOT-capable JVM now pass it as `-Dfabric.gameJarPath` instead.

| | Training | Assembly step | Boot to menu with the cache (n=4) |
|---|---:|---:|---:|
| Client jar on the classpath (1.8.0 and earlier) | 351 s | ~165 s | 12.31 s (11.26–14.92) |
| Client jar passed as `fabric.gameJarPath` | 32 s | ~14 s | 11.88 s (11.30–13.04) |
| No cache | — | — | 13.93 s (13.72–14.70) |

The boot itself does not change; what changes is how soon an instance gets its cache, and what building it costs. A game launched while the trainer is still booting supersedes it, so on a PC where training outlasts the pause between two sessions the cache was never finished. Vanilla launches still carry the signed jar on the classpath (the JVM's own loader needs it there) and still train slowly; the trainer and its assembly child now run at below-normal priority, which keeps that cost off the foreground. An unsigned derived jar would fix vanilla too and would let the JVM cache Minecraft's own classes (it skips classes from signed jars), but it is a new artifact with its own failure modes and was left out of this pass.

**A cache trained under different JVM options was refused silently.** The JVM rejects a cache when the module options differ from training (`Mismatched values for property jdk.module.addexports`, `Unable to map shared spaces`), and the launcher compared only the classpath stamp: it kept passing the dead cache and never retrained. `meta.json` now records the AOT-relevant options (`vm_flags`); a cache without them, or with different ones, is stale. Every cache trained by 1.8.0 or earlier therefore retrains once after the update — necessary in any case, since 1.8.2 adds the options Mojang ships in the version JSON.

**RAM in a world at `-Xmx2G`.** Native memory tracking at 1.94 GB RSS: Java heap 964 MB committed for 557 MB live, metaspace 139 MB, code 79 MB, GC structures 74 MB, thread stacks 49 MB, and about 540 MB outside the JVM (graphics driver, LWJGL buffers). The heap's committed-but-free share is the part the launcher controls.

| Arm (4 logical CPUs) | Runs | RSS | Committed heap | Live heap | Game CPU | p50 frame | p99 frame | Frames > 50 ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Options before this pass (1.8.0 plus the 26.3 crash-fix options) | 6 | 1804 MB | 849 MB | 650 MB | 349 s | 1.12–1.17 ms | 2.77–3.15 ms | 0 |
| `+UseCompactObjectHeaders`, heap free ratio 20/40 | 8 | 1700 MB | 717 MB | 617 MB | 356 s | 1.12–1.17 ms | 3.05–3.14 ms | 0 |
| `+UseCompactObjectHeaders`, heap free ratio 10/30 | 2 | 1592 MB | 615 MB | 484 MB | 378 s | +6% (static) | +37% | 2 |
| Heap free ratio 20/40, no compact headers | 6 | 1734 MB | 756 MB | 669 MB | 356 s | 1.12–1.16 ms | 2.66–3.09 ms | 2 |

Shipped: `-XX:+UseCompactObjectHeaders` on 26.x vanilla and Fabric (it is in Mojang's own `default-user-jvm` for those versions, and Temurin 25 ships the matching `classes_coh.jsa`), and, for those same launches on a PC with 8.5 GB of RAM or less, `-XX:MinHeapFreeRatio=20 -XX:MaxHeapFreeRatio=40`. Against the regression envelope: p50 −0.1%/−0.1%/−0.2% across the three scenes, p99 −1.3%/+11.7%/−0.3%, no frame above 50 ms, boot +1.8%, CPU +2%. The middle p99 figure is over the envelope's 5% on paper. Per-run p99 in these scenes falls into two groups in both arms, about 2.2–2.5 ms and about 3.1–3.2 ms, and the median moves with how many runs land in each group; what separates the groups was not found. It is not garbage collection: a GC log (`-Xlog:gc`, two runs per arm) shows 21 young pauses in world without the options and 31–33 with them, far fewer than the ~260 frames that make up the worst 1%. The same log gives the direct answer on stutter: with the options the collector runs about half again as often but each young pause is shorter (median 5.0 ms against 8.0 ms, longest 8–10 ms against 12 ms), the longest pause of any kind is the same (24–30 ms, a remark), pauses above 16 ms are the same (3–4 per run), and total pause time rises from about 280 to about 320 ms over the session. The ratio also makes the footprint predictable: without it the committed heap ended a run anywhere between 822 and 1144 MB (RSS up to 2121 MB), with it between 715 and 722 MB.

Rejected: the same ratio without compact headers, which is what a 1.21.x, JDK 21 or NeoForge launch would have got (p99 +10.5% in the third scene and two frames above 50 ms in six runs, for 75 MB); the 10/30 ratio (another 100 MB, but 10% more CPU and frames above 50 ms on 4 threads); `-XX:+UseStringDeduplication` (no measurable gain: 1889 vs 1875 MB); the rest of `default-user-jvm` (`-Xms2G` and `-XX:+AlwaysPreTouch` commit the heap up front; ZGC cannot use the AOT cache on JDK 25). Compact headers alone lower the live heap by 2–8% and do not move RSS on their own; they pay off together with the ratio.

**The launcher.** 1280×800 window, warm disk cache.

| Renderer | Window visible | Private memory | Working set | CPU for a 9-page tour | With a game running |
|---|---:|---:|---:|---:|---:|
| OpenGL (femtovg) | 250–260 ms (1.2 s on a cold driver cache) | 142–150 MB | 136–144 MB | 1.48 s | 149 MB |
| Software | 74–100 ms | 20–26 MB | 48–62 MB | 0.41 s | 26 MB |

The software renderer is now the default on PCs with 8.5 GB of RAM or less. Launcher CPU while a game runs is 0.7 s over a whole 14 s boot and 0.03 s per 30 s afterwards on either renderer, so the log pump, the PiP drawer and the poll timers were left alone.

**Play to `java`.** 15–47 ms for an offline account; 631, 632, 668 and 827 ms for a signed-in one, because the launch awaited the identity grant refresh. With a stored grant that is valid for more than ten minutes the refresh now runs beside the boot and rewrites the cosmetics file when it lands (only where that file still names the same account). Timed on the installed 1.8.2 with the same signed-in account and instance: 91 ms and 20 ms, and the stored grant's expiry had moved a day ahead afterwards, so the background refresh did run.

**Through the built launcher.** The figures above come from running `java` by hand with the launcher's argv. The release build was then driven end to end (launcher window, MCP launch, in-game bench), on the same data directory:

| Check | Result |
|---|---|
| Cache trained by the previous build (no `vm_flags`) | not passed to the JVM; retrained after the game exits |
| Training, `26.3` Fabric | 30.3–31.2 s over five runs (was 351 s); `1.21.11` Fabric on JDK 25: 34.6 s |
| Trainer priority | trainer and its assembly child at below-normal; a normal game launch at normal |
| Boot with the new cache, `26.3` Fabric (n=7 over two builds) | 10.6–12.0 s, median 11.7 s, `Using AOT-linked classes: true` every time, no retrain afterwards |
| `1.21.11` Fabric (older Fabric Loader) | reaches the title with the game jar passed as a property; second boot uses the cache |
| Vanilla `26.3` | reaches the menu with compact headers; classpath unchanged |
| `ESPECTRAL_TOTAL_RAM_MB=8000` | software renderer chosen, `-Xmx2G`, compact headers and the 20/40 ratio in the argv, cache retrained once because the options changed |

Emulated 8 GB PC, 4 logical CPUs for launcher and game, in a world (two runs each, interleaved, plus one on the final build): the game's RSS went from 1852/1903 MB to 1665/1679/1631 MB and its committed heap from 924/975 MB to 710/712/712 MB; the launcher's private memory from 140 MB to 25 MB; p50 frame time 1.06–1.13 ms before and 1.05–1.12 ms after; no frame above 50 ms in either. Together about 320 MB less for the pair.

Launcher start on the final build: 65–67 ms to a visible window and 23 MB private under the emulated 8 GB (software renderer chosen by itself); 215–352 ms and 144 MB on this 32 GB PC, where it stays on OpenGL — the same as before the pass (219–241 ms, 145 MB). The smaller start-up changes (one account load instead of two, the first server-status fetch after the window, the country table 20 s later, two to four runtime threads) are not measurable on this machine; they remove work from the first second on a slower one, and that was not measured.

**Bench gotcha.** `inactivityFpsLimit:"afk"`, the 26.x default, caps the game at 30 fps after a minute without input. That is the "locked 33.3 ms" in the chunkload and entities scenes of the 2026-09-13 baseline: a bench instance needs `inactivityFpsLimit:"minimized"`.

**Left for a decision.** A low-end options profile (render distance, graphics mode, particles) is the largest FPS lever on such a PC and the one this pass did not touch: product rule 1 forbids silent reductions, so it has to be a visible, reversible choice (rule 6).

## Workstream 2: FPS and smoothness

### Objective

Improve frame-time consistency at the ceiling created by Sodium and the retained performance stack, without hiding visual-quality reductions.

Sodium already provides the largest renderer improvement. A credible v2 must not promise a dramatic universal multiplier from this baseline. It should optimize and report:

- p50, p95, and p99 frame time
- 1% lows
- frames above 50 ms and 100 ms
- heap after garbage collection
- fixed static, chunk-loading, entity-heavy, and shader scenes

### Instrumentation and first baseline (2026-09-13)

**Status: instrumentation shipped on 26.2; first hardware baseline recorded on the Fedora reference machine.**

**Bench mode.** The branding mod (mc262 lane only) gains a file-gated bench engine: `<instance>/espectral-bench.json` present → armed; absent → fully inert (no hooks, no per-frame work, no I/O). The mod auto-creates or opens a named fixed-seed singleplayer world (no launcher/engine change needed — `progArgs` are hardcoded, so QuickPlay args were never an option), auto-skips the accessibility onboarding screen that otherwise blocks fresh gameDirs forever, executes scene steps (`tp`, `look`, `summon` via the integrated server's command stack at permission level 4, `wait_s`), and records per-frame times plus heap-after-GC (`GarbageCollectorMXBean` listener) into `<instance>/bench-metrics.jsonl` as `scene_start`/`scene_end`/`done` lines. Missing or un-creatable worlds emit a `world missing` line + `done` instead of hanging (30 s still-on-title detector).

**Harness.** `bench-fps.py` (next to `bench-menu.py`, outside the repo) writes the scene config, launches via the engine API, tails the metrics file, gives the shader scene its own launch with `iris.properties` pre-written (`shaderPack`/`enableShaders` — verified against `IrisConfig.class`; `currentShader` does not exist), and appends `fps` records to `results.jsonl`. `check-envelope.py` implements the regression envelope below as pass/fail (exit 0/1) with `--record-baseline`; superseded records are excluded from baselines. `setup-bench-world.sh` restores the world snapshot (`world-snapshot.tar.gz`) and checks the shaderpack.

**World + scenes.** `espectral-bench` save: seed 20260913, default terrain, survival/normal, spawn (−816, 76, −32). Scenes: `static`, `chunkload`, `entities` (20 cows — zombies burn in daylight), `shader` (MakeUp Ultra Fast 9.5e, Modrinth `izsIPI7a`, 26.2-compatible).

**First baseline (radeonsi hardware GL, 30 s scenes, single pass).** The box's amdgpu was never bound at boot (simple-framebuffer held the PCI device; module present in initramfs but never probed) — every earlier run rendered on **llvmpipe**. Fixed via `modprobe amdgpu` + `/etc/modules-load.d/amdgpu.conf`; the llvmpipe record is kept in `results.jsonl` marked superseded. On hardware:

| Scene | Frames | p50 | p95 | p99 | >50 ms | >100 ms | Heap after GC |
|---|---:|---:|---:|---:|---:|---:|---:|
| static | 1 766 | 16.66 ms | 17.23 ms | 29.66 ms | 3 | 0 | 547 MB |
| chunkload | 1 165 | 33.33 ms | 33.34 ms | 33.34 ms | 0 | 0 | 547 MB |
| entities | 900 | 33.33 ms | 33.34 ms | 33.34 ms | 0 | 0 | 542 MB |
| shader | 1 780 | 16.68 ms | 17.08 ms | 25.17 ms | 2 | 0 | 549 MB |

Static and shader hold a clean 60 fps; chunkload and entities sit at a locked 33.3 ms — a frame-pacing characteristic (likely vsync at a halved interval under load), not a tail problem: p50/p95/p99 are nearly identical inside those scenes. Baseline stored in `bench-baseline.json`; every later Workstream 2 change now has a pass/fail gate.

**Engine fix found by the swarm.** Contract C gave non-active-account launches `nativesDir = profiles/<uuid>/natives`, but `installLibraries` only ever extracts to the instance natives dir — every such launch died on `liblwjgl.so`. Profile launches now share the instance natives dir (`launch.mjs`; test updated).

### 1.7.4 pass: 26.3 default stack and Forge 1.8.9 (2026-11-06)

**Method.** Same Fedora machine (Ryzen 7 5825U, Barcelo iGPU on radeonsi, Mesa 26.2.4), 1600×900 window, vsync off, `maxFps` 260. Instances built by the engine like a player's: `26.3` Fabric + the performance preset + espectral-menu (render distance 16, Fancy), and `1.8.9` Forge 11.15.1.2318 + espectral-menu on the bundled Temurin 8. The bench file now drives 1.8.9 too (`branding-mod-forge189/.../bench/BenchEngine.java`, same `espectral-bench.json` → `bench-metrics.jsonl` contract, steps `{"cmd": "<server command>"}` / `{"wait_s": N}`), and the 26.x bench gained the same `cmd` verb. Scenes after a 30 s warm-up: `vista` and `vista2` (two headings from y 140 over a seed-20260913 world), `entities` (60 cows in view), `hidden` (150 no-AI cows under a 20×5×20 chest slab — occlusion culling). Every comparison interleaves its arms run by run; boot is `menu_ms` (launch → `Sound engine started`), 1 discarded warm-up per round. This laptop throttles after an hour of back-to-back launches (26.3 per-scene averages drift ±15% between rounds), so only interleaved same-session pairs are compared.

**Forge 1.8.9 — shipped.**

| Change | Result |
|---|---|
| `useVbo` on (vanilla default off) | avg fps vista 166 → 206 (+24%), vista2 176 → 198 (+13%), entities 286 → 308 (+8%), hidden 165 → 206 (+25%); 1% lows +6–18%. Set once per game dir by `GameDefaults` (`applied_defaults` in `espectral-client.json`), so turning it off sticks. |
| JDK 8 boot flags (`-XX:-UsePerfData -Xverify:none -XX:MetaspaceSize=128M`) | menu 6 257 → 5 730 ms (−527 ms, −8.4%, n = 9 each). The 21 MB default metaspace forced two `Metadata GC Threshold` full collections per boot. |
| `-XX:+DisableExplicitGC` | no boot change (the five `System.gc()` calls are on world load/quit) — rejected |
| G1 instead of the JDK 8 Parallel default | entities −17% avg fps — rejected |
| EntityCulling 1.5.0 (tr7zw, only 1.8.9 build) | within noise in every scene, `hidden` included — rejected |

**26.3 default stack — nothing to ship beyond a HUD early-out.** A JFR profile of the render thread puts espectral-menu at ≤1% of samples (the HUD overlay path resolved its theme every frame with every overlay off; now skipped). The rest is Sodium (`ClonedChunkSectionCache.cleanup`, region updates) and vanilla (`doAnimateTick` block lookups); GC is 12 young pauses of 5–10 ms in 110 s.

| Candidate | Result |
|---|---|
| ImmediatelyFast 1.17.1 + EntityCulling 1.11.3 + BadOptimizations 2.4.1 | two interleaved 3×3 sessions: +12% vista/hidden in the first, −1…−10% in the second; boot 11 413 vs 11 442 ms — no reliable gain. Sodium already culls entities (`use_entity_culling`). Rejected. |
| `-XX:+UseCompactObjectHeaders` | −2…−14% avg fps per scene (and it refuses the AOT cache) — rejected |
| AOT cache vs none, FPS | mixed within noise (−25…+17% per scene) — no steady-state cost to act on |
| Sodium 0.9.3-beta.1 | Iris 1.11.7 requires exactly 0.9.2 — not testable as a pin |

**Launcher bug found by the harness.** A per-instance `jdk_path_override` was cached as the launcher-wide `config.jvm` (and from there into `runtimes/jdk-<tier>/info.json`): after one instance pointed at a JDK 8, every Java 8 launch kept using it even with the override removed, and the AOT key of every 26.x instance (derived from `config.jvm`'s build) changed, so their caches were "not found" — boot 9.0 → 11–12 s in this session. `jvm::get_jvm_info` no longer caches overrides.

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
- All eight performance-stack components remain present and enabled.

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
- Confirm before opening the canonical support URL: `https://espectral.uhce.top/donaciones`.
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
- Disable the eight performance mods, which require restart-level changes.
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

### Measured outcome (2026-09-12) — 26.2 lane

**Status: shipped on 26.2 and verified on the Fedora reference machine; the 1.21.11 lane is not verified (no rig for it on this box).**

**One registry, two consumers.** `src/engine/suite-registry.json` is now the only list of features: ids, categories (visual / hud / chat / controls), kinds, `default_enabled`, per-feature state defaults, keybinds and the EN/ES names/descriptions. `scripts/sync-suite-registry.mjs` copies it into the mod jar as `assets/espectral-menu/suite-registry.json` and generates the jar's `lang/en_us.json` / `lang/es_es.json` (60 keys each); `tests/suite-registry.test.mjs` fails if the generated files, the launcher i18n keys or the registry ever drift apart (freshness is byte-compared). The engine derives its `REGISTRY`/`FEATURE_DEFAULTS` from the same file — a grep for feature ids in `src/engine/client.mjs` returns nothing. **The `nofog` mismatch is resolved to `false` on both sides** (the launcher's historical default won; an existing explicit `nofog: true` in a player file is left untouched), and `fullbright.gamma` defaults to the legal `1.0`.

**Schema 2 and the master switch.** `config/espectral-client.json` gains `suite.enabled` (default true) and `schema: 2`. Migration is additive and non-destructive on both sides: a v1 fixture carrying an unknown root key, an unknown feature id, an extra per-feature key and a bare-boolean entry survives load→save with all of it intact, plus `schema: 2` and `suite.enabled: true` (`tests/client-config.test.mjs`, and a live launch that migrated the instance's own v1 file). In the mod, one choke point — `ClientConfig.isFeatureEnabled(id)` — is `suiteEnabled && stored flag`, so every engine's existing check becomes master-aware without a second code path; `isFeatureEnabledRaw(id)` exists only so the UI can show the stored flag.

**In-game UI (26.2).** The flat toggle list is gone. New: a dark-glass/gold Suite screen (header with a live count, master chip, search box, category row, paginated feature rows with explicit `Enabled`/`Disabled` text and per-row tooltips, pager, Reset/Support/Done footer), a confirm dialog for both the reset and the support link, and a title screen that **keeps vanilla navigation** and appends one right-aligned action row (`★ Espectral Client…`, `♥ Apoyar`) plus a glass strip with the wordmark and version. Right Shift keeps opening the Suite from play, and the shortcut no longer lives inside `MacroEngine`.

**Evidence (Fedora box, `Espectral default`, jar sha1 `c0001105…`).** Screenshots at 854×480 and in a 1920-wide window: title (vanilla logo/splash/Realms/icon row/version/copyright all intact, no overlap), Suite (`13/13`, page 1/3, five rows, footer), search filtered to `1/13` for "zoom", the same screen in Spanish (`Suite: Activada`, `Buscar función…`, `Reiniciar`/`Apoyar`/`Listo` — the generated lang files drive the game language), the support dialog showing exactly `https://espectral.tebex.io/` with no query, UUID or token, and a resize to 640×400 that re-lays out to 12 rows / 2 pages with **no duplicated widgets**. Transformed-class proof (`-Dmixin.debug.export=true`): `TitleScreen` carries both `handler$…$espectralMenu$appendActions` and `handler$…$espectral$paintSuiteBackdrop` alongside Iris's and Mod Menu's own injections. Master-switch proof in-game: with `suite.enabled: false` the log shows **no** fullbright activity at all, with `true` it shows `fullbright: gamma 0.5 → 1.0`, and the stored flags are byte-identical before and after — and the fullbright gamma bug is fixed (no `Illegal option value 15.0 for Brightness` line is possible any more, 0 across every run).

**Gate mapping.** Registry contract ✅ (one file, parity tests, `nofog` resolved). Migration ✅ (both consumers, unknown data preserved). Master off/on ✅ (in-game logs + launcher file diffs; stored flags untouched). Disabled paths ✅ (chat-head intake now gated at the source, engines early-return through the choke point, no network in the mod). Title/Suite/search/resize/no-duplicates ✅ (screenshots). 854×480 ✅ and a 1920-wide window ✅ on 26.2; the 1.21.11 lane ✗ not verified here. Parent return, keyboard traversal and narration audio are vanilla `Screen`/`Button` behaviour and were code-audited (every `%s` language key was checked to receive its arguments — two were silently missing them and are fixed), but this box cannot inject keyboard input (no Wayland virtual-keyboard tooling, no passwordless `sudo`) so they were not exercised interactively. The Esc-screen additions are registered and compile, but `PauseScreen.init()` needs an in-world player and this box has no world, so they are **structurally verified only** (mixin registered, anchor `javap`-checked) and need one in-world launch on a rig with a world. `branding-mod/mc12111/.../Compat.java` gained the same five drawing primitives + `openUri` written in that lane's idiom — unverifiable here (no 1.21.11 jar, toolchain 21 absent, auto-download disabled) and therefore the first thing to compile on a 1.21.11 rig.

**Launcher half (Svelte).** The Client page now has a master switch with real state text (`Suite activada` / `Suite DESACTIVADA`) and an explanatory banner, accent-insensitive search, category filter derived from the registry, an explicit enabled/disabled label per feature, a results counter and a confirmed reset that PATCHes every registry default in one request; `InstanceHub` shows a suppressed-state chip and `Activado · suprimida por la Suite` text while keeping the granular toggles operable. Verified in a real browser against the running engine (accessibility tree + PATCH round-trips + on-disk file checks); `svelte-check` 566 files / 0 errors, UI build green, `node --test` 288 tests with the same three pre-existing lunar-import failures as the pristine baseline.


## Workstream 4: Personalization, badges, and supporter cosmetics

### Objective

Give every Espectral user a client worth personalizing, mark who plays on Espectral, and let supporters wear cosmetics — without creating gameplay advantage or duplicating what the server plugins already sell. (Scope widened 2026-11-09: items are no longer required to be ones *only a client can render*; see the decisions below.)

### Decisions (2026-09-27, amendments marked)

- **Free for everyone:** menu background packs, client colour themes, HUD themes, and the ⚛ Espectral badge.
- **One supporter mark, no client tier badges.** Ranks are already shown in-game by LuckPerms prefixes (documented format `✦ [VIP]`…`✦ [MVP+]`), so the client adds only a ★ next to the ⚛ for any donor role (VIP, VIP+, MVP, MVP+): `⚛★`.
- **One cosmetic gate: the MVP Discord role** (MVP `1190433806499446895` and MVP+ `1189380795744387182`). There are no cumulative client tiers; every cosmetic in the supporter line unlocks at MVP. The ★ comes from any donor role: VIP (`[Vip-Donador]`) `648324691659849754`, VIP+ `1190433903698268211`, MVP or MVP+. The Booster role is not a donor role.
- **The Discord role is the source of truth** for both the ★ and the MVP line. Staff grant the roles by hand; no purchase path assigns them automatically.
- **Emotes stay local.** Only the wearer sees their own emotes (for example in third person); there is no trigger channel and no peer sync (decided 2026-09-28).
- **Cloaks, never capes.** No write to the vanilla cape layer, no Mojang cape URL, no elytra recolour from a cape texture; an owned Mojang cape can always be shown instead.
- **Widened on 2026-11-09 (was: client-exclusive only).** A supporter cosmetic no longer has to be something only a client can render. Items a Paper plugin with a resource pack, display entities or ModelEngine-style rigs could also draw — hats, bandanas, masks, neckerchiefs, ears, static wings — are allowed, because they are cheap to author and they wear well; the bandanas are the first set. The limit that stays is the next bullet: nothing may duplicate a perk the Espectral server itself sells. The originally qualifying properties (cloth or spring physics, geometry bound to the player's bones at render time, posing the real player model, first-person rendering of your own cosmetic, per-player elytra textures, per-viewer control) are still the things a plugin cannot do, so they remain the bar for *motion*, not for a hat-shaped item existing.
- **Never duplicate server perks.** The server already sells kill particles, kill sounds, walk trails, baby-mob hats, lobby fly, nicknames, join messages, shield patterns, armor trims and death-message tags (`espectral_core` `donations/*`). The client line must not overlap them.
- **Rejected:** crosshair packs, victory screen, launcher profile banner, early access, monthly drops, creator codes, second-nametag or title cosmetics, and client tier badges.

### Free personalization (local, no backend)

| Feature | Scope | Notes |
|---|---|---|
| Menu background packs | Curated Espectral packs, your own image, animated frame-sequence packs (PNG strip + timeline) | Video decoding is deferred: it conflicts with "no asset decoding on the render thread". The load path already exists (`MenuBackground`, `-Despectral.menu.background`). |
| Client themes | One palette drives the Suite, the title menu and the launcher: Oro (default), Carmesí, Amatista, Esmeralda, Glaciar | Requires turning `SuiteTheme` into a lookup, adding a launcher `data-accent` token set, and purging hard-coded accent literals. |
| HUD themes | Cristal, Oro, Mínimo, Píxel | Extract a shared `HudTheme` from the per-lane box literals. |

All three store their choice under `features.<id>.<key>` in `espectral-client.json`, which round-trips today without an engine schema change.

### ⚛ badge and ★ mark (peer-visible)

- `⚛` marks any player whose Espectral client publishes presence; `⚛★` marks a presence backed by a signed grant carrying a donor role.
- Rendered to the left of the server-sent display name in nametags and the tab list. The client never rebuilds or recolours the server name, so LuckPerms prefixes stay authoritative.
- Both glyphs ship as a mod bitmap font (`espectral-menu:badge`) at Mojangles metrics. Vanilla's default font has neither ⚛ nor ★ (⚛ would fall back to 16 px Unifont; ★ drew a missing-glyph box until it moved into the badge font); U+FE0E renders as a visible "VS15" box and is never emitted.
- Being peer-visible, the badge needs the presence path (Phase 3) even though it is free. Only other Espectral clients see it.

### MVP supporter line (launch draft, revised 2026-11-09)

Slots: **Espalda** (one pair of wings), **Cuello** (one neckerchief), **Cabeza** (one head item), **Élitros**, **Gestos**. One item per slot; with the cloth items retired there are no exclusivity rules left (`catalog.base.json` → `exclusive: []`). The **Brazos** slot and its only item, *Vendas Rúnicas*, were **withdrawn on 2026-10-25** after playtest (the wraps read as clutter over the arms and the hair-thin first-person ends clipped the hand); stored loadouts that still name the slot lose the entry on the next sanitize.

| Item (ES / EN) | Slot | Notes |
|---|---|---|
| Alas de Eco / Echo Wings | Espalda | Translucent wings folded at rest that open while falling or jumping, tuck while sneaking, and beat while sprinting, all driven by the render-time pose. |
| Orejas Vulpinas / Vulpine Ears | Cabeza | Tapered fox ears (orange-red, cream inner, dark tips) that twitch on head turns, flop on landing and flatten while sneaking. |
| Orejas Felinas / Cat Ears | Cabeza | Shorter, rounder cat ears (near-black fur, pink inner) that flick on head turns, sweep back while sprinting and flatten further than the fox pair while sneaking. |
| Pañuelo Rojo / Red Bandana | Cabeza | Bandana wrapped around the head with a short knotted tail at the back; hugs the skull inside the vanilla hat-layer allowance, so it reads as cloth on the head, not a box floating above it. |
| Pañuelo Pirata / Pirate Bandana | Cabeza | The same cap with the knot and tail at the side and a darker, worn palette. |
| Cinta Espectral / Spectral Headband | Cabeza | Thin gold-edged headband; the only head item that leaves the top of the head uncovered. |
| Pañuelo Vaquero / Cowboy Neckerchief | Cuello | Neckerchief knotted at the throat with a single hanging front panel that swings like the vanilla cape. |
| Pañuelo de Forajido / Outlaw Mask | Cuello | Bandana pulled up over the lower face, one panel front and one short tail at the back of the neck. |
| Élitros: Vidriera, Nebulosa, Ceniza / Elytra skins | Élitros | Per-player elytra textures (Nebulosa animated); a vanilla server can only retexture the elytra for everyone. |
| Gestos / Emotes | Gestos | Pose the real player model. Local-only: the wearer sees them, peers never do. |

**Retired 2026-11-09:** *Manto Espectral*, *Capucha Errante*, *Jirones de Ceniza* and *Bufanda Llama de Alma* — the smooth cloth-sim items rejected on 2026-10-25 — are deleted from the code, textures, catalog and tests together with the cloth simulation, its torso collider and the `exclusive` rule they needed. They are not coming back as designed; a cloak returns only as a box-built, panel-hinged item in the data pipeline.

A hat-shaped item is no longer excluded by principle (see the widened decision above); it is still excluded if the server already sells it — the server sells baby-mob hats, kill particles, kill sounds and walk trails, so those stay out. Shoulder pets, walking companions and particle auras stay out too: the server already reproduces them, and a companion returns only if it is designed around client-only behaviour.

### Ruta Espectral (MVP network perk, decided 2026-11-07)

An opt-in, per-server relay for Minecraft traffic, for MVP and MVP+ only. It is not a VPN: only the
game connection to the servers the player picks is relayed; the rest of the PC is untouched, and
no driver or admin rights are involved.

- **Relay:** `ruta.uhce.top` (OVM2, Oracle Ashburn, ports 25565 and 443), source and wire spec in
  `services/ruta-relay/` (`PROTOCOL.md`). OVM2 was emptied for it: the Pterodactyl panel moved to
  OVM3 the same day, and OVM2 keeps only the Galera arbitrator. Its public IP no longer appears in any
  allowlist, and on the tailnet it can reach only the two Galera nodes on port 4567.
- **Entitlement:** the relay verifies the player's existing espectral-id grant (same Ed25519 key and
  claim rules as the mod) and requires the `mvp` flag; no new backend.
- **Client:** a toggle in each multiplayer-list row, off by default, shown only to MVPs.
  *Amended 2026-11-07:* it is now shown on Espectral's own servers too. It was hidden
  there because routing them put every MVP behind one relay IP for AuthMe limits and
  IP bans. The relay now sends the player's real address in a PROXY protocol v2
  header to the PROXY-aware fronts (`--proxy-v2`), so those limits and bans keep
  seeing each player. With the toggle on, the status ping and the join both go
  through the relay, and the shown ping is the real routed ping. A server without a
  PROXY-aware front sees only the relay's address. If the relay refuses or is down,
  the connection fails with a message naming Ruta Espectral; it never silently goes
  direct.
- **Honest pitch:** hides the home IP and gets past networks that block Minecraft's port. It is not a
  lower-ping product from Spain: the relay is in the US, which suits US servers and adds latency to
  European ones.
- **Abuse limits:** IP-literal destinations only, no private, tailnet or metadata ranges, ports
  1024–65535 minus database/remote-desktop ports, per-player concurrency, rate and daily byte quota,
  per-IP and global caps. Logs keep one line per connection (no payload).

### Engineering rules (fixed)

- Benefits are cosmetic-only, with one recorded exception: Ruta Espectral (above), a network-routing perk that changes nothing in gameplay. Fullbright, no-fog, macros, HUD data, themes, backgrounds, performance controls, the media player and the multiplayer ping/flag display remain free.
- One equipped item per slot. Every cosmetic is individually toggleable and respects the Suite master.
- Viewers can hide all cosmetics or hide a specific player.
- Reduced motion replaces simulation and animation with a static pose; global particle caps apply to any particle use.
- Physics and animation run only for the nearest eight visible wearers within 24 blocks; the rest render a static pose (the motion drivers in `docs/cosmetics-pipeline.md`).
- Every item has a fixed footprint cap, enforced by the build (`--check`, rule 4): at most **0.6 px above the head top** — the thickness of vanilla's own hat layer, so a head-hugging bandana is legal — wider than the shoulders plus one block (|x| ≤ 12.5) or lower than the knees, and no transparency below a minimum opacity (alpha 0 or ≥ 200). Checked at rest and at every reachable extreme of each motion driver. **Recorded exception:** the two ear items (`orejas_vulpinas`, `orejas_felinas`) may rise up to 5 px (0.31 block) above the head top — no taller than vanilla's own extra-ears model — because an ear that stays under the skull silhouette reads as a flat box instead of an ear. The ears add no collision, stay inside the width cap, and never render above that bound at any reachable reaction angle.
- Nothing renders when the wearer is invisible to the viewer (Invisibility, spectator) or when vanilla would hide the wearer's cape layer.
- Reactions use only publicly visible pose state: sneak, sprint, fall, swim, glide and idle. They never react to health, damage, inventory or other state that would leak information.
- Cosmetics never alter hitboxes, imitate armor, mob silhouettes, the Glowing outline or other status effects, occlude opponents, or add a second nametag or title text.
- Server rank perks remain separate from client cosmetics.

### Peer visibility

“Visible to other players” means **visible to participating, supported Espectral Fabric clients**. Vanilla clients and brand-only NeoForge clients see an unchanged player. Unlike plugin cosmetics, this works on any server the wearer and the viewer share.

No Minecraft plugin jar or configuration change is required, but a first-party HTTPS backend is required:

1. The launcher proves Minecraft UUID ownership with the standard session handshake: it calls Mojang `session/minecraft/join` with a backend-issued `serverId`, and the backend confirms with `hasJoined`.
2. The launcher links a Discord identity: it keeps the Discord client ID, while the code exchange and the client secret live in the backend.
3. The backend resolves immutable Discord role IDs in guild `648307511648321536` using a bot credential and derives two flags: `supporter` (any donor role) and `mvp` (MVP or MVP+).
4. The backend issues a short-lived, Ed25519-signed grant bound to that UUID and audience.
5. The launcher equips only catalog items allowed by the grant.
6. A viewer requests presence only for UUIDs already visible through the vanilla player list.
7. The viewer verifies the grant itself and renders only catalog items whose models and textures ship inside the mod jar; v1 never loads remote assets.

The backend is `espectral-id` (`services/espectral-id/` in this repository), served at `https://id.uhce.top` from OVM3 (`https://id.uhce.top/v1/healthz` → `{"ok":true,"kid":"eid1"}`). The `espectral.es` zone lapsed around 2026-10-22, so `https://id.espectral.es` is no longer the canonical name; installs that persisted it are migrated to the `uhce.top` host on config load.

**Issuer migration.** Grants carry an `iss` claim. As of this change the launcher engine (`src/engine/grant.mjs`) and the mod (`cosmetics/GrantVerifier.java`) accept **both** `https://id.uhce.top` and the legacy `https://id.espectral.es`; every other claim check (aud, kid, typ, alg, sub, exp, iat skew) is unchanged. The service still signs with `GRANT_ISS=https://id.espectral.es`; flipping that constant to `https://id.uhce.top` (`services/espectral-id/app/config.py`) is a separate ops step that can follow once this build is out, and it is safe in either order — the clients accept both issuers.

**Discord.** The launcher authorizes with the public application id `1454618588659843072` (built-in default, overridable from `config.json`/env), which is the same application `espectral-id` uses for its server-side code exchange; only the secret stays on the service.

A local tier flag, forged brand, wearer-provided URL, or wearer-supplied model grants nothing. The mod never contacts Discord; entitlement is resolved by the backend and handed to the mod as signed data. Cosmetic sharing is a choice distinct from telemetry consent; it is **on by default** (decided 2026-10-24) and can be turned off in Armario at any time. Presence is published only for verified accounts. Sharing off publishes no presence and shows no ⚛.

### Runtime architecture

The local Node engine remains the network-policy authority. A per-launch authenticated loopback bridge allows the Fabric mod to report visible UUIDs; the engine batches HTTPS polling and stages bounded, signed cache updates for the mod.

Start with batched HTTPS polling. Do not add WebSockets, P2P, custom Minecraft packets, or a plugin relay without measured need. Emotes are local-only, so they need no trigger channel.

### Rollout order

1. Free local personalization: background packs, themes, HUD themes.
2. Signed catalog, role fixtures, UUID proof, and the entitlement contract.
3. Own-player elytra skins from valid grants (cloaks return only as data-pipeline items).
4. Presence, bridge, and the ⚛/★ badge on nametags and the tab list.
5. Peer-visible wings, ears and bandanas.
6. Local-only emotes for MVP.

### Playtest status (2026-10-25, owner, 1.3.15 on 26.2 — updated 2026-11-09)

| Item | Verdict |
|---|---|
| Élitros (Vidriera / Nebulosa / Ceniza) | ✅ approved |
| Orejas Felinas | ✅ approved |
| Alas de Eco | ⚠️ not approved as built (smooth cloth stack); now rendered from data — needs a fresh playtest |
| Orejas Vulpinas | ⚠️ rebuilt in 1.3.15, not re-approved; now rendered from data — needs a fresh playtest |
| Pañuelo Rojo, Pañuelo Pirata, Cinta Espectral (cabeza) | ⏳ pending — first items authored in the data pipeline |
| Pañuelo Vaquero, Pañuelo de Forajido (cuello) | ⏳ pending — first items authored in the data pipeline |
| Manto Espectral | 🚫 **retired 2026-11-09** (rejected 2026-10-25 as non-Minecraft; deleted with the cloth simulation) |
| Bufanda Llama de Alma | 🚫 **retired 2026-11-09** (same verdict; deleted with the cloth simulation) |
| Capucha Errante, Jirones de Ceniza | 🚫 **retired 2026-11-09** (never approved; built on the retired cloth stack) |
| Vendas Rúnicas | withdrawn 2026-10-25 |
| Emotes (B) | ⚠️ reworked in 1.3.15 with pose-geometry tests; not yet re-playtested |

**Design direction (owner, 2026-10-25): cosmetics must look like Minecraft.** The approved items (elytra, cat ears) are built from boxes and pixel textures at the player model's 1 px scale; the smooth cloth sheet is not. 2026-11-09: rather than redesign the four rejected cloth items, the owner retired them and moved authoring to a data pipeline built on this same direction:

- Geometry is cuboids on the 16 px-per-block grid, like vanilla's cape (one 10×16×1 panel) and the ears, with no smooth, subdivided cloth surfaces. A head-hugging item may reach 0.6 px above the head top (the thickness of vanilla's hat layer).
- Textures are flat pixel art at model resolution (one texel per model pixel), with no painted gradients or fine shading.
- Motion is rigid: whole panels or a short chain of 2–3 box segments hinged at their edges and swinging like the vanilla cape and elytra, never a deforming mesh. The cloth sim and its torso collider **retired on 2026-11-09** together with the four smooth items.
- A scarf is a box wrapped around the neck plus one or two hanging box-segment tails; a cloak is one or two panels hinged at the shoulders. Cloaks and scarves return only as data-pipeline items satisfying the bullets above.

All cosmetic geometry and textures are now authored as data in `branding-mod/cosmetics/` — Blockbench-compatible Bedrock geometry, pixel-art textures (or a text pixel-art source) and declarative per-bone motion — and compiled by `scripts/cosmetics/build.py`; see [cosmetics-pipeline.md](cosmetics-pipeline.md). The first artist is an LLM (deepseek-v4.1-flash) working in that format. No third-party assets.

### Acceptance gate

- Role fixtures map every combination correctly: no role → ⚛ only; any donor role → `supporter`; MVP or MVP+ → `supporter` + `mvp`.
- Wrong UUID/audience, unknown key, tampered signature, expired grant, forged brand, local tier edit, unknown asset, or peer URL renders no ★ and no MVP cosmetic.
- Role removal and revocation are reflected within the documented entitlement lifetime.
- Share-off produces no presence publication.
- Presence contains only the UUID, permitted loadout/grant reference, and short expiry; it contains no coordinates, chat, or server address.
- Presence history is not retained and expires within minutes.
- Vanilla packet capture contains no Espectral custom gameplay payload.
- A healthy-service loadout becomes visible to an eligible viewer within a target p95 of five seconds.
- Backend failure never blocks game launch or the render/game thread.
- An invisible or spectating wearer renders no cosmetic on any viewer.
- The initial eight-player cosmetics scene (eight wearers, wings + ears + bandanas, motion drivers on) stays inside the global boot/frame regression envelope.

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

- Telemetry and cosmetic sharing use separate consent flows. Telemetry defaults off; cosmetic sharing defaults on (2026-10-24) and stays one switch away from off.
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
- Confirm the donor and MVP/MVP+ role IDs in guild `648307511648321536` (the guild ID is confirmed; no role ID is recorded anywhere in the ops workspace).
- Adopt the Mojang `join`/`hasJoined` session handshake as the UUID ownership proof (Workstream 4, Peer visibility step 1).
- Decide multi-account, gifting, offline-account, Boost expiry, role-loss, chargeback, and revocation policies.
- Approve Spanish/English consent, retention, deletion, and minors treatment.
- Assign backend, signing-key, catalog, privacy, and operational owners.
- Capture missing in-game frame-time baselines. *(done 2026-09-13 — four locked scenes baselined on radeonsi hardware; see Workstream 2 "Instrumentation and first baseline")*
- Freeze additive/versioned schemas for Suite config, `/api/v2`, entitlement, catalog, presence, and telemetry.
- Establish beta/stable feature flags and rollback behavior.

### Phase 1: Truthful, valuable core

- Automatically apply valid AOT caches to normal launches and expose proof. *(done 2026-09-11 — see Workstream 1 "Measured outcome")*
- Add locked performance scenes and instrumentation. *(done 2026-09-13 — file-gated bench engine in the mod, bench-fps.py harness, check-envelope.py regression gate, espectral-bench world + MakeUp shaderpack; see Workstream 2)*
- Migrate to schema 2 and canonical feature defaults. *(done 2026-09-12 — schema 2 with `suite.enabled`, one canonical registry for launcher + mod, `nofog` resolved to off)*
- Build the title, Esc, Suite, master-toggle, recovery, and Apoyar experience. *(done 2026-09-12 on 26.2 — see Workstream 3 "Measured outcome"; the Esc additions and the 1.21.11 lane still need a rig with a world / a 1.21.11 toolchain)*
- Put Fedora artifacts into CI with signed install, update, and rollback paths.
- Remove unsupported fixed-percentage, competitor, and verified-client messaging.
- Ship the free personalization set: menu background packs, client themes, and HUD themes (Workstream 4).

This release remains useful without a backend.

### Phase 2: Identity and own cosmetics

- Deploy account linking and backend Discord-role resolution.
- Add signed entitlement/catalog issuance, cache, expiry, and revocation.
- Build launcher equip controls.
- Render own-player elytra skins and model items only from valid grants.
- Keep telemetry local.

### Phase 3: Peer expression

- Add separate “share my cosmetics” consent.
- Implement the authenticated mod-to-engine loopback bridge.
- Poll presence only for vanilla-visible UUIDs.
- Verify viewer-side grants and content-addressed assets.
- Add global and per-player hide controls.
- Add the ⚛/★ badge first, then peer-visible wings, ears and bandanas, after the frame-time, shader, accessibility, and abuse gates. Emotes stay local-only.

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
- The exact eight-mod preset and Balanced visuals remain intact.
- Custom UI and master state pass on both supported Fabric versions and required resolutions. *(26.2 side met 2026-09-12 at 854×480 and a 1920-wide window — see Workstream 3 "Measured outcome"; the 1.21.11 lane is still unverified on this box, so this gate stays open)*
- Spoofed brand, local tier, peer URL, tampered asset, and invalid grant cannot produce paid cosmetics on a conforming viewer.
- Share-off sends no presence publication.
- Telemetry-off sends no analytics upload.
- Entitlement operation never silently enables analytics.
- Vanilla packet capture contains no Espectral custom gameplay payload.
- Backend, DNS, signature, or cache failure never blocks launch or gameplay.
- Zero-equipped cosmetics pass the global frame and boot regression envelope.
- The initial eight-player cosmetics scene (motion drivers on) adds no more than 5% to p99 frame time and performs no network or decode work on the render thread.
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

Performance claims must name the hardware, operating system, Minecraft version, loader, JDK, mod stack, visual profile, cache state, and benchmark method. FastClient’s 60-mod result is not directly comparable with Espectral’s preset (seven mods in the measurements above, eight since 2026-11-11); Lunar and Feather comparisons remain suspended until equivalent reproducible automation exists.

## Guiding principle

Deliver the measured boot improvement and useful UI first. Contain network, identity, and cosmetic risk behind explicit contracts and feature flags. Make paid benefits expressive but fair. Treat telemetry as untrusted client reporting even when signed. Promote every ambitious claim only after the exact behavior has produced reproducible evidence.