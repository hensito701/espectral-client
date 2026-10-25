# Changelog

All notable changes to Espectral Client. Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [1.3.18] - 2026-10-25

### Fixed
- **26.2 no longer launches on Java 24.** Modern instances accepted any JDK
  21 or newer found on the machine, so a player with Java 24 installed saw
  Fabric refuse every mod ("necesita la versión 25 o posterior … tienes 24").
  The launcher now picks a runtime that meets the version's own requirement
  (26.2 → Java 25) and downloads Temurin 25 when the machine has none.
- **"Vincular Discord" works in one click.** The button stayed greyed out
  until the Minecraft account was verified, so players who had signed in to
  the launcher with Discord thought it was linked and stuck at "Discord sin
  vincular". Linking now runs the verification first when needed, and the
  hint says the launcher's Discord sign-in is a separate identity.

## [1.3.17] - 2026-10-25

### Fixed
- **Microsoft sign-in works again for new accounts.** Since 1.3.15 the
  launcher opened `microsoft.com/link?otc=<code>` with the code pre-filled,
  and Microsoft rejects that link for personal accounts ("The application is
  a first party application … users are not permitted to consent", 
  `invalid_request`). The launcher now opens the plain `microsoft.com/link`
  page and shows the code to type in, which signs in normally.

## [1.3.16] - 2026-10-25

### Fixed
- **"launch error: fetch failed" now says what failed, and retries first.**
  Metadata fetches (Fabric loader list, Mojang manifests, maven checksums)
  retry twice with backoff on connection errors, timeouts and 429/502/503/504.
  When every attempt fails, the error names the host and the network reason
  (for example `could not reach meta.fabricmc.net (UND_ERR_CONNECT_TIMEOUT)`)
  instead of Node's bare `fetch failed`.
- **Server radar and Discord status use the live server names.** The radar
  pinged `uhc.espectral.es`, `uhc2.espectral.es` and `24h.espectral.es`, which
  stopped resolving with the zone, so every card showed offline. It now shows
  MAIN (`main.uhce.top`, UHC and every swapper mode) and the 24/7 Meetups
  server (`24h.uhce.top`), and the Discord status says "Jugando en
  main.uhce.top" / "24h.uhce.top".

## [1.3.15] - 2026-10-25

### Changed
- **Arm wraps withdrawn (2026-10-25).** Playtest found the cloth forearm wraps
  read as clutter over the arms rather than a cosmetic. `vendas_runicas`, the
  whole `brazos` slot and its plumbing are gone: the three catalog copies (still
  byte-identical, now 10 items), the first-person `CosmeticHand` layer, the
  arm-band model, the wrist chains in `WearerSims`, the `ClientConfig` slot bit,
  the Suite submenu's slot button, `suite-registry`'s `slot_brazos` state key
  (regenerated), the `espectral.slot.brazos` / `wardrobe.slot.brazos` strings and
  the texture. Persisted loadouts that still name the slot are sanitized to drop
  it instead of erroring (covered by a test); `validateLoadout` still rejects a
  client that sends it.
- **Manto Espectral redesigned.** The mantle was a flat dark slab from shoulders
  to calves with a painted diagonal strap. It is now a real one-shoulder drape:
  the sheet is pinned high on the left shoulder (clasp side) and sweeps down to
  the opposite hip, its hem ends between hip and knee instead of past the knees,
  and it is drawn with three soft fold ridges that deepen toward the hem, so it
  reads as cloth from behind and from the side. The folds are applied at draw
  time rather than as a rest shape: the solver's in-plane links flatten any
  rest bulge, which is exactly why the old mantle resolved to a slab. The
  texture carries matching vertical fold shading, a woven twill grain, a round
  gold clasp boss on the clasp shoulder and a gold-mist hem that dissolves into
  light. `tools/MantleShapeTest.java` replays the sheet simulation and asserts
  the length, footprint, torso clearance and fold depth (29 checks).
