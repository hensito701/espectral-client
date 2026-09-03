# Changelog

All notable changes to Espectral Client. Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [1.3.7] - 2026-09-03

### Fixed
- **Booting an instance opened two Minecraft windows instead of one.** When the
  game reached the main menu, the engine spawned the AOT trainer immediately —
  a second full game in the same game directory, alongside the one being
  played (double RAM, both writing the same log). Auto-training now waits until
  you close the game, then trains alone, matching what the settings screen
  already promised ("after closing the game"). A manual "train now" click while
  the game runs defers the same way instead of opening a second window, and a
  relaunch during training aborts the trainer spawn (the new session's exit
  re-queues it).
- **The trainer window never closed itself and stayed on as a second instance.**
  The graceful-close step targeted the process "main window", but the training
  game reports no main window, so the close was a guaranteed no-op and the
  10-minute exit wait expired doing nothing. It now asks Windows to close every
  window of the game process (`taskkill` without force, i.e. a polite close
  request), with SIGTERM on other platforms. Force-kill is still never used —
  the cache is only written on a normal exit.


### Added
- **Boot-time JVM flags.** Every launch on Java 21+ now runs with `-XX:-UsePerfData` and
  `-Xverify:none`. A new opt-in `fast_boot` setting (Settings → Engine & Client Features) adds
  `-XX:TieredStopAtLevel=1` (C1-only JIT), which measurably shortens time-to-menu on a modded
  instance at the cost of peak JIT quality once you are in-game. The tradeoff is stated in the UI.

### Fixed
- Launching under a non-active account failed with a server error since 1.0.0 (a missing resolver
  helper). Launching as any saved account works again.

## [1.3.6] - 2026-09-01

### Fixed
- **The AOT cache was being invalidated on every launch, so the speed-up silently never applied.**
  Fabric's metadata publishes no checksum for two loader jars, so the launcher re-downloaded them
  during each verification pass. The content was identical but the file timestamps changed, and
  Java records path, size and timestamp for every classpath entry — so the JVM refused the 147 MB
  cache and the game paid the full slow boot forever. Fixed in three parts: missing checksums are
  now fetched from Maven and cached; a byte-identical re-download keeps the existing file untouched;
  and the launcher records the trained classpath's stamp, skips a stale cache, and retrains at the
  main menu instead of booting slowly. Caches trained before 1.3.6 retrain once.

### Added
- **Update button in the title bar.** When a signed release is available, an "Actualizar a X.Y.Z"
  chip downloads, installs and relaunches, with progress shown in the button. Replaces the floating
  banner. The launcher also re-checks every 6 hours, so a release published mid-session is noticed
  without a restart.
- The in-app update endpoint is now actually published, so in-app updating works for the first time.

## [1.3.5] - 2026-08-24

### Fixed
- **Microsoft sign-in failed for every browser sign-in** ("Minecraft login failed"): the launcher
  read the wrong field names from Minecraft's token response, so a fully successful sign-in still
  failed at the last step. Failed sign-ins now report the actual HTTP status and error text instead
  of a generic message.

## [1.3.4] - 2026-08-23

### Fixed
- **Discord login still hung**: the component that opens your browser was bundled but never
  registered, so nothing happened. It is now initialized, and if opening the browser fails, the
  launcher shows the URL for you to open manually.

## [1.3.3] - 2026-08-23

### Fixed
- **Discord login now opens in your system browser.** The app's embedded WebView blocks popups, so
  the button spun forever. Sign-in happens in your real browser and the app claims the session
  through a one-time link.
- **The density toggle (Compacto/Espacioso) did nothing** — a CSS scoping bug meant none of the
  compact rules could ever match. Instance tiles now visibly resize.
- **Server cards**: removed the decorative ping sparkline. It was derived from a hash, not measured,
  and it overlapped the server info.
- **Orphaned engine after a force-quit**: killing the app from Task Manager left the engine holding
  its port, and the next launch would talk to a stale process. The engine now exits with the app.

## [1.3.2] - 2026-08-23

### Fixed
- Discord sign-in failed with "Failed to fetch" in the desktop app: the engine omitted a required
  CORS header for credentialed requests.

## [1.3.1] - 2026-08-23

First public download build.

### Fixed
- **Fresh installs failed at "natives extraction timed out"** and could never launch the game. The
  bundled Node runtime is now pinned to a version that does not stall during archive extraction.
  Do not distribute 1.3.0 installers.
- The version reported by the launcher is now the real package version in both development and
  installed layouts.
- If Fabric's metadata service is unreachable, the launcher falls back to its cached copy instead of
  blocking launches.

## [1.3.0] - 2026-08-22

Interface rebuild and optional Discord identity.

