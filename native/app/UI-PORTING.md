# Native UI (Slint) — architecture & porting guide

Branch `rust-native`. The Svelte 5 UI (`src/ui/src`) was replaced by a
native Slint UI in the binary crate **`espectral-client`** (`native/app`). The
binary links **`espectral-engine`** (`native/engine`, see `../PORTING.md`) and
calls `Engine` methods directly: no webview, no Node, no HTTP between UI and
engine. Goal: visual + functional parity with the Svelte UI at a fraction of
the RAM (target: well under 150 MB while open; idle = 0% CPU).

**1.7.0 shipped this UI as the only UI.** The Svelte sources (`src/ui/`), the
Tauri shell (`src-tauri/`) and the Node engine were deleted in that release;
they are referenced below only as the historical porting reference (read them
from git history, tag `v1.6.3`). Nothing here is a live dependency any more.

The foundation (this document, the shell, the widgets, the bridge) is done.
Five agents ported the pages in parallel, each in its own worktree, touching
**only their own files** (see [File ownership](#file-ownership)); all five
groups are merged and integrated — see [12. Integration status](#12-integration-status)
for the per-page status, the release size and the memory / CPU measurements.

- Historical reference implementation: `src/ui/src` (deleted in 1.7.0; see git history at tag `v1.6.3`). The live reference is now the Slint UI itself plus `i18n/*.json`.
- Reference screenshots of the (deleted) Svelte UI: `/tmp/claude-0/-home-user-espectral-client/7274c801-56fc-5f5d-bdc2-b83be817cf63/scratchpad/ref-screens/*.png` (index below).
- Headless screenshots of the Slint UI: `native/app/scripts/screenshot.sh` (below).

---

## 1. Architecture

```
main thread (Slint event loop, UI only)          tokio runtime (4 workers)
──────────────────────────────────────           ─────────────────────────
AppWindow + globals  <── invoke_from_event_loop ──  Engine futures (ctx.call)
  callbacks ──────────── ctx.call(|e| async {..}) ─>  engine.subscribe() pump
                                                     (batched EngineEvents)
```

- **One process.** `main.rs`: log file (`<data>/logs/launcher.log`, panics
  included; `logging.rs`) → single-instance lock → renderer choice → tokio
  runtime → `Engine::start` (onboarding + cluster hooks, blocking before any
  window exists) → `AppWindow::new()` → every module's `init` →
  `router::navigate("#/")` → `ui.show()` (the window appears only once the
  engine is ready) → `slint::run_event_loop()` → on close: `engine.shutdown()`
  (games keep running). A window that fails on OpenGL restarts the launcher on
  the software renderer; a UI-thread panic ends in an error dialog naming the
  log file.
  `--mcp` is dispatched before all of that: the binary becomes the stdio MCP
  server (`espectral_engine::mcp::run_stdio()`) and exits with its status.
- **Single instance** (`single_instance.rs`): exclusive lock on
  `<data>/ui.lock` + loopback port in `<data>/ui.port` (`<port> <token> <pid>`,
  served from the moment the lock is taken). A second launch sends its argv
  and waits for the answer: `ok` (the first one queued it while starting, or
  raised its window and took the `.mrpack` args on the UI thread) → exit;
  `closing` → keep trying the lock and become the first instance; no answer
  for 10 s → "already open but not responding, close it?" dialog (kills that
  pid after checking it is the launcher exe). `.mrpack` args are imported
  with `engine.import_mrpack` (outcome toasted from the `import-done` event),
  the first launch's own ones too.
- **The UI thread never awaits.** Engine calls run on the runtime through
  [`Ctx`](src/bridge.rs); results come back with `slint::invoke_from_event_loop`
  through a `Weak<AppWindow>`.
- **Idle = 0% CPU.** No polling unless needed (launches poll 15 s only while a
  game runs; servers poll 60 s only while Home/Servers is shown), no
  `animation-tick()` on anything visible while idle, static backdrop.
- Renderers: `winit` backend with **femtovg** (OpenGL, default) and the
  **software** renderer as fallback and for headless screenshots
  (`renderer.rs`). The launcher switches a PC to software by itself (per
  launcher version, pref `renderer_fallback`) when the event loop fails on
  OpenGL, or when an OpenGL start never drew its first frame (pref
  `renderer_gl_pending` left behind by a driver crash/hang).
  `SLINT_BACKEND=winit-software` forces it by hand. Measured (release build,
  Xvfb, idle Home): software
  ≈ 48 MB PSS, femtovg on Mesa llvmpipe ≈ 162 MB PSS (most of it is llvmpipe;
  real GPU drivers are lighter). Idle CPU 0% in both (table in section 12).

### Source layout

```
native/app/
  build.rs                 compiles ui/app.slint, embeds i18n/*.json, lints missing i18n keys
  i18n/<namespace>.json    {"es": {...}, "en": {...}} string tables (ported 1:1 from lib/i18n/*.ts)
  ui/
    app.slint              AppWindow: z-order, router viewport, overlays, global hotkeys   [foundation]
    theme.slint            Theme global (tokens.css)                                         [foundation]
    i18n.slint             T global (T.tr / tr1 / tr2 / tr3)                                 [foundation]
    state.slint            Page enum, Router, AppState, Shell, Session, Util, PaletteState  [foundation]
    widgets.slint          barrel: import everything shared from here                       [foundation]
    widgets/*.slint        shared widgets (catalogue below)                                  [foundation]
    shell/*.slint          TopChrome, AmbientBackdrop, ToastHost, CommandPalette, LoginGate  [foundation]
    fonts/                 Space Grotesk 400-700 (OFL) + Minecraft Espectral pixel font (regular/bold) [foundation]
    assets/                logo, favicon, Twemoji SVGs (assets/emoji, CC-BY 4.0)             [foundation]
    pages/<group>/         one directory per page group                                      [group agent]
    skin3d/skin-view.slint SkinView + SkinBridge                                             [skin-renderer]
  src/
    main.rs                startup / shutdown                                                [foundation]
    bridge.rs              Ctx (engine calls), on_event (engine events)                      [foundation]
    router.rs toast.rs i18n.rs theme.rs prefs.rs util.rs store.rs session.rs
    palette.rs dialogs.rs single_instance.rs screenshot.rs                                  [foundation]
    pages/mod.rs           group list + init_all                                             [foundation]
    pages/<group>.rs       one Rust module per page group (+ pages/<group>/*.rs children)    [group agent]
    skin.rs                SkinBridge glue                                                   [skin-renderer]
  scripts/screenshot.sh    headless route -> PNG
  scripts/compare.py       reference | slint | diff side by side
  scripts/gen-emoji.py     regenerates widgets/emoji.slint
native/skin3d/             espectral-skin3d: CPU skin rasterizer (trait SkinRenderer)       [skin-renderer]
```

---

## 2. File ownership

Only edit files you own. Shared files already contain every hook a page group
needs; if you truly need a change in a foundation file, **don't make it** —
describe it in your report (it will be applied at merge).

| Group / agent | Svelte sources | Slint files (own) | Rust (own) | i18n files (own) |
|---|---|---|---|---|
| **home-instances** | pages/Home, pages/InstanceHub, components/HorizonRail, HorizonTile, TelemetryCapsule, InstanceWizardModal, MrpackModal, DryRunModal, PipLogDrawer, lib/stores.ts launchLog | `ui/pages/home-instances/*` (`home.slint`→`HomePage`, `instance-hub.slint`→`InstanceHubPage`, `pip-log.slint`→`PipLogDrawer`, `crash-modal.slint`→`CrashDiagnosisModal`, the mod-compat launch modal in `modals.slint`, `api.slint`→`HomeBridge`) | `src/pages/home_instances.rs` (+ `src/pages/home_instances/*.rs`, `launchlog.rs` runs the crash modal) | `home.json`, `instance.json`, `piplog.json`, `personalization.json`, `crash.json` |
| **library-mods** | pages/VersionArmory, ImportCenter, ModCatalog, DonationsPavilion | `ui/pages/library-mods/*` (`versions.slint`→`VersionsPage`, `import.slint`→`ImportPage`, `mods.slint`→`ModsPage`, `packs.slint`→Resource Packs/Shaders panes, `modrinth-pane.slint`→Explore Modrinth tab, `project-detail-modal.slint`→Modrinth project detail, `donations.slint`→`DonationsPage`, `parts.slint`→shared lib pieces (`ModIcons`, `LoadingPanel`, `ErrorBanner`, `EmptyPanel`, `GlassPanel`, `Chip`, `StatePanel`), `api.slint`→`LibraryBridge`) | `src/pages/library_mods.rs` (+ children: `mods.rs`, `browse.rs` (Modrinth tab, installs), `gallery.rs` (detail-modal screenshots), `packs.rs` (Resource Packs/Shaders), `icons.rs`, `drop.rs`, `mrpack.rs`, `versions.rs`, `import.rs`, `donations.rs`, `debug.rs`) | `versions.json`, `import.json`, `mods.json`, `donations.json`, `modrinth.json`, `packs.json` |
| **account-wardrobe** | pages/AccountVault, Wardrobe, components/AccountPopover | `ui/pages/account-wardrobe/*` (`account-vault.slint`→`AccountPage`, the wardrobe sections fold into it, `account-popover.slint`→`AccountPopover`, `api.slint`→`AccountBridge`) | `src/pages/account_wardrobe.rs` (+ children) | `account.json`, `wardrobe.json` |
| **settings-suite-servers** | pages/SettingsDashboard, components/ServerRadarCard | `ui/pages/settings-suite-servers/*` (`settings.slint`→`SettingsPage`, `server-radar-card.slint`→`ServerRadarCard` (**shared**: Home imports it), `api.slint`→`SettingsBridge`) | `src/pages/settings_suite_servers.rs` (+ children) | `settings.json`, `servers.json`, `agent.json` |
| **skin-renderer** | components/SkinViewer (skinview3d/three.js) | `ui/skin3d/*` (`SkinView`, `SkinBridge`, `SkinFrameRequest`) | `src/skin.rs`, the whole `native/skin3d/` crate | `skin.json` (create if needed) |
| foundation | App.svelte, TopChrome, AmbientBackdrop, Toast, CommandPalette, LoginGate, HotkeyLegend, ThemeToggle, LanguageToggle, Btn, GlassCard, Badge, Field, GradientText, ProgressBar, MonogramTile, LoaderBadge, lib/theme, lib/i18n, lib/toast, lib/discord, lib/format, lib/stores (shared lists) | everything else under `ui/` | everything else under `src/` | `common.json`, `palette.json`, `discord.json` |

Rules that keep the merge conflict-free:

- **Keep the exported names and interfaces** that `app.slint` uses
  (`HomePage`, `InstanceHubPage`, `PipLogDrawer`, `VersionsPage`, `ImportPage`,
  `ModsPage`, `DonationsPage`, `AccountPage`, `AccountPopover`,
  `SettingsPage`, `ServerRadarCard`, the four
  `*Bridge` globals, `SkinView`/`SkinBridge`/`SkinFrameRequest`). Pages take no
  properties from the shell: they read globals.
- **One bridge global per group** (`HomeBridge`, `LibraryBridge`,
  `AccountBridge`, `SettingsBridge`, `SkinBridge`) is the only page global Rust
  can reach (`app.slint` re-exports exactly these; Slint allows one `export *`
  per file, so no other globals can be added without touching `app.slint`).
  Put every property/callback/model your group needs on it. Structs/enums
  declared in your files and used by your bridge become Rust types
  automatically (`crate::YourStruct`).
- New Slint files: anywhere under your directory; import them from your page
  files. New Rust files: child modules of your module
  (`src/pages/home_instances.rs` + `src/pages/home_instances/wizard.rs` with
  `mod wizard;` inside `home_instances.rs`). Never edit `pages/mod.rs`.
- New strings: add keys to **your** i18n JSON files (or create
  `i18n/<your-new-file>.json` — build.rs picks it up). Never rename or delete
  existing keys (other groups may read them). Reading keys from any file is fine.
- Cross-group components: `ServerRadarCard` (owner settings-suite-servers, used
  by Home) — keep its interface stable (add, don't rename). If you need another
  group's component, ask in your report instead of copying it.
- Never edit `native/engine` except tiny additive helpers the UI truly needs
  (list them in your report). Never add dependencies to `native/app/Cargo.toml`
  without listing them in your report (workspace pins in `native/Cargo.toml`).

---

## 3. Bridge pattern (Rust ⇄ Slint ⇄ engine)

`Ctx` (`src/bridge.rs`) is handed to every module's `init(ctx, ui)`; it is a
cheap `Clone` holding the `Engine`, the runtime handle and a `Weak<AppWindow>`.

```rust
// src/pages/library_mods.rs — illustrative: `search`, `hits`, `ModHit`, `to_row` are things the
// agent adds; check the real request/reply types in native/engine/src/api/*.rs.
use espectral_engine::api::content::ModrinthSearchQuery;
use slint::{ComponentHandle, ModelRc, VecModel};
use crate::bridge::{self, Ctx};
use crate::router::{self, Route};
use crate::{toast, AppWindow, LibraryBridge, ModHit, Page};

pub fn init(ctx: &Ctx, ui: &AppWindow) {
    // 1) Slint -> Rust: a bridge callback starts an engine call
    let c = ctx.clone();
    ui.global::<LibraryBridge>().on_search(move |query| {
        let q = ModrinthSearchQuery { query: query.to_string(), ..Default::default() };
        c.call(
            move |e| async move { e.search_modrinth(q).await },   // runs on tokio
            |ui, reply| {                                          // runs on the UI thread
                let rows: Vec<ModHit> = reply.hits.iter().map(to_row).collect();
                ui.global::<LibraryBridge>().set_hits(ModelRc::new(VecModel::from(rows)));
            },
        ); // errors -> "AVISO DEL SISTEMA" toast automatically
    });

    // 2) route enter -> load data
    let c = ctx.clone();
    router::on_route(move |ui, route: &Route| {
        if route.page == Page::Mods { ui.global::<LibraryBridge>().invoke_search("".into()); }
    });

    // 3) engine events (UI thread, batched)
    bridge::on_event(|ui, ev| {
        if let espectral_engine::events::EngineEvent::ModProgress(p) = ev { /* update progress */ }
    });
}
```

| API | Use |
|---|---|
| `ctx.call(f, then)` | `f: FnOnce(Engine) -> Future<Output = EngineResult<T>>` on tokio; `then(&AppWindow, T)` on the UI thread; `Err` → error toast. |
| `ctx.call_with(f, then, on_err)` | same, custom `on_err(&AppWindow, EngineError)` (e.g. inline form error). |
| `ctx.fire(f)` | fire-and-forget engine call (errors toasted). |
| `ctx.spawn(fut)` / `ctx.on_ui(g)` | raw future on tokio / run `g(&AppWindow)` on the UI thread from anywhere. |
| `ctx.engine()` / `ctx.rt()` / `ctx.weak()` / `ctx.ui()` | engine clone source, runtime handle, `Send` weak handle, upgrade (UI thread). |
| `bridge::on_event(|ui, ev| ..)` | every `EngineEvent` on the UI thread. |
| `router::on_route(|ui, route| ..)`, `router::navigate(ui, "#/..")`, `router::instance_route(name)`, `route.param("tab")` | routing. |
| `toast::ok/err/info(text)`, `toast::push(kind, text, href)`, `toast::engine_error(&e)` | toasts from any thread (6 s, clickable href). |
| `i18n::t(key)`, `i18n::tf(key, &[("name", v)])`, `i18n::lang()` | strings in Rust (toasts, generated rows). |
| `prefs::get/get_str/get_bool/set` | localStorage replacement (`<data>/ui-prefs.json`); prefix keys with your group (`"mods.view"`). |
| `dialogs::pick_file/pick_files/pick_folder/save_file` (async, inside a `ctx.call`/`spawn` future) | native dialogs (rfd: xdg-portal on Linux, Win32 on Windows). |
| `util::decode_png(&bytes)` (any thread) → `util::to_image(buf)` (UI thread) | engine PNG bytes → `slint::Image` (decode off the UI thread). |
| `util::open_url(url)`, `util::copy_text(text)`, `util::format_bytes`, `util::mem_label`, `util::time_ago` | misc (also exposed to Slint via `Util.*` / `AppState.open-url` / `AppState.copy-text`). |
| `store::instances()/accounts()/launches()/servers()` + `store::refresh_*()` | shared cached lists (also in `AppState.*`). Call `refresh_*` after you mutate them (create/delete/rename instance, add account, …). |
| `session::session_id()` / `session::user()` | Discord session (Wardrobe / identity calls). |
| `skin::invalidate(ui, "account:<name>")` | after an account skin changes. |

Slint models: build `Vec<Row>` and set `ModelRc::new(VecModel::from(rows))`;
for incremental updates keep an `Rc<VecModel<Row>>` in a `thread_local!` (see
`toast.rs`). Never hold a strong `AppWindow` in a closure — use `ctx`/`ctx.weak()`.
Slint `float` is f32: pass epoch-ms timestamps as strings.

### Skin renderer contract

`native/skin3d` exposes `trait SkinRenderer { fn render(&self, &SkinTexture, &RenderParams) -> RgbaFrame }`,
`SkinTexture::new(w, h, rgba)` (64×64 / 64×32 / HD multiples), `RenderParams { width, height, yaw_deg,
pitch_deg, zoom, model: ArmModel, time_s, overlay }` and `default_renderer()` (today `StubRenderer`, a flat
front view). `src/skin.rs` wires `SkinBridge.render(SkinFrameRequest) -> image` (a pure callback evaluated
from the `SkinView` binding: re-renders only when yaw/pitch/size/texture change) and loads textures by
source (`account:<username>`, `library:<id>`, `file:<path>`). Pages use
`SkinView { source: "account:" + name; variant: "slim"; }`.

---

## 4. Slint globals (shared state)

| Global | What |
|---|---|
| `Theme` | design tokens (below). `Theme.dark`, `Theme.accent`, … |
| `T` | strings: `T.tr("key")`, `T.tr1("key", "name", v)`, `T.tr2(..)`, `T.tr3(..)`; `T.lang` ("es"/"en"). |
| `Router` | `page` (enum `Page`), `instance-name`, `query`, `route`, `serial`; `navigate("#/...")`, `back()`. |
| `AppState` | `instances: [InstanceRow]`, `instances-loaded`, `accounts: [AccountRow]`, `active-account` + `has-active-account`, `launches: [LaunchRow]`, `running-count`, `servers: [ServerRow]`, `servers-loaded`, `version`, `data-dir`; `refresh-*()`, `open-url(url)`, `open-folder(path)`, `copy-text(text)`. |
| `Shell` | `set-theme-mode("dark"\|"light"\|"system")`, `toggle-theme()`, `set-accent(id)`, `set-language("es"\|"en")`, `toggle-language()`, `palette-open`, `account-popover-open`, `pip-log-open`, `pip-log-pinned`, `wizard-open` (the instance wizard, mounted globally; Home, Instances and the palette set it), `modal-depth` (Modal maintains it; hotkeys off while > 0), `hide-hotkeys`, `show-advanced` / `set-show-advanced(bool)` (the "Opciones avanzadas" disclosures; persisted), `toast(kind, text, href)`, update CTA (`update-status`: `"idle" / "available" / "downloading" / "ready" / "error"`, `update-version`, `update-progress`, `install-update()`) driven by `src/updater.rs`. |
| `Session` | Discord: `status`, `show-gate`, `user-name`, `user-avatar`, `authenticating`, `auth-error`; `login-discord()`, `continue-anonymous()`, `logout()`. |
| `Util` | pure helpers: `initial(name)`, `name-hue(name, hue)`, `format-bytes(n)`, `relative-time(ms-or-iso, lang)`, `format-memory(mb)`. |
| `PaletteState` | command palette model (shell only). |

Global hotkeys (in `app.slint`): Ctrl/⌘+K toggles the palette; `/` opens it;
Space toggles the PiP log; Esc closes palette → account popover → PiP (unless
pinned). Text inputs consume `/` and Space first (same as the Svelte
`isEditable` check). Disabled while a `Modal` is open or the gate is shown.

---

## 5. Theme — the "glass" look (1.9)

The native UI uses the in-game Espectral menu's palette (branding-mod
`MenuSkin` / `SuiteTheme`), not its layout: a dark charcoal window with three
large soft glows (accent, the logo's purple, a cool blue; `shell/backdrop.slint`,
they glide to a new arrangement when the page changes and are otherwise still),
translucent dark glass panels with 1px hairlines and a faint top sheen, warm
white text and the accent used sparingly. Rules of thumb:

- solid accent fill only for THE primary action of a view (Jugar, Crear…);
- selected / active = accent tint (`accent.with-alpha(0.14)`) + `edge-strong` hairline;
- every border is `Theme.hairline` (1px); no hard offset shadows; soft `shadow-*`
  only on floating layers (menus, popovers, modals, the play bar);
- floating layers over content are opaque (`surface-solid`): Slint has no
  backdrop blur, so translucency only works over the (already soft) glows;
- the pixel font is reserved for the "Espectral" wordmark.

`ui/theme.slint` holds the tokens for both schemes and all five accents
(`oro carmesi amatista esmeralda glaciar`, the same palettes as the mod).
`Theme.mode` / `Theme.accent-id` are set from the engine config (`theme`,
`accent`) by `src/theme.rs`; `"system"` follows `Palette.color-scheme`.

| Token | Use |
|---|---|
| `bg` | window charcoal under the glows |
| `surface`, `card-bg`, `surface-up` | glass panel fill / hover glass |
| `card-bg-soft`, `rail-bg` | wells and rows inside a panel |
| `surface-solid`, `surface-up-solid`, `log-bg`, `backdrop` | opaque floating layers; modal scrim |
| `sheen`, `glass-border`, `border-solid`, `hairline` | top highlight; 1px panel edge; 1px separators |
| `text`, `muted-strong`, `muted`, `title-color` | warm white text; headings are warm white, not accent |
| `accent`, `accent-bright`, `edge`, `edge-strong`/`hover-border`, `btn-fill(-hover)`, `btn-ink` | the accent; hover text; 25%/45% hairlines; the primary button |
| `shadow-{sm,md,lg}-*` | soft dark shadows for floating layers |
| `font-body` (Space Grotesk; headings too), `font-pixel` (wordmark only), `font-mono` | |
| `accent-green/alt/red/cyan/purple`, `loader-*`, `mode-*`, `radius-*`, `space-*`, `text-*`, `dur-*`, `hsla()` | as named |

Fonts: `ui/fonts/` holds static instances (400/500/600/700) of Space Grotesk
(OFL) and the "Minecraft Espectral" pixel font (built by
`espectral-web/scripts/build-mc-font.py` from the game's font atlases, the
same font the website serves; see `LICENSE-MinecraftEspectral.txt`).

## 6. Widget catalogue (`import { ... } from "../../widgets.slint";`)

| Widget | Svelte | Notes |
|---|---|---|
| `Btn { text; variant: BtnVariant.primary\|secondary\|ghost\|danger; size: BtnSize.sm\|md\|lg; icon: Icons.x; loading; enabled; block; clicked }` | Btn.svelte | primary = solid accent (the one main action); secondary = dark glass with an accent hairline; ghost = bare label; danger = red hairline; spinner |
| `EmojiBtn { text; emoji; variant: BtnVariant.*; size: BtnSize.*; enabled; loading; clicked }` | Btn.svelte (`emoji` prop) | same paint as `Btn`; leading color emoji (`Emoji`, Twemoji) instead of an SVG icon |
| `IconBtn { icon; size; icon-size; flat; active; clicked }` | square icon buttons | |
| `Spinner { size; tint }` | btn spinner | animates only while visible |
| `GlassCard { title; subtitle; footer; elevation: Elevation.*; interactive; clickable; featured; flat; padding-size; spacing; clicked; <children> }`, `GlassSheen { radius }` | GlassCard.svelte | translucent glass panel: 1px hairline, top sheen (`GlassSheen`), accent edge on hover/featured; soft shadow only for `Elevation.lg`/featured; `flat` = opaque |
| `Surface { soft; solid; hoverable; clicked }` | .card-surface / .glass-row | plain container |
| `Badge { text; variant: BadgeVariant.neutral\|ok\|warn\|err\|info\|accent\|gold\|purple\|vanilla\|fabric\|neoforge\|quilt; dot; pixel; large }` | Badge.svelte | |
| `Kbd { text; accent }` | `<kbd class=font-pixel>` | |
| `GradientText { text; gradient: GradientKind.*; font-size… }` | GradientText.svelte | solid colour per preset |
| `McTitle { text; font-size; ink; horizontal-alignment; wrap }` | site `h1`–`h4` | semibold heading in the body font (warm white); never widens its parent when wrapping |
| `Wordmark { font-size }`, `ClientLockup { font-size; version }` | site logo | per-letter gold pixel "Espectral" wordmark — the only pixel-font text |
| `Sticker { text; fill; font-size }` | site "CLIENT" tag | small tinted hairline tag |
| `AdvancedToggle { summary; changed; title }` | — | the "Opciones avanzadas" bar; pair with `if Shell.show-advanced: …` |
| `LoaderBadge { loader; version; size: LoaderBadgeSize.*; show-icon }` | LoaderBadge.svelte | |
| `MonogramTile { name; hue; avatar; has-avatar; size; shape: MonogramShape.tile\|rounded\|circle }` | MonogramTile.svelte | |
| `TextField { text <=>; placeholder; icon; password; mono; clearable; has-error; field-height; pad-y; fill; focus-color; edited(t); accepted(t) }`, `TextArea` | `<input>` / `<textarea>` | focus ring; sets only `height` (never `min-height`), so it works inside layouts. `HiTextField` / `SsTextField` are thin aliases |
| `SelectableText { text; color; font-family; font-size; font-weight; wrap; horizontal-alignment; vertical-alignment; letter-spacing }` | selectable `<p>` | read-only `TextInput` styled as `Text`. Policy: only for copyable content (logs, versions, paths, Modrinth descriptions/body, report excerpts); plain `Text` for static chrome (labels, buttons, headings, badges). No `overflow` (cannot elide) and it takes the press, so never above a `TouchArea` that must receive the click |
| `Field { label; hint; error; required; <TextField/Select/...> }` | Field.svelte | |
| `ProgressBar { value; max; indeterminate; tone: ProgressTone.*; thickness: ProgressThickness.*; label; show-value }` | ProgressBar.svelte | no shimmer |
| `Toggle { checked <=>; toggled(v) }`, `Checkbox { checked; text }` | switches | |
| `Slider { value <=>; minimum; maximum; step; changed(v); released(v) }` | range inputs | commit on `released` |
| `Segmented { items: ["ES","EN"]; current; selected(i) }` | segmented pills | |
| `Tabs { items: [{ label, icon, badge }]; current <=>; selected(i) }` | InstanceHub tabs | ←/→ keys |
| `Select { options: [{ value, label }]; current-index <=>; placeholder; selected(i, value) }` | `<select>` | PopupWindow; options are anonymous struct literals |
| `Modal { open <=>; title; subtitle; dialog-width; dialog-height; dismissable; close; <body>; ModalFooter { Btn… } }`, `ConfirmDialog { message; confirm-text; cancel-text; danger; confirmed; close }` | modal scaffold | put it as the LAST child of the page root Rectangle (fills it), outside layouts |
| `ScrollArea { <one layout> }` | scrolling + themed scrollbar | use instead of std ScrollView |
| `PageScaffold { content-padding; max-content-width; spacing; <children> }`, `PageHeader { title; subtitle; gradient; divider; <eyebrow children> }`, `SectionTitle { title; description }` | page wrappers | PageScaffold = ScrollArea + centered column |
| `Icon { data: Icons.search; size; color; filled; weight }`, `Icons.*` | inline feather SVGs | add page-only icons locally (`Icon { data: "M ..." }`) |
| `Emoji { char: "🎨"; size; fallback-color }` | color emoji | Twemoji images; text-presentation symbols (⛏ ▶ ⚙ ⚛ …) render as text |
| `PlaceholderPage` | — | delete its use when you port your page |

Shell components (foundation): `TopChrome` (logo + wordmark, the floating
navigation pill, account chip; `ThemeToggle`), `AmbientBackdrop` (the glows),
`ToastHost`, `CommandPalette` (Ctrl+K or "/"), `LoginGate`, `HotkeyLegend`.

---

## 7. i18n workflow

- Tables: `native/app/i18n/<ns>.json`, `{"es": {...}, "en": {...}}`, keys
  identical to `src/ui/src/lib/i18n/<ns>.ts` (converted 1:1; `home.capsuleTiming`
  from `i18n-dicts.ts` lives in `home.json`). All files are merged into one
  table per language; build.rs embeds every `*.json` it finds.
- Slint: `T.tr("mods.title")`, `T.tr1("command.noResults", "query", q)`,
  `T.tr2(...)`, `T.tr3(...)` (`{name}` placeholders). CSS `text-transform:
  uppercase` → `T.tr("x").to-uppercase()`.
- Rust: `i18n::t("key")`, `i18n::tf("key", &[("name", v)])`.
- Runtime switch: `Shell.set-language("en")` / `Shell.toggle-language()`;
  every binding re-evaluates because `tr*()` reads `T.lang`. Persisted in prefs
  (`"lang"`, default `es`). Missing key → other language → the key itself.
- `cargo build` warns `i18n: key "x" used in ui/... is in no native/app/i18n/*.json`
  for every literal `T.tr*("x"` that is missing. Keep it warning-free.
- Text that the Svelte UI hard-coded (e.g. the toast tags "LOGRO
  DESBLOQUEADO") stays hard-coded for parity.

---

## 8. Svelte → Slint mapping

| Svelte | Slint / Rust | Owner |
|---|---|---|
| App.svelte | ui/app.slint, src/main.rs, src/router.rs | foundation |
| components/TopChrome, ThemeToggle, LanguageToggle | ui/shell/top-chrome.slint | foundation |
| components/AmbientBackdrop | ui/shell/backdrop.slint (flat background) | foundation |
| components/Toast + lib/toast | ui/shell/toast-host.slint + src/toast.rs | foundation |
| components/CommandPalette | ui/shell/command-palette.slint + src/palette.rs | foundation |
| components/LoginGate + lib/discord.svelte.ts | ui/shell/login-gate.slint + src/session.rs | foundation |
| components/HotkeyLegend | ui/shell/login-gate.slint (`HotkeyLegend`) | foundation |
| components/Btn, GlassCard, Badge, Field, GradientText, ProgressBar, MonogramTile, LoaderBadge | ui/widgets/*.slint | foundation |
| lib/theme.svelte.ts, styles/tokens.css, base.css | ui/theme.slint + src/theme.rs | foundation |
| lib/i18n*.ts | i18n/*.json + ui/i18n.slint + src/i18n.rs | foundation |
| lib/stores.ts (instances, launches, servers, health, import-done toasts, close-on-game-start) | src/store.rs + AppState | foundation |
| lib/stores.ts launchLog (ring buffers) | src/pages/home_instances.rs | home-instances |
| lib/format.ts | src/util.rs + Util | foundation |
| lib/dialog.ts, /api/pick-* | src/dialogs.rs | foundation |
| lib/updater.svelte.ts | `src/updater.rs` + `espectral_engine::updater` | done |
| pages/Home + HorizonRail/Tile, TelemetryCapsule, InstanceWizardModal, MrpackModal, DryRunModal | ui/pages/home-instances/home.slint (+ your files) | home-instances |
| pages/InstanceHub | ui/pages/home-instances/instance-hub.slint | home-instances |
| components/PipLogDrawer | ui/pages/home-instances/pip-log.slint | home-instances |
| pages/VersionArmory / ImportCenter / ModCatalog / DonationsPavilion | ui/pages/library-mods/{versions,import,mods,donations}.slint | library-mods |
| pages/AccountVault + Wardrobe sections, components/AccountPopover | ui/pages/account-wardrobe/{account-vault,identity,account-popover}.slint | account-wardrobe |
| pages/SettingsDashboard, components/ServerRadarCard | ui/pages/settings-suite-servers/{settings,server-radar-card}.slint | settings-suite-servers |
| components/SkinViewer (skinview3d, three.js) | ui/skin3d/skin-view.slint + src/skin.rs + native/skin3d | skin-renderer |

Routes (`router::parse`): `#/` → `Page.home`; `#/instances` → `Page.instances`
(every instance); `#/instances/:name` → `Page.instance` (+ `Router.instance-name`);
`#/library/versions` (legacy `#/versions`); `#/library/import`
(legacy `#/import`); `#/mods` (tabs: 0 Mods, 1 Resource Packs, 2 Shaders,
3 Presets, 4 Explore Modrinth; `#/mods?tab=packs|shaders|presets|modrinth`);
`#/account` (`?section=skin`, the wardrobe lives
here too); `#/settings`; `#/donaciones` → `Page.donations`; anything else →
home. The retired `#/servers`, `#/client` and `#/armario` hashes are unknown
routes: they land on home, like any other unknown hash. The page component is instantiated on entry and
destroyed on exit (like the Svelte `{#key}` remount): keep cross-visit state
in your bridge global or in Rust.

---

## 9. Screenshots

```sh
native/app/scripts/screenshot.sh '#/settings' /tmp/settings.png
native/app/scripts/screenshot.sh '#/' /tmp/home-light.png --theme light --lang en
native/app/scripts/screenshot.sh '#/' /tmp/palette.png --open palette --query set
native/app/scripts/screenshot.sh '#/instances/Espectral%20default?tab=mods' /tmp/i.png --wait 3000
native/app/scripts/screenshot.sh '#/settings' /tmp/s.png --scroll 600
python3 native/app/scripts/compare.py <ref.png> /tmp/settings.png /tmp/cmp.png
```

`--screenshot <route> <out.png>` is a flag of the binary itself: it installs a
headless Slint platform (software renderer, no X/GPU), boots the engine and
the whole shell against a fixture data dir (`native/target/shot-data`), pumps
events for `--wait` ms (default 1500) so engine calls land, renders one frame
and exits. Options: `--theme`, `--lang`, `--accent`, `--size WxH`, `--wait`,
`--scroll <px>` (PageScaffold pages), `--open palette|wizard|account|pip|gate`,
`--query`, and scripted input run in order after half of `--wait`: `--click X,Y`
(logical px), `--type <text>`, `--key Escape|Return|Tab|Space|Down|Up|Backspace`,
`--nav <route>` (all repeatable). The headless pump draws a frame every 40 ms
(like the real loop), so `if` subtrees, Timers and `--scroll` are alive before
the PNG is taken; `--scroll` is clamped to the content (`--scroll 99999` = bottom).
The software renderer paints gradient text with its first stop (the
on-screen femtovg renderer paints the gradient). To see the real windowed app:
`xvfb-run -a -s "-screen 0 1280x800x24" native/target/debug/espectral-client`
(set `SLINT_BACKEND=winit-software` if GL fails).

### Reference screenshots (Svelte UI, 1280×800, dark, Spanish unless noted)

Directory: `/tmp/claude-0/-home-user-espectral-client/7274c801-56fc-5f5d-bdc2-b83be817cf63/scratchpad/ref-screens/` (see `INDEX.md` there).
Captured from the JS engine with a fresh data dir (instance "Espectral default", no accounts, anonymous).

| File | State |
|---|---|
| login-gate.png | Discord gate (first run) |
| home.png, home-scrolled.png, home-bottom.png, home-light.png | `#/` |
| instance.png, instance-tab-{config,servers,mods,danger}.png, instance-light.png | `#/instances/Espectral%20default` |
| library-versions.png, library-import.png | `#/library/versions`, `#/library/import` |
| mods.png | `#/mods` |
| account.png, account-scrolled.png | `#/account` |
| settings.png, settings-scrolled.png, settings-en.png, settings-light.png | `#/settings` |
| donaciones.png | `#/donaciones` |
| command-palette.png, command-palette-query.png | palette open / query "set" |
| instance-wizard.png | Home → "+ Nueva Instancia" |
| account-popover.png | TopChrome account popover |
| piplog-open.png | Home after Space |

Note: the reference screenshots show the old Svelte glass look; the native UI
now uses the poster look (§5), so compare layout and content, not colours.

---

## 10. Slint gotchas (1.18) — read before porting

- **Wrapped text in layouts**: a `VerticalLayout` with `padding-left/right`
  (or wrapped `Text` inside an `if … : VerticalLayout` directly in a column)
  can measure word-wrapped `Text` too wide, so the parent gets too short and
  the last lines are clipped. Put horizontal padding on an enclosing
  `HorizontalLayout`, and wrap conditional blocks in their own component
  (see `LoginGate`/`GatePrompt`, `GlassCard`, `PageScaffold`). Size dialogs by
  centering them with layouts, not with `height: x.preferred-height`.
- CSS `line-height: L` → `line-height-factor: L / 1.28` for Space Grotesk.
- `color.alpha` is 0–255; use your own float for alpha math.
- No `for` loops in functions; compute lists in Rust.
- A component's root cannot reference `parent`; `Modal` fills its parent
  when placed outside layouts.
- Name clashes: inside a component with a `text` property, write
  `mouse-cursor: MouseCursor.text`.
- `Flickable` uses `content-x/y/width/height` (viewport-* is deprecated).
- One drop shadow per element; nest rectangles for a second one.
- Gradient text needs femtovg (software renderer = first stop).
- Never leave `animation-tick()` running on a visible idle element; use
  `if` to remove animated items (spinners, indeterminate bars) when not busy.
- Images from Rust: decode off-thread (`util::decode_png`), build the
  `Image` on the UI thread. Skins/avatars are pixel art: `image-rendering: pixelated`.
- Only literal `T.tr*("key"` are lint-checked; dynamic keys are fine but
  unchecked.
- **Struct values are anonymous literals**: `Select { options: [{ value: "a", label: "A" }] }` — never `SelectOption { … }`; the struct name is a type, not a constructor.
- `SelectableText` inherits `TextInput`, which has no `overflow` property: it cannot elide. Keep plain `Text` for labels that must elide or clip.
- **Responsive layouts: one instance, not two branches.** `if narrow: …` / `if !narrow: …` compiles a full component tree per branch (both live at once). Bind the geometry on one instance and switch it on the `narrow` property — `settings.slint` `AboutCard` (`x` / `y` / `width` and the card height all read `root.narrow`) and the data-folder chips above it.

---

## 11. Build / verify (every agent, before handing back)

```sh
cd native
cargo build -p espectral-client
cargo test -p espectral-client
cargo check -p espectral-client --target x86_64-pc-windows-gnu
cargo clippy -p espectral-client --all-targets   # warning-free
cargo fmt --all
native/app/scripts/screenshot.sh '<your routes>' ... # compare with the references
```

**Crates:** the `.slint` sources live here (`native/app/ui`) but are compiled
by their own crate, `native/ui` (`espectral-client-ui`, ~60 MB of generated
Rust); the app crate re-exports it (`pub use espectral_client_ui::*`). An edit
to the app's Rust code no longer recompiles the UI: measured 10 s and 1.8 GB
for a rebuild after touching `src/util.rs` (was ~5 min and ~12 GB). The test
harness and the bin share the one compiled UI crate.

**Live preview (UI work):** `scripts/screenshot.sh` builds a dev binary with
`SLINT_LIVE_PREVIEW=1 --features live-preview` into `native/target/live`. It
loads `ui/*.slint` when it starts (Slint interpreter), so a `.slint` edit costs
~10 s of cargo plus ~6 s per screenshot, with no recompile of the generated
module; the first live build peaks at ~3 GB. Its screenshots are
pixel-identical to the compiled build's. Adding or renaming a property or
callback that Rust uses still needs a normal rebuild, and release builds
refuse the feature (`native/ui/build.rs`). `ESPECTRAL_SHOT_COMPILED=1` shoots
the compiled build instead.

**Memory:** one rustc on the UI crate peaks at ~11.5-13.9 GB (debug, test and
release alike; ~7.7 GB for `check`/`clippy`). On a 16 GB machine never run two
cargo commands at once, and keep `-j 2` for builds that compile the UI crate
(its codegen threads draw from the same job slots). Release builds: `-j 2`,
~15-20 min. With Minecraft open on Windows the 13.9 GB peak exhausts the
host's commit limit and Windows stops the WSL VM: build release with the game
closed. **Incremental:** off for the UI crate
(`[profile.dev.package.espectral-client-ui] incremental = false`; its caches
ran to tens of GB and it only rebuilds on a `.slint` change), on for the small
crates.

Disk is limited: `[profile.dev.package."*"]` already disables debug info for
dependencies (the UI and app crates build with `debug = "line-tables-only"`).
Use your worktree's own `native/target`, and delete it when you are done and
committed. Do not push.

Engine changes made by the foundation: **none** (the UI uses only the public
`Engine` facade, `engine.config()`, `engine.paths()`, `engine.http()`,
`paths::resolve_data_dir()`).

---

## 12. Integration status

All five page groups are merged into `rust-native` (2026-10-31). Cross-group
wiring done at integration: Home "Importar modpack" opens the shared
library-mods `MrpackModal` (Home's duplicate was removed); VersionArmory
"Crear instancia" opens the wizard with `Shell.wizard-version` preselected
(filter switched to "all" so snapshots stay visible); the Skin Atelier gallery
cards use `SkinView`'s own `clicked` (drag + spin button work); `TextField`
works inside layouts and replaces the `HiTextField` / `SsTextField` copies.

Every reference screenshot was re-taken with the Slint app
(`ref-screens/final/<name>.png`, side-by-side contact sheet
`ref-screens/final/contact-sheet.png`: Svelte left, Slint right).

| Page / component | Status | Notes |
|---|---|---|
| Shell (TopChrome, theme / language toggles, palette, toasts, login gate, hotkeys) | done | poster look: purple header bar with gold rule, wordmark, text nav; flat background; dark/light and ES/EN switch live |
| Home (`#/`) | done | photo hero (pixel greeting, quick actions, MOTD strip), rail, telemetry capsule, server strip; wizard, dry-run, shared .mrpack modal |
| Instance wizard | done | 3 steps; preselected version from VersionArmory |
| InstanceHub (`#/instances/:name`) | done | config / servers / mods / danger tabs + live console; capture tools toggle (Replay Mod install/remove) |
| PiP launch log | done | Space / palette / hub button |
| VersionArmory (`#/library/versions`) | done | manifest unreachable in the sandbox (error card matches the reference); rows checked with `ESPECTRAL_SHOT_LIB=versions-fake` |
| ImportCenter (`#/library/import`) | done | dropzone border fainter than the reference (dashed CSS border approximated) |
| MrpackModal | done | hosted by Home and ImportCenter; OS drag & drop |
| ModCatalog (`#/mods`) | done | the filter placeholder stays hard-coded Spanish, as in the Svelte page |
| AccountVault + Skin Atelier + wardrobe (`#/account`) | done | 3D viewer, gallery preview / try / apply / rename / delete; identity verification and cosmetics folded in from the old `#/armario` page; MSA device-code flow not exercised (no network); Discord-linked states not exercised visually (no Discord session in the sandbox) |
| ServerRadarCard (Home server strip) | done | compact card only; the Server Radar page and the Cliente / Suite pages were removed (in-game configuration through the Espectral Suite) |
| AccountPopover | done | |
| Settings (`#/settings`) | done | incl. "Acceso para agentes" (MCP): toggle, safety rules, JSON/TOML config copy, token path + rotate, allowlist editor. Deliberate difference: the blocked-host warning also catches `host:port` forms (`play.uhce.top:25565`), which the Svelte check let through |
| Donations (`#/donaciones`) | done | |
| Updater (lib/updater) | done | `native/app/src/updater.rs` + `espectral_engine::updater`: boot check 30 s after start, re-check every 6 h, minisign-verified download, Windows NSIS `/P /R /UPDATE`, Linux `pkexec dpkg -i` + relaunch |

Known cosmetic differences: the Home tile's ghost version watermark is a bit
more visible than in the reference; the wizard dialog sits a little higher.
Screenshot tool caveat: `--scroll` is applied after `--click`/`--type`
actions (clicks hit the unscrolled page; use `--size 1280x1400` instead).

### Release binary

| | size | notes |
|---|---|---|
| before | 129.9 MB | `strip = "symbols"` was silently dropped: a `[profile.release.package.espectral-client]` override of the root package loses the profile-wide `strip` (42 MB `.strtab`/`.symtab` left in) |
| strip fixed | 88.0 MB | `.text` 64.5 MB, of which ~37 MB is Slint-generated code (app crate + its `core` generic instances) |
| after (`opt-level = "s"` for the app crate) | **61.1 MB** | `.text` 39.0 MB, `.rodata` 6.3 MB (fonts + images + emoji SVGs embedded: < 1 MB), `.eh_frame` + `.gcc_except_table` 9 MB |

`panic = "abort"` is not an option: the engine relies on unwinding
(`catch_unwind` in launch / aot / game_launch, tokio `JoinError` on task
panics). The skin rasterizer is its own crate and stays at opt-level 3.
Release build: 14m52s with `-j 2`, peak 12.9 GB used system-wide.

### Measurements (release build, Xvfb 1280x800, fresh temp `ESPECTRAL_DATA_DIR`)

PSS from `/proc/<pid>/smaps_rollup`; CPU = utime+stime ticks (1 tick = 10 ms)
over 20 s. (a) idle on Home 20 s after start; (b) on Home after visiting every
route (instance hub + 3 tabs, versions, import, mods, account, settings,
donations) via `ESPECTRAL_TOUR`; (c) Skin Atelier (`#/account`) with the 3D
viewer spinning (a gallery skin previewed in the stage). Offline account
"Steve", anonymous Discord session. Scripts: scratchpad `mem.sh` / `mem-inner.sh`.

| Backend | (a) Home idle | (b) after all pages | (c) skin viewer spinning | idle CPU (a) / (b) | CPU (c) |
|---|---|---|---|---|---|
| `winit-software` | 47.8 MB PSS (52 MB RSS) | 60.5 MB (65 MB) | 50.9 MB (56 MB) | 0 / 1 ticks | 222 ticks ≈ 11% of a core |
| femtovg (Mesa llvmpipe) | 162.2 MB (186 MB) | 219.6 MB (244 MB) | 188.6 MB (213 MB) | 0 / 0 ticks | 4460 ticks ≈ 2.2 cores (llvmpipe rasterizes on the CPU) |

The femtovg numbers are dominated by llvmpipe (its JIT, threads and buffers);
on a real GPU driver femtovg is expected to sit much closer to the software
numbers. Idle is 0% CPU on both renderers; the viewer stops costing CPU when
paused or scrolled off-screen.
