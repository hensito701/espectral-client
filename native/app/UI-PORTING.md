# Native UI (Slint) — architecture & porting guide

Branch `rust-native`. The Svelte 5 UI (`src/ui/src`) is being replaced by a
native Slint UI in the binary crate **`espectral-client`** (`native/app`). The
binary links **`espectral-engine`** (`native/engine`, see `../PORTING.md`) and
calls `Engine` methods directly: no webview, no Node, no HTTP between UI and
engine. Goal: visual + functional parity with the Svelte UI at a fraction of
the RAM (target: well under 150 MB while open; idle = 0% CPU).

The foundation (this document, the shell, the widgets, the bridge) is done.
Five agents ported the pages in parallel, each in its own worktree, touching
**only their own files** (see [File ownership](#file-ownership)); all five
groups are merged and integrated — see [12. Integration status](#12-integration-status)
for the per-page status, the release size and the memory / CPU measurements.

- Reference implementation: `src/ui/src` (never modify it, nor `src/`, `src-tauri/`, `tests/`).
- Reference screenshots of the Svelte UI: `/tmp/claude-0/-home-user-espectral-client/7274c801-56fc-5f5d-bdc2-b83be817cf63/scratchpad/ref-screens/*.png` (index below).
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

- **One process.** `main.rs`: single-instance lock → tokio runtime →
  `Engine::start` (onboarding + cluster hooks, blocking before any window
  exists) → `AppWindow::new()` → every module's `init` → `router::navigate("#/")`
  → `ui.show()` (the window appears only once the engine is ready) →
  `slint::run_event_loop()` → on close: `engine.shutdown()` (games keep running).
- **Single instance** (`single_instance.rs`): exclusive lock on
  `<data>/ui.lock` + loopback port in `<data>/ui.port`. A second launch sends its
  argv and exits; the first re-shows its window and imports `.mrpack` args
  (`engine.import_mrpack`, outcome toasted from the `import-done` event).
  `.mrpack` args of the first launch are imported too.
- **The UI thread never awaits.** Engine calls run on the runtime through
  [`Ctx`](src/bridge.rs); results come back with `slint::invoke_from_event_loop`
  through a `Weak<AppWindow>`.
- **Idle = 0% CPU.** No polling unless needed (launches poll 15 s only while a
  game runs; servers poll 60 s only while Home/Servers is shown), no
  `animation-tick()` on anything visible while idle, static backdrop.
- Renderers: `winit` backend with **femtovg** (OpenGL, default) and the
  **software** renderer as fallback (`SLINT_BACKEND=winit-software`) and for
  headless screenshots. Measured (release build, Xvfb, idle Home): software
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
    fonts/                 Space Grotesk 400-700 + Press Start 2P (static TTF + OFL licenses) [foundation]
    assets/                logo, favicon, Twemoji SVGs (assets/emoji, CC-BY 4.0)             [foundation]
    pages/<group>/         one directory per page group                                      [group agent]
    skin3d/skin-view.slint SkinView + SkinBridge                                             [skin-renderer]
  src/
    main.rs                startup / shutdown                                                [foundation]
    bridge.rs              Ctx (engine calls), on_event (engine events)                      [foundation]
    router.rs toast.rs i18n.rs theme.rs prefs.rs util.rs store.rs session.rs
    palette.rs backdrop.rs dialogs.rs single_instance.rs screenshot.rs                       [foundation]
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
| **home-instances** | pages/Home, pages/InstanceHub, components/HorizonRail, HorizonTile, TelemetryCapsule, InstanceWizardModal, MrpackModal, DryRunModal, PipLogDrawer, lib/stores.ts launchLog | `ui/pages/home-instances/*` (`home.slint`→`HomePage`, `instance-hub.slint`→`InstanceHubPage`, `pip-log.slint`→`PipLogDrawer`, `api.slint`→`HomeBridge`) | `src/pages/home_instances.rs` (+ `src/pages/home_instances/*.rs`) | `home.json`, `instance.json`, `piplog.json`, `personalization.json` |
| **library-mods** | pages/VersionArmory, ImportCenter, ModCatalog, DonationsPavilion | `ui/pages/library-mods/*` (`versions.slint`→`VersionsPage`, `import.slint`→`ImportPage`, `mods.slint`→`ModsPage`, `donations.slint`→`DonationsPage`, `api.slint`→`LibraryBridge`) | `src/pages/library_mods.rs` (+ children) | `versions.json`, `import.json`, `mods.json`, `donations.json` |
| **account-wardrobe** | pages/AccountVault, Wardrobe, components/AccountPopover | `ui/pages/account-wardrobe/*` (`account-vault.slint`→`AccountPage`, `wardrobe.slint`→`WardrobePage`, `account-popover.slint`→`AccountPopover`, `api.slint`→`AccountBridge`) | `src/pages/account_wardrobe.rs` (+ children) | `account.json`, `wardrobe.json` |
| **settings-suite-servers** | pages/SettingsDashboard, ClientSuite, ServerRadar, components/ServerRadarCard, PingWaveform | `ui/pages/settings-suite-servers/*` (`settings.slint`→`SettingsPage`, `client-suite.slint`→`ClientPage`, `servers.slint`→`ServersPage`, `server-radar-card.slint`→`ServerRadarCard` (**shared**: Home imports it), `api.slint`→`SettingsBridge`) | `src/pages/settings_suite_servers.rs` (+ children) | `settings.json`, `client.json`, `servers.json`, `agent.json` |
| **skin-renderer** | components/SkinViewer (skinview3d/three.js) | `ui/skin3d/*` (`SkinView`, `SkinBridge`, `SkinFrameRequest`) | `src/skin.rs`, the whole `native/skin3d/` crate | `skin.json` (create if needed) |
| foundation | App.svelte, TopChrome, AmbientBackdrop, Toast, CommandPalette, LoginGate, HotkeyLegend, ThemeToggle, LanguageToggle, Btn, GlassCard, Badge, Field, GradientText, ProgressBar, MonogramTile, LoaderBadge, lib/theme, lib/i18n, lib/toast, lib/discord, lib/format, lib/stores (shared lists) | everything else under `ui/` | everything else under `src/` | `common.json`, `palette.json`, `discord.json` |

Rules that keep the merge conflict-free:

- **Keep the exported names and interfaces** that `app.slint` uses
  (`HomePage`, `InstanceHubPage`, `PipLogDrawer`, `VersionsPage`, `ImportPage`,
  `ModsPage`, `DonationsPage`, `AccountPage`, `WardrobePage`, `AccountPopover`,
  `SettingsPage`, `ClientPage`, `ServersPage`, `ServerRadarCard`, the four
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
| `Shell` | `set-theme-mode("dark"\|"light"\|"system")`, `toggle-theme()`, `set-accent(id)`, `set-language("es"\|"en")`, `toggle-language()`, `ambient-hue` (CSS `--ambient-hue`, set it from Home/InstanceHub), `palette-open`, `account-popover-open`, `pip-log-open`, `pip-log-pinned`, `wizard-open` (Home's instance wizard; the palette sets it), `modal-depth` (Modal maintains it; hotkeys off while > 0), `hide-hotkeys`, `toast(kind, text, href)`, update-CTA placeholders (`update-status`, `update-version`, `update-progress`, `install-update()`). |
| `Session` | Discord: `status`, `show-gate`, `user-name`, `user-avatar`, `authenticating`, `auth-error`; `login-discord()`, `continue-anonymous()`, `logout()`. |
| `Util` | pure helpers: `initial(name)`, `name-hue(name, hue)`, `format-bytes(n)`, `relative-time(ms-or-iso, lang)`, `format-memory(mb)`. |
| `PaletteState` | command palette model (shell only). |

Global hotkeys (in `app.slint`): Ctrl/⌘+K toggles the palette; `/` opens it;
Space toggles the PiP log; Esc closes palette → account popover → PiP (unless
pinned). Text inputs consume `/` and Space first (same as the Svelte
`isEditable` check). Disabled while a `Modal` is open or the gate is shown.

---

## 5. Theme (tokens.css → `Theme`)

`ui/theme.slint` ports every token of `styles/tokens.css` for both schemes and
all five accents (`oro carmesi amatista esmeralda glaciar`). `Theme.mode` /
`Theme.accent-id` are set from the engine config (`theme`, `accent`) by
`src/theme.rs`; `"system"` follows `Palette.color-scheme`.

| CSS | Slint |
|---|---|
| `var(--accent)` / `rgba(var(--accent-rgb), .12)` | `Theme.accent` / `Theme.accent.with-alpha(0.12)` |
| `--bg --surface --surface-solid --surface-up(-solid) --card-bg(-soft) --chrome-bg --topbar-bg --rail-bg --log-bg --backdrop` | same names |
| `--text --text-inverse --muted --muted-strong` | same |
| `--accent-ink/-bright/-2/-gold/-alt/-cyan/-purple/-red/-green`, `--loader-*` (brushes), `--mode-*` | same |
| `--border --border-solid --border-focus --glass-border --hover-border` | same |
| `--shadow-sm/md/lg/card` | `drop-shadow-color: Theme.shadow-md-color; drop-shadow-blur: Theme.shadow-md-blur; drop-shadow-offset-y: Theme.shadow-md-y;` (one shadow per element) |
| `--radius-xs..xl/tile/pill`, `--space-1..16` | `Theme.radius-md`, `Theme.space-4`, … |
| `--text-xs..3xl`, `--text-hero` | `Theme.text-sm`, …, `Theme.text-hero(window-width)` |
| `--font-body`, `.font-pixel`, `--font-mono` | `Theme.font-body` (default), `Theme.font-pixel`, `Theme.font-mono` |
| `--dur-fast/med/slow` | `Theme.dur-fast` (120ms), `dur-med`/`dur-slow` capped at 180/200 ms |
| `hsla(h, s%, l%, a)` | `Theme.hsla(h, s, l, a)` (s, l in 0..1) |
| `--topbar-height --content-max --max-width` | same |

Glass `backdrop-filter: blur()` cannot be reproduced: use the semi-opaque fills
(already in the tokens), a border, the 1px top highlight (GlassCard does it)
and a drop shadow. Scrims are a bit denser than the CSS ones instead of blurring.

Fonts: `ui/fonts/` holds static instances (400/500/600/700) of the Space
Grotesk variable font the Svelte UI ships, and Press Start 2P, with their OFL
licenses; `font-weight: 800` renders as 700 (as in the browser).

## 6. Widget catalogue (`import { ... } from "../../widgets.slint";`)

| Widget | Svelte | Notes |
|---|---|---|
| `Btn { text; variant: BtnVariant.primary\|secondary\|ghost\|danger; size: BtnSize.sm\|md\|lg; icon: Icons.x; emoji: "📦"; loading; enabled; block; clicked }` | Btn.svelte | hover lift, press, spinner |
| `IconBtn { icon; size; icon-size; flat; active; clicked }` | square icon buttons | |
| `Spinner { size; tint }` | btn spinner | animates only while visible |
| `GlassCard { title; subtitle; footer; elevation: Elevation.*; interactive; clickable; featured; flat; padding-size; spacing; clicked; <children> }` | GlassCard.svelte | children in a VerticalLayout |
| `Surface { soft; solid; hoverable; clicked }` | .card-surface / .glass-row | plain container |
| `Badge { text; variant: BadgeVariant.neutral\|ok\|warn\|err\|info\|accent\|gold\|purple\|vanilla\|fabric\|neoforge\|quilt; dot; pixel; large }` | Badge.svelte | |
| `Kbd { text; accent }` | `<kbd class=font-pixel>` | |
| `GradientText { text; gradient: GradientKind.emerald-gold\|cyan-emerald\|gold-amber\|purple-pink\|accent; font-size… }` | GradientText.svelte | inherits Text |
| `LoaderBadge { loader; version; size: LoaderBadgeSize.*; show-icon }` | LoaderBadge.svelte | |
| `MonogramTile { name; hue; avatar; has-avatar; size; shape: MonogramShape.tile\|rounded\|circle }` | MonogramTile.svelte | |
| `TextField { text <=>; placeholder; icon; password; mono; clearable; has-error; field-height; pad-y; fill; focus-color; edited(t); accepted(t) }`, `TextArea` | `<input>` / `<textarea>` | focus ring; sets only `height` (never `min-height`), so it works inside layouts. `HiTextField` / `SsTextField` are thin aliases |
| `Field { label; hint; error; required; <TextField/Select/...> }` | Field.svelte | |
| `ProgressBar { value; max; indeterminate; tone: ProgressTone.*; thickness: ProgressThickness.*; label; show-value }` | ProgressBar.svelte | no shimmer |
| `Toggle { checked <=>; toggled(v) }`, `Checkbox { checked; text }` | switches | |
| `Slider { value <=>; minimum; maximum; step; changed(v); released(v) }` | range inputs | commit on `released` |
| `Segmented { items: ["ES","EN"]; current; selected(i) }` | segmented pills | |
| `Tabs { items: [TabItem{label, icon, badge}]; current <=>; selected(i) }` | InstanceHub tabs | ←/→ keys |
| `Select { options: [SelectOption{value,label}]; current-index <=>; placeholder; selected(i, value) }` | `<select>` | PopupWindow |
| `Modal { open <=>; title; subtitle; dialog-width; dialog-height; dismissable; close; <body>; ModalFooter { Btn… } }`, `ConfirmDialog { message; confirm-text; cancel-text; danger; confirmed; close }` | modal scaffold | put it as the LAST child of the page root Rectangle (fills it), outside layouts |
| `ScrollArea { <one layout> }` | scrolling + themed scrollbar | use instead of std ScrollView |
| `PageScaffold { content-padding; max-content-width; spacing; <children> }`, `PageHeader { title; subtitle; gradient; divider; <eyebrow children> }`, `SectionTitle { title; description }` | page wrappers | PageScaffold = ScrollArea + centered column |
| `Icon { data: Icons.search; size; color; filled; weight }`, `Icons.*` | inline feather SVGs | add page-only icons locally (`Icon { data: "M ..." }`) |
| `Emoji { char: "🎨"; size; fallback-color }` | color emoji | Twemoji images; text-presentation symbols (⛏ ▶ ⚙ ⚛ …) render as text |
| `PlaceholderPage` | — | delete its use when you port your page |

Shell components (foundation): `TopChrome` (+ `ThemeToggle`, `LanguageToggle`),
`AmbientBackdrop`, `ToastHost`, `CommandPalette`, `LoginGate`, `HotkeyLegend`.

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
| components/AmbientBackdrop | ui/shell/backdrop.slint + src/backdrop.rs | foundation |
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
| lib/updater.svelte.ts | Shell.update-* placeholders (not ported) | later |
| pages/Home + HorizonRail/Tile, TelemetryCapsule, InstanceWizardModal, MrpackModal, DryRunModal | ui/pages/home-instances/home.slint (+ your files) | home-instances |
| pages/InstanceHub | ui/pages/home-instances/instance-hub.slint | home-instances |
| components/PipLogDrawer | ui/pages/home-instances/pip-log.slint | home-instances |
| pages/VersionArmory / ImportCenter / ModCatalog / DonationsPavilion | ui/pages/library-mods/{versions,import,mods,donations}.slint | library-mods |
| pages/AccountVault / Wardrobe, components/AccountPopover | ui/pages/account-wardrobe/{account-vault,wardrobe,account-popover}.slint | account-wardrobe |
| pages/SettingsDashboard / ClientSuite / ServerRadar, components/ServerRadarCard, PingWaveform | ui/pages/settings-suite-servers/{settings,client-suite,servers,server-radar-card}.slint | settings-suite-servers |
| components/SkinViewer (skinview3d, three.js) | ui/skin3d/skin-view.slint + src/skin.rs + native/skin3d | skin-renderer |

Routes (`router::parse`, identical to App.svelte): `#/` and `#/instances` →
`Page.home`; `#/instances/:name` → `Page.instance` (+ `Router.instance-name`);
`#/servers`; `#/library/versions` (legacy `#/versions`); `#/library/import`
(legacy `#/import`); `#/mods`; `#/client`; `#/account` (`?section=skin`);
`#/settings`; `#/donaciones` → `Page.donations`; `#/armario` → `Page.wardrobe`;
anything else → home. The page component is instantiated on entry and
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
| servers.png | `#/servers` |
| library-versions.png, library-import.png | `#/library/versions`, `#/library/import` |
| mods.png | `#/mods` |
| client.png | `#/client` |
| account.png, account-scrolled.png | `#/account` |
| settings.png, settings-scrolled.png, settings-en.png, settings-light.png | `#/settings` |
| armario.png | `#/armario` |
| donaciones.png | `#/donaciones` |
| command-palette.png, command-palette-query.png | palette open / query "set" |
| instance-wizard.png | Home → "+ Nueva Instancia" |
| account-popover.png | TopChrome account popover |
| piplog-open.png | Home after Space |

Note: the Svelte orbs drift (animated), so backdrop pixels differ slightly
from the static native backdrop; compare layout, sizes and colors of content.

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

**Memory:** the Slint-generated module of the app crate is ~60 MB of Rust;
one rustc on it peaks at ~11.5-12.7 GB (debug build, test build *and* release
build alike; ~7.7 GB for `check`/`clippy`). On a 16 GB machine never let two
app-crate compilations overlap: `cargo clippy --all-targets` checks the bin and
the test harness in parallel unless you pass `-j 1`, and never run two cargo
commands at once. Release builds: `-j 2`, ~15 min. **Disk:** each incremental
session of the app crate is several GB (`target/debug/incremental` reached
~20 GB after build + test + clippy and filled the disk); use
`CARGO_INCREMENTAL=0` for one-off `test` / `clippy` runs or delete
`target/*/incremental` afterwards.

Disk is limited: `[profile.dev.package."*"]` already disables debug info for
dependencies (the app crate itself builds with `debug = "line-tables-only"`). Use your worktree's own `native/target`, and delete it when you
are done and committed. Do not push.

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
| Shell (TopChrome, theme / language toggles, palette, toasts, login gate, hotkeys) | done | dark/light and ES/EN switch live; backdrop is static (no drifting orbs), no glass blur; gradient text needs femtovg (software = first stop) |
| Home (`#/`) | done | greeting, quick actions, MOTD ticker, rail, telemetry capsule, server strip; wizard, dry-run, shared .mrpack modal |
| Instance wizard | done | 3 steps; preselected version from VersionArmory |
| InstanceHub (`#/instances/:name`) | done | config / servers / mods / danger tabs + live console |
| PiP launch log | done | Space / palette / hub button |
| Server Radar (`#/servers`) + ServerRadarCard | done | |
| VersionArmory (`#/library/versions`) | done | manifest unreachable in the sandbox (error card matches the reference); rows checked with `ESPECTRAL_SHOT_LIB=versions-fake` |
| ImportCenter (`#/library/import`) | done | dropzone border fainter than the reference (dashed CSS border approximated) |
| MrpackModal | done | hosted by Home and ImportCenter; OS drag & drop |
| ModCatalog (`#/mods`) | done | the filter placeholder stays hard-coded Spanish, as in the Svelte page |
| Client Suite (`#/client`) | done | |
| AccountVault + Skin Atelier (`#/account`) | done | 3D viewer, gallery preview / try / apply / rename / delete; MSA device-code flow not exercised (no network) |
| Wardrobe (`#/armario`) | done | Discord-linked states not exercised visually (no Discord session in the sandbox) |
| AccountPopover | done | |
| Settings (`#/settings`) | partial | "Acceso para agentes" (MCP) section missing: the engine agent API (upstream c22767f) is not ported yet |
| Donations (`#/donaciones`) | done | |
| Updater (lib/updater) | missing | `Shell.update-*` placeholders only |

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
route (instance hub + 3 tabs, servers, versions, import, mods, client,
account, settings, donations, wardrobe) via `ESPECTRAL_TOUR`; (c) Skin Atelier
(`#/account`) with the 3D viewer spinning (a gallery skin previewed in the
stage — the Wardrobe page `#/armario` has no viewer). Offline account "Steve",
anonymous Discord session. Scripts: scratchpad `mem.sh` / `mem-inner.sh`.

| Backend | (a) Home idle | (b) after all pages | (c) skin viewer spinning | idle CPU (a) / (b) | CPU (c) |
|---|---|---|---|---|---|
| `winit-software` | 47.8 MB PSS (52 MB RSS) | 60.5 MB (65 MB) | 50.9 MB (56 MB) | 0 / 1 ticks | 222 ticks ≈ 11% of a core |
| femtovg (Mesa llvmpipe) | 162.2 MB (186 MB) | 219.6 MB (244 MB) | 188.6 MB (213 MB) | 0 / 0 ticks | 4460 ticks ≈ 2.2 cores (llvmpipe rasterizes on the CPU) |

The femtovg numbers are dominated by llvmpipe (its JIT, threads and buffers);
on a real GPU driver femtovg is expected to sit much closer to the software
numbers. Idle is 0% CPU on both renderers; the viewer stops costing CPU when
paused or scrolled off-screen.
