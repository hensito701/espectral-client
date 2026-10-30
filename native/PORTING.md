# Porting the engine to Rust — foundation & cluster guide

Branch `rust-native`. The Node engine (`src/engine/*.mjs`) is being rewritten
as the Rust library crate **`espectral-engine`** (`native/engine`, lib
`espectral_engine`). A later phase adds a Slint UI that calls it directly — no
webview, no Node, no internal HTTP. `native/app` (`espectral-client` bin) is a
placeholder `main` that starts the engine and prints `health()`.

**Reference implementation:** `src/engine`, `src/ui`, `src-tauri`, `tests/` —
never modify or delete them. Read the JS module **and** its tests before
porting it. Parity is the goal: same on-disk layout and JSON formats (existing
users' data keeps working), same endpoints, same SHA-1 verification, same JVM
argv, same Java tier selection, same AOT cache keys.

## Verify (every cluster, before merge)

```sh
cd native
cargo build
cargo test -p espectral-engine
cargo check --target x86_64-pc-windows-gnu --all-targets
cargo clippy --all-targets            # keep it warning-free
cargo fmt --all                       # rustfmt.toml: max_width 120
```

Use the default `CARGO_TARGET_DIR` (`<worktree>/native/target`) — never share
a target dir across worktrees.

## Layout

```
native/
  Cargo.toml          workspace + ALL third-party versions ([workspace.dependencies])
  engine/src/
    lib.rs            module list (every module already declared — do not reorder)
    engine.rs         Engine (Arc inner), EngineOptions, start/shutdown, ActiveInstances
    api/              the facade: one `impl Engine` block per cluster (+ core.rs)
    types.rs          shared cross-cluster + API JSON types (mirror of src/ui/src/lib/types.ts)
    error.rs events.rs paths.rs platform.rs config.rs download.rs util.rs rules.rs
    stats.rs ziputil.rs nbt.rs fingerprint.rs onboarding.rs test_support.rs
    <domain>.rs       one file per JS module (stubs owned by a cluster)
  engine/tests/       integration tests (facade.rs is a template)
  app/src/main.rs     placeholder binary
```

## Ownership — module → Rust file → owner

Only edit files your cluster owns. Need a sub-module? Create it as a child of
your own module (`src/resolver.rs` + `src/resolver/fabric.rs` with
`mod fabric;` inside `resolver.rs`) — never touch `lib.rs`.

| JS module | Rust file | Owner | Status |
|---|---|---|---|
| error.mjs | `error.rs` | foundation | **ported** |
| events.mjs | `events.rs` | foundation | **ported** (SSE → broadcast) |
| config.mjs | `config.rs` (+ `paths.rs` data dir) | foundation | **ported** |
| onboarding.mjs | `onboarding.rs` | foundation | **ported** (calls cluster contracts) |
| stats.mjs | `stats.rs` | foundation | **ported** |
| ziputil.mjs | `ziputil.rs` | foundation | **ported** |
| nbt.mjs | `nbt.rs` | foundation | **ported** |
| fingerprint.mjs | `fingerprint.rs` | foundation | **ported** |
| download.mjs | `download.rs` | foundation | **ported** (rustls, retries, mirrors, .part resume) |
| server.mjs / cli.mjs (non-HTTP) | `engine.rs`, `api/core.rs` | foundation | **ported** |
| routes/misc.mjs helpers (decodePngImage, isWithinDir) | `util.rs` | foundation | **ported** |
| resolver.mjs rulesAllow/mavenPath/nativesClassifier/libraryDownload | `rules.rs` | foundation | **ported** |
| resolver.mjs | `resolver.rs` | game-resolve | stub (+ `resolve_classpath`, `load_instance_meta` done) |
| runtimes.mjs | `runtimes.rs` | game-resolve | stub |
| jvm.mjs | `jvm.rs` | game-resolve | stub (+ `jcmd_path`) |
| neoforge.mjs | `neoforge.rs` | game-resolve | stub (+ `neoforge_version_id`) |
| launch.mjs | `launch.rs` | game-launch | stub |
| aot.mjs | `aot.rs` | game-launch | stub (+ `cache_key` done) |
| game-sessions.mjs | `game_sessions.rs` | game-launch | stub (+ `read_sessions`) |
| version-change.mjs | `version_change.rs` | game-launch | stub |
| instances.mjs | `instances.rs` | content | stub (+ `effective_game_dir(_by_name)`, `effective_mods_dir`) |
| mods.mjs | `mods.rs` | content | stub |
| mrpack.mjs | `mrpack.rs` | content | stub |
| import.mjs | `import.rs` | content | stub |
| servers.mjs | `servers.rs` | content | stub |
| accounts.mjs | `accounts.rs` | accounts-skins | read side done (list/get/active/public/offline uuid); writes to port |
| msauth.mjs | `msauth.rs` | accounts-skins | stub |
| skins.mjs | `skins.rs` | accounts-skins | stub |
| cosmetics.mjs | `cosmetics.rs` | accounts-skins | stub |
| backgrounds.mjs | `backgrounds.rs` | accounts-skins | stub |
| identity.mjs | `identity.rs` | identity-discord | stub (+ `normalize_uuid`, `identity_url`) |
| grant.mjs | `grant.rs` | identity-discord | stub (constants) |
| discordAuth.mjs | `discord_auth.rs` | identity-discord | stub (constants) |
| discord.mjs | `discord.rs` | identity-discord | stub |
| presence.mjs | `presence.rs` | identity-discord | stub |
| bridge.mjs | `bridge.rs` | identity-discord | stub (+ `mint_token`) |
| client.mjs (+ suite-registry.json) | `client.rs` | identity-discord | stub |
| cosmetics-catalog.json | `include_str!` from `src/engine/` in `cosmetics.rs` | accounts-skins | — |
| *(new)* loopback HTTP listener | `loopback.rs` | identity-discord | stub |

Facade files: `api/game_resolve.rs`, `api/game_launch.rs`, `api/content.rs`,
`api/accounts_skins.rs`, `api/identity_discord.rs` — each owned by the
cluster of the same name, including its `*State` struct (held by `Engine`,
reach it with `self.<cluster>_state()`), and its `on_engine_start` /
`on_engine_shutdown` hooks. `api/core.rs` is foundation (done).

JS tests to port, per module: resolver.test (rules part already in
`rules.rs`), runtimes.test, jvm.test, linux-compat.test, neoforge.test →
game-resolve · launch.test, preferences.test, multi-instance.test,
presence.test, aot.test, game-sessions.test, version-change.test →
game-launch · mods.test, mrpack.test, import.test, (instances parts of
multi-instance / personalization / backgrounds tests) → content ·
accounts.test, msauth.test, skin.test, skins.test, skin-avatar.test,
cosmetics.test, backgrounds.test, personalization(-routes).test →
accounts-skins · identity.test, grant.test, discord-auth.test, bridge.test,
client-config.test, suite-registry.test → identity-discord. Already ported:
config, config-identity-migration (config part), stats, ziputil, nbt,
download, decodePngImage (personalization.test).
`server-cors.test` and `skin-gallery-ui.test` cover HTTP/UI only — not ported.

## Route → Engine method

Binary routes (PNG) return `Vec<u8>`. Methods return the JS JSON shapes
(reply structs serialize identically). Query/body params become arguments.

| Route | Method | Cluster |
|---|---|---|
| GET /api/health | `health` | core ✅ |
| POST /api/shutdown | `request_shutdown` | core ✅ |
| GET /api/events (SSE) | `subscribe` → `broadcast::Receiver<EngineEvent>` | core ✅ |
| POST /api/open-folder | `open_folder(path)` | core ✅ |
| GET / PUT /api/theme | `get_theme` / `set_theme` | core ✅ |
| GET / PUT /api/accent | `get_accent` / `set_accent` | core ✅ |
| GET / PATCH /api/config | `get_config` / `patch_config(ConfigPatch)` (`ConfigPatch::from_json` accepts `{patch}` envelope) | core ✅ |
| POST /api/pick-file, /api/pick-folder | — not ported (UI uses `rfd`) | — |
| GET /api/versions | `list_versions` | game-resolve |
| GET /api/versions/:id/resolve | `resolve_version_info(id)` | game-resolve |
| GET /api/jvm | `jvm_info` | game-resolve |
| POST /api/instances/:name/launch | `launch_instance(name, LaunchRequest)` → `LaunchOutcome` | game-launch |
| POST /api/instances/:name/stop | `stop_instance` | game-launch |
| POST /api/instances/:name/train | `train_instance` | game-launch |
| GET /api/instances/:name/aot | `instance_aot_status` | game-launch |
| GET /api/launch/:key/log?cursor | `launch_log(key, cursor)` | game-launch |
| GET /api/launches | `list_launches` | game-launch |
| GET /api/stats/launches?limit | `launch_stats(limit)` ✅ | game-launch |
| POST /api/instances/:name/version-change/plan | `plan_version_change` | game-launch |
| POST /api/instances/:name/version-change | `apply_version_change` | game-launch |
| GET /api/instances | `list_instances` | content |
| POST /api/instances | `create_instance(CreateInstanceRequest)` | content |
| GET /api/instances/:name | `get_instance_detail` | content |
| DELETE /api/instances/:name | `delete_instance` | content |
| PATCH /api/instances/:name | `patch_instance(name, InstancePatch)` | content |
| GET / PUT /api/instances/:name/servers | `instance_servers` / `put_instance_servers` | content |
| GET /api/instances/:name/options | `instance_options` | content |
| POST /api/instances/:name/options/import | `import_instance_options(name, source)` | content |
| POST / DELETE / GET /api/instances/:name/icon | `upload_instance_icon` / `remove_instance_icon` / `instance_icon` | content |
| GET /api/instances/:name/mods | `list_mods` | content |
| GET /api/instances/:name/mods-dir | `mods_dir` | content |
| POST …/mods/:filename/enable, /disable | `set_mod_enabled(name, filename, bool)` | content |
| POST …/mods/install | `install_mod_preset(name, ModPreset)` | content |
| GET …/mods/preset | `mod_preset_info` | content |
| GET /api/modrinth/search | `search_modrinth(ModrinthSearchQuery)` | content |
| POST …/mods/install-modrinth | `install_modrinth_mod(name, InstallModrinthRequest)` | content |
| POST /api/instances/import-mrpack | `import_mrpack(ImportMrpackRequest)` | content |
| GET /api/import/sources | `import_sources` | content |
| POST /api/instances/:name/import | `import_profile(name, ImportProfileRequest)` | content |
| GET /api/servers | `server_statuses` | content |
| GET / PATCH /api/instances/:name/client | `get_client_info` / `patch_client(name, ClientPatch)` | content (logic in `client.rs`, identity-discord) |
| GET / POST /api/accounts | `list_accounts` / `create_account(username)` | accounts-skins |
| POST /api/accounts/active | `set_active_account` | accounts-skins |
| POST / DELETE / GET /api/accounts/:u/avatar | `upload_account_avatar` / `remove_account_avatar` / `account_avatar` | accounts-skins |
| POST /api/accounts/:u/avatar-color | `set_account_avatar_color` | accounts-skins |
| GET / PUT /api/accounts/microsoft/client-id | `msa_client_id` / `set_msa_client_id` | accounts-skins |
| POST /api/accounts/microsoft/device-code | `msa_device_code` | accounts-skins |
| POST /api/accounts/microsoft/poll | `msa_poll(flow_id)` → `MsaPollOutcome` | accounts-skins |
| POST /api/accounts/microsoft/logout | `msa_logout` | accounts-skins |
| GET /api/accounts/:u/skin, skin.png | `account_skin` / `account_skin_png` | accounts-skins |
| POST / DELETE /api/accounts/:u/skin | `upload_account_skin` / `reset_account_skin` | accounts-skins |
| GET / POST /api/skins | `list_library_skins` / `save_library_skin` | accounts-skins |
| GET /api/skins/:id.png | `library_skin_png` | accounts-skins |
| PATCH / DELETE /api/skins/:id | `patch_library_skin` / `delete_library_skin` | accounts-skins |
| POST /api/skins/:id/apply | `apply_library_skin(id, username)` | accounts-skins |
| POST /api/skins/import-vanilla | `import_vanilla_skins` | accounts-skins |
| GET /api/cosmetics, PUT /api/cosmetics/:uuid | `get_cosmetics` / `put_cosmetics` | accounts-skins |
| POST /api/bridge/visible | `bridge_visible(token, uuids)` (HTTP via `loopback.rs`) | accounts-skins (logic in `bridge.rs`) |
| GET / POST /api/backgrounds, POST …/frames | `list_backgrounds` / `import_image_background` / `import_frames_background` | accounts-skins |
| DELETE /api/backgrounds/:id, GET …/image | `delete_background` / `background_image` | accounts-skins |
| PATCH /api/instances/:name/personalization | `patch_personalization(name, PersonalizationPatch)` | accounts-skins |
| GET /api/identity | `identity_state` | identity-discord |
| POST /api/identity/:uuid/verify, /refresh | `verify_identity` / `identity_refresh` | identity-discord |
| GET / DELETE /api/identity/:uuid/discord(/start) | `identity_discord_start` / `identity_discord_unlink` | identity-discord |
| GET /api/discord/auth/start, /claim | `discord_auth_start` / `discord_claim(state)` | identity-discord |
| GET /api/discord/callback | `discord_callback(query)` (HTTP via `loopback.rs`) | identity-discord |
| GET /api/discord/me, POST /logout | `discord_me(session_id, mock)` / `discord_logout(session_id)` | identity-discord |

(`PATCH /api/accounts/:username` is called by the UI but never existed in the
engine — the UI falls back to avatar-color; not a route.)

## Cross-cluster contract functions

Called across clusters; stubs return `EngineError::NotImplemented`. Keep the
signatures (add parameters only with a note in the merge request). All take
`&Engine` first.

- **game-resolve**: `resolver::{get_version_manifest, resolve_version,
  get_version_json, read_local_version_json✅, resolve_version_chain,
  fabric_supported_for_version, resolve_fabric, install_libraries,
  install_fabric_libraries, install_assets, seed_instance,
  resolve_classpath✅, load_instance_meta✅}`, `runtimes::{java_for_version,
  java_for_major_exact}`, `jvm::{get_jvm_info, probe_java, jcmd_path✅}`,
  `neoforge::{neoforge_version_id✅, ensure_neoforge_loader,
  launch_artifacts_ready, latest_neoforge_for}`.
- **game-launch**: `aot::{cache_key✅, queue_train_instance (+TrainOptions),
  instance_key}`, `launch::{resolve_launch (+ResolveLaunchOptions,
  LaunchAccountInput), dry_run_result}`, `game_sessions::{read_sessions✅,
  live_sessions}`, `version_change::{plan_version_change, apply_version_change}`.
- **content**: `instances::{get_instance, list_instances, create_instance,
  effective_game_dir✅, effective_game_dir_by_name✅, effective_mods_dir✅,
  aot_status, compute_mod_set_hash}`, `mods::{install_preset,
  ensure_branding_seeded, plan_mods_for_target, apply_mods_plan}`,
  `import::{detect_import_sources, import_profile}`.
- **accounts-skins**: `accounts::{list_accounts✅, get_account✅,
  get_active_account✅, public_account✅, offline_uuid✅, has_avatar✅,
  is_valid_username✅}`, `msauth::{ensure_minecraft_token → McToken,
  prewarm_msa_token}`, `cosmetics::{write_cosmetics_file, share_enabled}`,
  `backgrounds::{stage_background_pack, migrate_legacy_background}`.
- **identity-discord**: `identity::{normalize_uuid✅, identity_url✅,
  flags_for, refresh_grant_soft}`, `client::{load_client_config,
  seed_client_config, get_client_info, patch_client_config}`,
  `presence::{launch_started(LaunchPresence), launch_exited, sync_share}`,
  `discord::{set_presence(Presence), clear_presence}`, `bridge::mint_token✅`,
  `loopback::configured_port✅` (the bridge port game-launch puts in
  `-Despectral.bridge.url`).

Need something not listed? Add a `pub fn` to the OWNING cluster's module only
if you own it; otherwise ask the owner (or add a clearly marked stub there in
your merge and flag it).

## Shared types (`types.rs`)

API shapes (mirror `src/ui/src/lib/types.ts` field-for-field): `VersionEntry
VersionManifest VersionInfo JvmInfo AppConfig InstanceSummary InstanceDetail
ModEntry ModPresetInfo ModrinthProject ModrinthSearchResult ModRef
ModInstallPlan ServerEntry ServerStatus ImportSource ImportResult LunarInfo
AotStatus AotProof DryRunResult LaunchReply LaunchHandle LogChunk LaunchStat
VersionTarget VersionChangePlan VersionChangeResult ClientConfig ClientInfo
ClientPatch PersonalizationPatch ClientRegistryEntry ClientMacro SkinInfo
SkinLibraryEntry IdentityAccount IdentityState CosmeticsCatalog
CosmeticsState CosmeticsAccountState CosmeticsPatch BackgroundPack
PublicAccount MsDeviceFlow HealthInfo OkReply CreateInstanceRequest InstancePatch`.

Domain shapes crossing clusters: `InstanceMeta` (instance.json), `Account` /
`MicrosoftAuth` (config.accounts), `JavaRuntime` (config.jvm / runtime
info.json), `JavaProbe`, `LibraryDownload`, `FabricProfile`, `VersionJson`
(= `serde_json::Value`), `LibrariesInstall`, `FabricLibrariesInstall`,
`AssetsInstall`, `LaunchAccount`, `ResolvedLaunch` (+ `ResolvedVersionRef`,
`LoaderLaunch`), `LaunchMode`, `GameSession` (running-games.json), enums
`Loader TokenKind JvmSource ModPreset SkinVariant OverwritePolicy ImportSourceKind`.

Rules: on-disk documents deserialize **leniently** (every field defaulted,
unknown keys kept in a flattened `extra`, odd values ignored via `types::de`)
so a JS-written file always round-trips. Patches use `Option<Option<T>>`
(`de::double_option`) to tell explicit `null` from absent. Put
cluster-private types in your own module; edit `types.rs` only to fix a
shared type (separate commit, mention it at merge).

## Conventions

**Errors** — `EngineResult<T> = Result<T, EngineError>`. Port
`httpError(status, code, msg)` as `EngineError::api(status, code, msg)`
(helpers `bad_request`, `not_found`, `conflict`); `throw new Error(msg)` as
`EngineError::other(msg)`; lowercase codes from routes/mods.mjs `sendError`
as `EngineError::api_verbatim`. JS checks like `err.code === 'X'` →
`err.has_code("X")`. The UI gets `{status, code, message}` from
`err.envelope()` (JS `normalizeError`). Stubs: `crate::not_ported!("…")` (501
NOT_IMPLEMENTED).

**Events** — `engine.emit(EngineEvent::…)`; every JS SSE event is a variant
(`launch-log launch-exit launch-ready mod-progress asset-progress
train-progress train-done import-progress import-done`); `name()` and
`payload()` reproduce the SSE frame. `mod-progress` / `asset-progress` use
`ProgressEvent` (common keys typed + `extra`). Batch downloads emit their
per-item start/done/error events themselves when an item carries
`DownloadItem::event(ProgressChannel::Mod|Asset, context)`.

**Paths** — always via `engine.paths()` (`Paths` spells out the JS layout:
config.json, versions/, libraries/, assets/, runtimes/jdk-N/, cache/aot/<key>/,
instances/<name>/{instance.json,icon.png,natives,resolved.json}, avatars/,
skins/, skin-library(.json), backgrounds/, identity.json, cosmetics.json,
discord-users.json, mc-token-cache.json, running-games.json,
launch-stats.jsonl) and `paths::client_config_in(game_dir)` etc. for per-game-dir
files. A custom `instance.game_dir` moves game files, never instance.json —
use `instances::effective_game_dir`. Atomic JSON writes:
`paths::write_json_atomic` (`.tmp` + rename, 2-space indent like the JS).
Data dir: `ESPECTRAL_DATA_DIR` → `%LOCALAPPDATA%\espectral-client` (Windows)
→ `$XDG_DATA_HOME|~/.local/share/espectral-client` → `<exe dir>/data`.

**Config** — `engine.config()` keeps config.json as an ordered JSON map
(unknown keys survive). Typed accessors replicate JS coercions
(`discord_enabled()` = `!== false`, `default_memory_mb()`, `jvm()`,
`accounts()`/`set_accounts()`, `is_true(key)` …). Read-modify-write with
`update(|doc| …)`. Secrets (`access_token xbl_token xsts_token user_hash`) are
stripped at any depth on every write — keep MC tokens in your cluster state.

**HTTP** — `engine.http()`: `fetch_text/fetch_json/fetch_bytes` (3 attempts,
429/5xx retry, host+reason in errors), `download_file` (sha1/size verify,
`.part` + Range resume, per-dest lock, mtime-preserving hashless re-download),
`download_all` (concurrency = config `download_concurrency`), Fabric
`meta2`/`maven2` mirror fallback built in. For POSTs/other verbs use
`engine.http().client()` (rustls reqwest). Hash helpers: `download::sha1_hex`,
`download::sha1_file`. Never add native-tls/OpenSSL.

**Async** — tokio. Public methods are `async`; blocking fs work that can be
large (hashing, extraction) goes through `tokio::task::spawn_blocking`.
Fire-and-forget work (JS `void (async () => …)()`) → `engine.spawn_background(name, fut)`
(cancelled at shutdown). Long-lived loops select on `engine.cancel_token()`.
Logging: `tracing` (`warn!` where JS used `console.warn`, keep the
`[module]` prefixes).

**Platform** — `crate::platform`: `os_name()` (Mojang 'windows|linux|osx'),
`node_arch()` (Node spelling — AOT keys hash it), `is_pid_alive`,
`total_memory_bytes`, `open_path`, `open_url`, `now_iso()` (JS
`toISOString`), `now_ms`, `mtime_ms`. Windows-only code behind `cfg(windows)`
with `windows-sys` (features pinned in the workspace). JS `windowsHide: true`
→ `CREATE_NO_WINDOW` on every spawned helper; the game itself is spawned
detached so it outlives the launcher.

**Tests** — `#[cfg(test)]` in your module or `engine/tests/<cluster>_*.rs`.
`test_support::temp_engine()` gives an Engine on a temp data dir (no system
proxy, ms backoff); `test_support::MockServer` is a loopback HTTP server
(`MockResponse::{ok,json,status,drop_connection}`, `.hits()`, `.requests()`);
map a real host onto it with `HttpOptions::mirrors` or make base URLs
injectable via your cluster state. `test_support::fixture(name)` →
`tests/fixtures/<name>` (shared with the JS tests; do not modify them).
Tests must never touch the real network or the real home dir (detection code
must take injectable roots). Engine start hooks and onboarding are off in
`test_options()` — keep it that way.

**Dependencies** — only crates pinned in `native/Cargo.toml`
`[workspace.dependencies]` (tokio, tokio-util, futures-util, bytes, reqwest
(rustls), axum 0.7, serde/serde_json (preserve_order), indexmap, thiserror,
tracing, zip 2, flate2, tar, sha1, sha2, md-5, hex, base64 0.22,
ed25519-dalek 2 (pkcs8), rand 0.8, uuid, url, regex, chrono, dirs, libc,
windows-sys 0.59, tempfile). If you truly need another crate, add it there in
its own commit and call it out at merge.

## Intentionally NOT ported

- The HTTP server, router, CORS / Host / CSRF guards, static UI serving,
  `/api/events` SSE — the UI calls the `Engine` directly.
- `POST /api/pick-file`, `/api/pick-folder` (PowerShell STA pickers) — the
  native UI uses `rfd` file/folder dialogs.
- cli.mjs parent-process watchdog and `--open` browser launch — the engine
  lives in the UI process.

Still needed as HTTP (owned by identity-discord, `loopback.rs`): the in-game
mod calls `POST /api/bridge/visible` on `127.0.0.1:<ESPECTRAL_PORT|4199>`, and
Discord redirects to `http://127.0.0.1:4199/api/discord/callback` (registered
redirect URI). Keep the Host check and the `x-espectral-client: 1` guard there.

The 3D skin viewer is a UI concern (three.js in a minimal webview, or a
native renderer); the engine only serves skin PNG bytes.

## Known, deliberate differences from the JS

- `AppConfig.download_concurrency` / `aot_auto_train` are typed (invalid
  persisted values read as 6 / JS-truthiness) instead of echoed raw.
- `download_file` retries a body stream that breaks mid-transfer (resuming
  from `.part`) instead of failing on the first non-"premature close" error.
- `stats` re-serializes legacy rows with explicit `null`s for missing
  `spawn_ms` / `boot_ms` / `phases`.
- Health version comes from the workspace version (`native/Cargo.toml`,
  keep it equal to package.json).