- **Support/donations URL is `https://espectral.uhce.top/donaciones`.** The
  Donaciones page's store button, its "copy link" action and the label under
  the store title all use the `uhce.top` host now (the `espectral.es` name
  301-redirects to it). The label is derived from the link itself, so the two
  can no longer drift.
- **Cosmetic sharing is on by default.** New accounts show `⚛` (and their
  equipped cosmetics) without opting in; the Armario switch turns it off and
  no longer requires a verified account to toggle. Presence is still only
  published to `espectral-id` for verified accounts.
- **In-game Suite support URL moved to `uhce.top`.** `SuiteScreen.SUPPORT_URL`
  is `https://espectral.uhce.top/donaciones`, so the in-game *Apoyar ♥*
  confirmation opens the live host instead of the lapsed `espectral.es` zone.
- **Title menu polish.** The background PNG is decoded off the render thread
  (`MenuBackground`), uploaded once per session with linear filtering, and
  fades in when it is ready. It drifts in a smooth sub-pixel Ken Burns motion
  with a light mouse parallax. The hard-edged half-screen scrim is now a soft
  horizontal gradient, with bottom and top vignettes. The logo floats slowly
  over a breathing gold halo and a dark pool that keeps it readable. Other
  additions: gold motes, a localized tagline ("Tu juego, a tu manera"),
  animated hover (accent bar, chevron), Up/Down/Tab + Enter keyboard
  navigation, and icon tooltips. The camera icon opens the screenshots
  folder (it was a no-op before), and Esc no longer closes the root title
  screen.
- `assets/branding/branding.json` now pins the jars that actually ship. Its
  sha1s were stale, so the engine logged `branding_sha1_mismatch` on every
  launch and the pin test failed.

### Added
- **Orejas Felinas (`orejas_felinas`), a new head cosmetic.** Shorter and
  rounder than the vulpine pair: near-black fur with a pink inner ear, a fur
  cuff at the base, a gentle flick on head turns, a perk-up while sprinting and
  a flatter fold than the fox ears while sneaking. It ships in all three
  catalog copies (mod resources, engine, `espectral-id`) and its texture in the
  mod jar, so `espectral-id` needs a redeploy before the live service accepts
  the new id.
- **Fox ears are real ears.** `orejas_vulpinas` was two flat boxes on the
  skull. It is now three stacked, tapering boxes per ear (3 → 2 → 1 px wide)
  splayed outward from the top corners of the head with a proud cream inner-ear
  panel and a near-black tip: a tapered triangle from the front, an ear from the
  side. Rest splay, twitch on head turns, landing flop and the sneak flatten are
  all preserved.
- **Suite sub-screens: Cosméticos and Macros.** The Cosméticos and Macros
  feature rows now carry a `⚙` button that opens an in-game settings screen
  (same dark-glass sheet, keyboard navigable, `Volver`/Esc returns to the
  list). Cosméticos holds viewer-side toggles — show my cosmetics, show other
  players', per-slot visibility for Espalda/Cuello/Cabeza/Brazos/Élitros,
  emotes and cloth physics — stored under `features.cosmetics.*` in
  `config/espectral-client.json`; the emotes switch is ANDed with the
  launcher's `espectral-cosmetics.json` flag (that file is never written by
  the mod), and physics-off renders the same static pose as reduced motion.
  Macros gains a full editor: paginated list, add/edit/delete, a
  "Pulsa una tecla…" capture button (Esc cancels), layout-aware key labels
  (`ñ`, `ç`), a conflict check against `Options.keyMappings`, the Suite
  registry keybinds and other macros (duplicates are refused, vanilla/mod
  clashes need a second confirm), Esc/Right Shift rejected, 32 macros and 256
  chars per action. Each action still only ever sends chat or a vanilla
  command.
- **`branding-mod/lang-extra.json`.** Hand-maintained Suite UI strings (ES/EN)
  merged by `scripts/sync-suite-registry.mjs` on top of the generated lang
  files, so new screen text no longer has to live in registry-owned keys. The
  script hard-fails if an extra key redeclares a registry-derived one.