### Added
- **New interface**: instance rail home screen, `Ctrl+K` command palette, picture-in-picture launch
  log, account popover, and an ambient backdrop that shifts hue to match the selected instance's
  loader. Every screen was rebuilt, and both dark and light themes re-tuned.
- **Optional Discord sign-in**: link your Discord name and avatar as an identity separate from your
  Minecraft accounts. Fully optional — anonymous use is supported everywhere. No Discord tokens are
  ever stored.
- New Fabric instances arrive pre-provisioned: the performance mod preset is queued on creation and
  the AOT cache shows as ready to train.

### Changed
- Zoom keybind moved from **C** to **Z**.
- "No Fog" now defaults to on.
- Instance tiles are about 30% shorter and sit directly on the backdrop.
- Copying a server address no longer shows a toast.
- Deleting an instance no longer asks for confirmation.
- AOT training retries transient download failures and reports the exact failing URL on give-up.

## [1.2.0] - 2026-08-21

Zoom is now built into the Espectral mod — no third-party jar needed.

### Added
- **Zoom**: hold the zoom key to ease the field of view to 0.25x, frame-rate independent, with
  smooth camera engaged while held. It applies live, without restarting the game, and the keybind
  appears in Minecraft's Controls screen.

### Changed
- Zoom is now provided by the Espectral mod itself; the launcher no longer manages an Ok Zoomer jar
  for it. The quality-of-life preset is fullbright and no-fog only. If an older instance still has
  Ok Zoomer installed, disable it on the Mods page to avoid a duplicate keybind.

## [1.1.1] — 2026-08-20

Visual polish. No new features.

### Changed
- Contrast and colour tokens reworked: lighter muted text, desaturated glass surfaces, tokenized
  shadows, a double focus ring, and a light-theme accent that meets WCAG AA on both light
  backgrounds.
- Layout: wider content ceiling, shorter title bar, tightened gaps, responsive card padding.
- Motion: softer glass sheen and orbs, and a `prefers-reduced-motion` guard.
- The pixel font is now limited to the play button and the instance monogram.

## [1.1.0] — 2026-08-20

Fix release.

### Fixed
- **Launching as a non-active account ignored the chosen account** — the account was only honoured
  in dry-run.
- **Native libraries were shared between accounts**; each account now gets its own natives and run
  directory, so concurrent launches cannot collide.
- **The in-game menu was unreachable on older Fabric instances**: the Espectral mod jar is now
  seeded into `mods/` automatically when it is missing.
- **The in-game menu keybind was undiscoverable**: it is now a real keybind, visible and remappable
  in Minecraft's Controls screen.
- **Instance detail had no log selector** when several games were running; it now matches the home
  screen.

### Changed
- Home-screen banners collapsed to a single prioritized slot, and toasts no longer cover the title
  bar on narrow windows.

## [1.0.0] — 2026-08-19

First public open-source release.

### Added — Espectral client mod
- The bundled Fabric mod gains an in-game toggle menu (keybind, plus a title-screen button), a
  versioned config file, and a feature list shared with the launcher.
- Launcher ↔ mod contract: one versioned file, `<instance>/config/espectral-client.json`, written by
  both sides with unknown fields preserved and atomic writes. The launcher seeds defaults; the mod
  writes on toggle.
- Features owned by the mod (macros) apply live in-game. Managed features (fullbright, no-fog, zoom)
  flip a flag that the launcher reconciles against `mods/` at the next launch.
- On Fabric instances the mod's fullbright takes precedence over the launcher's gamma seeding.

### Added — Client page
- A new **Client** page mirrors the in-game menu: per-instance feature toggles and a macro editor.

### Added — macros
- Keybind-triggered chat and command macros (for example `/hub`), with a per-key repeat guard.
  Deliberately limited: no auto-clicker, no CPS manipulation, no anti-AFK, no combat automation.

### Added — concurrent launches
- Launch the same instance several times at once, each under a different account.
- Non-active accounts get an isolated run directory (own options, logs and saves) while versions,
  libraries and mods stay shared and read-only.
- Per-game launch logs with a live selector.
- Discord presence follows the most recently started game and clears when the last one exits.

### Added — Microsoft sign-in
- Clear "pending Mojang approval" explanation instead of a raw Microsoft error code when using the
  default application ID.
- Documented bring-your-own client ID path — see `docs/msa-byo-client-id.md`.

### Notes
- The desktop shell is Windows-only: port reclaim shells out to `netstat`/`taskkill` and bundling is
  NSIS-only. The engine, UI and mod are portable.

## [0.9.0 and earlier]

Pre-release development: the vanilla, Fabric and NeoForge resolvers, `.mrpack` import, Modrinth
integration, Microsoft device-code sign-in, settings import from other launchers, AOT boot caching,
the Tauri desktop shell, and the espectral.es-styled interface.