- **Free personalization for everyone.** Five client themes (Oro, Carmesí,
  Amatista, Esmeralda, Glaciar) recolour the launcher, the Suite and the title
  menu from one setting (`/api/accent`, `personalization.theme`). Menu
  background packs: import your own PNG or an animated frame pack (≤300
  frames, ≤1280×720) from the instance Appearance card; animated packs are
  decoded off the render thread and loop. HUD themes (Cristal, Oro, Mínimo,
  Píxel) restyle the HUD boxes and the HUD editor. A "Movimiento reducido"
  setting stills the title-menu motion, cosmetic physics and emotes. A
  background placed by hand at `config/espectral-menu-background.png` is
  imported once as the pack "Mi fondo" and selected.
- **⚛ Espectral badge and ★ supporter mark.** Players who share their
  cosmetics show `⚛` before their name in nametags and the tab list for other
  Espectral players; any donor Discord role adds `★`. Server names and
  LuckPerms prefixes are never rebuilt. The glyph ships as a bitmap font at
  Mojangles metrics, and the title/pause buttons use it instead of ★.
- **MVP cosmetics (client-only).** Manto Espectral, Capucha Errante and
  Jirones de Ceniza (cloth physics), Alas de Eco (react to falling, sneaking
  and sprinting), Bufanda Llama de Alma, Orejas Vulpinas, Vendas Rúnicas
  (also on your own first-person hands) and three elytra skins. They render
  only from a verified grant, never on invisible or spectating players, stay
  inside a fixed footprint, and simulate physics for at most the eight
  nearest wearers. The launcher's new Armario page (`#/armario`) verifies the
  account, links Discord, and holds the sharing consent, the loadout and the
  hidden-players list.
- **Local emotes for MVP.** Press B for the emote wheel (Saludo, Reverencia,
  Aplauso, Sentarse, Invocación). Only you see them; movement, attacking or
  taking damage cancels them.
- **`espectral-id` service** (`services/espectral-id/`, live at
  `https://id.espectral.es` on OVM3): Mojang ownership proof, Discord role
  resolution for guild `648307511648321536`, Ed25519 grants and in-memory
  presence. The record is DNS-only with a Let's Encrypt origin certificate,
  because the zone's Flexible SSL mode would send bearer tokens from
  Cloudflare to the origin in plaintext.
- **Supporter cosmetics + identity plumbing (Workstream 4, engine).** The
  engine now verifies Minecraft uuid ownership through `espectral-id`
  (`POST /api/identity/:uuid/verify` runs the Mojang `join`/`hasJoined`
  handshake), links Discord without shipping the OAuth client secret
  (`/v1/discord/identify` + `/v1/discord/link`), and keeps a per-uuid session
  in `<data>/identity.json`. Grants are Ed25519 JWS tokens verified against a
  pinned public key (`kid eid1`); a missing or invalid grant means no ★ and
  no cosmetics. `GET/PUT /api/cosmetics` validates loadouts against the
  bundled catalog (slot, flag requirement, scarf↔cloak exclusivity) and the
  engine writes `<gameDir>/config/espectral-cosmetics.json` at launch,
  mirrored into the per-account profile dir like `espectral-client.json`.
  The in-game mod reaches the engine through a per-launch authenticated
  loopback bridge (`POST /api/bridge/visible`, `x-espectral-bridge` token);
  uncached uuids are batched to `/v1/presence/query` in the background
  (≤1 req / 3 s, 60 s positive and negative cache). Presence publishing
  follows running instances: `PUT /v1/presence` every 240 s while `share` is
  on, `DELETE` on exit or share-off. Grant refresh at launch is capped at 2 s
  and never blocks or fails the launch.
- **Tab list heads (`tabheads`, on by default).** Vanilla only draws the face
  column when the server runs Mojang authentication (26.2:
  `ClientPacketListener#onlineMode`; 1.21.11: `isLocalServer() ||
  Connection#isEncrypted()`), so offline-mode servers behind AuthMe never got
  one. The new Suite feature forces that check on, which also widens each
  column by the 9 px face so names and the ⚛/★ badge shift together instead of
  overlapping. Skins come from each `PlayerInfo`, never from a network fetch of
  our own.

### Fixed
- **Sneaking no longer hides cosmetics.** `CosmeticLayer` bailed out on
  `AvatarRenderState.isDiscrete`, and in 26.2 `Entity#isDiscrete()` is defined
  as `isShiftKeyDown()` — so every cloak, ear and wrap vanished the moment the
  wearer held shift. The guard is now only invisible-to-viewer and spectator;
  sneaking renders everything, and the crouched body pose (0.5 rad torso tilt,
  lowered head) carries the cosmetics with it because each piece is entered
  from its real `ModelPart` transform. Ears still visibly flatten on sneak.
- **Cloaks no longer clip into the body.** The cloth sim now collides with the
  torso: an armor-aware box (2 px half depth + 0.4 px skin offset, plus 1 px
  chestplate inflation and vanilla's 1.1 px cape-layer offset when a chestplate
  is worn) pushes every non-anchor point to the nearer face, run inside the
  constraint iterations so the cloth slides along the chest instead of
  collapsing, and the box stops at the hip line once the torso hinged away
  (bow). Gravity and wind are rotated into the body frame before stepping, so a
  crouching wearer's cloak still hangs along world-down instead of along the
  tilted torso. The mantle sheet's top row is pinned to the shoulders, so the
  neck seam never detaches.
- **Cosmetics follow the emote pose.** The layer used to draw everything at a
  fixed model-root offset. It now starts from `model.root()` and enters the
  wearer's head (ears, hood), body (cloak, scarf, tatters, wings) or arm
  (runic bands) `ModelPart` before emitting, so the sit root drop (−11 px), the
  bow's hip hinge and the sneak drop move the cloth and ears with the posed
  body instead of leaving them at standing height.
- **Emotes read naturally and no longer clip the body.** The five local emotes
  are re-derived from the player model's real pivots in a new Minecraft-free
  `EmotePose` class (head at the neck, arms at (±5,2), hips at (±1.9,12)) and
  cross-fade in and out over 250 ms: *Saludo* raises the right arm out to the
  side (the old pose rotated it to −π+0.45 rad, i.e. across the face, so the
  hand waved inside the head) and looks towards it; *Aplauso* swings both arms
  forward so the palms actually meet on the centre line (solved contact angle)
  three times a second; *Reverencia* hinges at the hips (head, chest and arms
  rotate about the invariant hip point, so nothing folds); *Sentarse* drops the
  whole model 11 px through the root — cosmetics anchored to the part
  transforms follow — with the legs forward on the ground and arms on the lap;
  *Invocación* swings the arms forward then opens a V clear of the head with a
  0.5 px hover bob. Head and chest offsets stay additive so the player can keep
  looking around mid-emote. `branding-mod/tools/EmotePoseTest.java` samples
  every frame of every emote and asserts the clearances (532 checks).

- **"Vincular Discord" works on a stock install.** The engine had no default
  OAuth client id, so every real install failed with *Discord client id is not
  configured in config.json*. It now falls back to the public Espectral
  application id `1454618588659843072` — the same app `espectral-id` exchanges
  the code with, so `/v1/discord/link` accepts it. `config.json`
  (`discord.client_id`), `DISCORD_CLIENT_ID` and `ESPECTRAL_DISCORD_CLIENT_ID`
  still override it.
- **Identity service points at `id.uhce.top`.** The `espectral.es` zone lapsed
  around 2026-10-22 and `identity_url` still defaulted to
  `https://id.espectral.es` — including the literal value every existing
  install had persisted. The default is now `https://id.uhce.top`, and a
  persisted legacy value (with or without trailing slash) is migrated on config
  load. A custom self-hosted URL is left untouched and `ESPECTRAL_ID_URL` still
  wins.
- **Grants verify across the issuer flip.** Both `https://id.uhce.top` and the
  legacy `https://id.espectral.es` are accepted by the engine verifier and the
  mod's `GrantVerifier`; aud, kid, typ, alg, sub, exp and iat skew are
  unchanged. The service can flip `GRANT_ISS` on its own schedule without
  invalidating grants held by already-shipped clients.
- **Custom main menu now shows its background photo.** Vanilla's
  `blit(Identifier, x0, y0, x1, y1, …)` takes its floats as `u0, u1, v0, v1`
  (checked in the 26.2 and 1.21.11 bytecode). The mod passed `0, 0, 1, 1`,
  which drew one texel of the image (its dark bottom-left corner) across the
  whole screen. The helper is now `Compat.uiBlitUv`, which uses the correct
  order in both lanes.
- **Chat heads show up on plugin-formatted servers.** Servers running a chat
  plugin (the Espectral ones among them) broadcast player chat as a *system*
  message — `<[ADMIN] henMV> asd` — and the feature only mapped signed
  `PlayerChatMessage`s, so those lines never got a head. System lines now
  resolve a sender by matching an exact, whole-word tab-list profile name
  inside the line's sender prefix (the text before the first `>`, `:`, `»` or
  `|`), ignoring bracketed rank tags. Broadcasts with no separator
  (`¡Bienvenido a ESPECTRAL, henMV!`), names that only appear in the message
  body, and names that are a mere substring of a longer word stay headless,
  and signed player messages still take the exact path.
- **Chat heads stay visible while the chat is open.** The heads were drawn only
  in the unfocused HUD pass, so pressing T made them disappear while the text
  stayed. The focused pass (`DrawingFocusedGraphicsAccess`) now draws the head
  and shifts its line by the same 10 px, restoring the pose per line, so
  scrolled lines keep their own heads. The click/tooltip capture pass applies
  the same shift, so links stay clickable where they are drawn.
- **The ★ supporter mark renders as a star.** It inherited the badge font,
  which had no ★ glyph, so it showed as a missing-glyph box after the ⚛ in the
  tab list and nametags. ★ now ships in the badge bitmap font next to a
  redrawn 16 px ⚛ (nucleus and three orbits instead of a crossed circle).

 ## [1.3.14] - 2026-09-06

 ### Added
- **Linux compatibility (.deb for Ubuntu 24.04 / Debian 12+, x86_64).**
  Tauri shell resolves the staged `node-bin/` runtime on Linux, uses
  `$XDG_DATA_HOME` (else `~/.local/share`) for the data dir, and reclaims a
  stale engine port via `/proc` (socket-inode match + `cli.mjs`/node identity
  check). The bundle targets `deb` (with `libwebkit2gtk-4.1` runtime dep;
  release builds run on the oldest supported base for glibc compat), the
  staging script handles the Linux `tar.xz` Node artifact, and
  `scripts/build-linux-deb.sh` documents the native build. Engine: POSIX
  import sources (`~/.minecraft`, Flatpak variants, `~/.lunarclient`),
  JDK discovery skips the Windows-only FastClient path off-Windows, and
  Discord presence tries the Linux abstract socket. File/folder picking on
  Linux uses the native Tauri dialog from the UI (Windows keeps the
  PowerShell picker byte-identical); the engine pick endpoints answer 501
  `UNSUPPORTED_PLATFORM` off-Windows, which the UI surfaces as a cancel.
 - **Client identifies via the `minecraft:brand` handshake.** The bundled jars
   report `espectral:<loader>[:<version>]` (Espectral Menu on Fabric, new
   brand-only `espectral-brand` companion on NeoForge 1.21.1) instead of the
   loader default, so the server plugin can detect Espectral users on join.
   The preset/retro-seed pick the jar per version + loader. Mod versions track
   the launcher release automatically (single source: `package.json`), and the
   version segment is omitted — never faked — when unknown.
 - **Header earns its space: shortcuts + real logo.** The top bar's empty
   center now holds quick pills for the Skin Atelier (deep-links into
   `#/account?section=skin` and scrolls to the 3D visualizer) and the
   Catálogo de Mods, with active-state highlighting. The brand mark drops
   the `✦` glyph for the real Espectral apple logo. Also fixed the vault's
   hero `<section>` never being closed (the rest of the page nested inside
   it in the live DOM).
 
 ### Fixed
 - **Windows installer back to ~31 MB.** The new per-platform node staging
   (added for Linux) keyed off the host OS, so cross-building the NSIS
   installer from WSL staged the 116 MB Linux `node` next to `node.exe`
   and shipped both. Staging now targets `STAGE_NODE_PLATFORM`
   (`win32` in `build-windows-exe.sh`, `linux` in `build-linux-deb.sh`)
   and always prunes the other platform's binary.

 ## [1.3.13] - 2026-09-05

 ### Fixed
 - **Performance preset progress no longer sticks on "Instalando mods...".**
   The install endpoint queues in the background, but the UI read per-byte
   fields the engine never sends, freezing the bar at 0/100 with a premature
   success toast. Progress is now file-count based, the success toast fires
   only on the terminal SSE event, foreign-instance/library/mrpack traffic no
   longer drives the bar, and a fallback timer clears it if the event is lost.
 - **Client Suite feature toggles translate.** All 13 feature names and
   descriptions now have Spanish + English strings instead of hardcoded
   English from the engine registry.
 - **Play panel and "Cuenta Activa" track skin + color changes.** The Home
   account chip and the Play-dock account display used name-only tiles while
   TopChrome showed the fresh avatar. All three now render the skin-head
   avatar with the account color and refresh on profile changes (with a
   cache-buster so replaced bytes never show stale).
 - **Custom-folder instances install mods/configs in their own folder.**
   Mods, client config, `servers.dat`, `options.txt`, and game subdirs now
   resolve under the user-selected `game_dir` instead of leaking into the
   default profile dir, and "Abrir carpeta de mods" opens the effective
   folder (the open-folder guardrail now allows instance game dirs).
 - **Account popover avatars are clean circles.** The active-account card and
   switch-row wrappers painted rounded-square backings behind the circular
   avatar tiles (circle-on-a-rounded-square). Both wrappers are circular now,
   matching every other avatar in the app.

 ## [1.3.12] - 2026-09-05

 ### Added
 - **Skin Atelier: named skin gallery (vanilla workflow).** Save staged uploads
   with a name, import all named skins from the vanilla launcher
   (`launcher_custom_skins.json`, idempotent), rename / delete per card, and
   apply any entry to the active Microsoft account with **Usar**. New
   **Probar** button previews a gallery skin in the rotating 3D stage without
   touching Mojang (also available for offline accounts).
 - **Gallery cards render frozen 3D previews** instead of flat PNGs (paused,
   still draggable; boot lazily on scroll so hidden cards never claim a WebGL
   context). A PNG fallback with a note renders if WebGL is unavailable.
 - **Microsoft account icons follow the skin.** The avatar auto-syncs to the
   head of the account's Minecraft skin (face + hat layer, 64×64) on skin
   fetch / upload / gallery apply. User-uploaded avatars are never clobbered;
   resetting the skin drops only skin-derived avatars.

 ### Fixed
 - **Gallery "Usar" no longer dies with HTTP 405.** Mojang's gateway rings
   disagree on the upload method: at least one ring answers an authenticated
   `PUT /minecraft/profile/skins` with 405 METHOD_NOT_ALLOWED while POST
   succeeds (same token, same bytes). Uploads now go POST-first with a single
   PUT retry on 405 only. Mojang's own rejection reason is surfaced in the
   error message and the server log instead of a bare status.
 - **Skin viewer proportions.** The boot canvas is capped at 300px with the
   aspect ratio locked, so CSS scaling can no longer squeeze skins thin and
   tall. New square pause/play button pauses/resumes rotation.

## [1.3.11] - 2026-09-04

### Changed
- **Fullbright and nofog are now native owned features.** The in-game mod drives gamma live
  (fullbright) and removes fog live (nofog) — no Gamma Utils / Clear Fog jars, no restart.
  Leftover third-party jars are inert; the QoL preset stays installable but is no longer required.
- **Zoom key text fixed: Z, not C.** Hold-to-zoom was always bound to Z; the launcher registry
  said C. It now says Z (`key.keyboard.z`).
- **Launcher toggles reach a running game.** Client feature and macro PATCHes apply live in-game.
- **Macro key support widened.** More keybind forms are accepted, and unrecognized keybinds
  surface per-feature warnings instead of failing silently.
- **Reconcile errors surfaced in the UI.** PATCH responses carry an additive `errors` array; the
  Client Suite toasts them as warnings (listing the feature ids) instead of a blanket success
  toast, without reverting the persisted toggle.
- **Macro editor overhauled.** The keybind box now captures the pressed key (numpad 4 and row 4
  are distinct: `numpad.4` vs `4`); action rows put the value in a full-width box below the
  type selector, with per-type examples (commands need no leading `/`).
- **Page containers unified.** Client Suite, Server Radar, and Version Armory use the same
  centered content rule as the mod catalog, so non-maximized windows keep side margins and
  air under the header.

## [1.3.10] - 2026-09-03

### Fixed
- **Blank window on machines without the WebView2 runtime.** Two layers: the
  NSIS installer now embeds the WebView2 bootstrapper (`embedBootstrapper`)
  instead of downloading it silently at install time, so a failed download
  fails visibly instead of shipping a runtimeless app. As a backstop, the
  shell checks the EdgeUpdate Clients registry key at startup and — if the
  runtime is absent — shows a native dialog offering the official evergreen
  installer instead of booting into a permanent blank frame.

## [1.3.9] - 2026-09-03

### Fixed
- **Fresh 1.3.8 installs opened to a blank dark window.** The rewritten updater
  module called `isTauri()` without importing it, so the whole UI bundle threw
  during startup and nothing mounted. The file carries `// @ts-nocheck` (the
  Svelte 5 `.svelte.ts` parser can't take TS annotations), which is why the
  type-check stayed green. Fixed with the missing import; the packaged bundle
  is now boot-verified in a real browser before release.

## [1.3.8] - 2026-09-03

### Fixed
- **One-click updates failed with "Error opening file for writing … node.exe".**
  Two defects combined: the updater ran the NSIS installer twice
  (`downloadAndInstall()` already installs, then `install()` ran it again over
  the same files), and the engine — which runs on the bundled `node.exe` — was
  still alive when the installer tried to replace that file. The update now
  downloads once, shuts the engine down via a new `POST /api/shutdown`
  (running games are untouched), waits for the file lock to release, runs the
  installer exactly once, and relaunches. The health store's engine self-heal
  is suppressed during the install so it can't respawn `node.exe` mid-write;
  if the install fails, the app restarts the engine instead of staying offline.


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


## [Unreleased]

### Added
- **Boot-time JVM flags.** Every launch on Java 21+ now runs with `-XX:-UsePerfData` and
  `-Xverify:none`. A new opt-in `fast_boot` setting (Settings → Engine & Client Features) adds
  `-XX:TieredStopAtLevel=1` (C1-only JIT), which measurably shortens time-to-menu on a modded
  instance at the cost of peak JIT quality once you are in-game. The tradeoff is stated in the UI.
- **In-game Suite UI with a master switch (client config schema 2).** The title, pause and Suite
  screens share one canonical feature registry (`src/engine/suite-registry.json`) that feeds the
  launcher API and is copied verbatim into the mod jar with its EN/ES strings
  (`scripts/sync-suite-registry.mjs`). Configs gain a `suite.enabled` master switch (on by
  default) gating every owned feature; v1 files upgrade in place without touching stored flags.
  Also resolves two defaults against the registry: No Fog defaults to off, and the fullbright
  gamma default is corrected to 1.0.

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
