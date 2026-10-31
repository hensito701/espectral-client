# Porting the engine to Rust — foundation & cluster guide

Branch `rust-native`. The Node engine (`src/engine/*.mjs`) is being rewritten
as the Rust library crate **`espectral-engine`** (`native/engine`, lib
`espectral_engine`). A later phase adds a Slint UI that calls it directly — no
webview, no Node, no internal HTTP. `native/app` (`espectral-client` bin) is the
native Slint UI that links the engine — see `native/app/UI-PORTING.md`.

**Reference implementation:** `src/engine`, `src/ui`, `src-tauri`, `tests/` —
never modify or delete them. Read the JS module **and** its tests before
porting it. Parity is the goal: same on-disk layout and JSON formats (existing
users' data keeps working), same endpoints, same SHA-1 verification, same JVM
argv, same Java tier selection, same AOT cache keys.

> **Status (integration on `rust-native`, agent access on `rust-agent`):** all
> five clusters are merged and wired, and the *agent access* feature that
> landed upstream afterwards (c22767f + the MCP tool texts of 5fc98f7:
> routes/agent.mjs, agent.mjs, ffmpeg.mjs, mcp.mjs, capture tools in mods.mjs,
> launch options) is ported too — see [Agent access](#agent-access). Every route
> of the JS engine has a working `Engine` method or loopback HTTP handler. On-disk compatibility was
> checked both ways against a data dir written by the JS engine
> (`cargo run -p espectral-engine --example data_dir_probe -- <dir> [--write]`,
> agent config + token included).

## Verify (every cluster, before merge)

```sh
cd native
cargo build
cargo test --workspace
cargo check --target x86_64-pc-windows-gnu --all-targets
cargo clippy --workspace --all-targets  # keep it warning-free (also with --target x86_64-pc-windows-gnu)
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
  app/                the native Slint UI binary (see app/UI-PORTING.md)
  skin3d/             espectral-skin3d: CPU 3D skin renderer used by the UI
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
| resolver.mjs | `resolver.rs` | game-resolve | **ported** |
| runtimes.mjs | `runtimes.rs` | game-resolve | **ported** |
| jvm.mjs | `jvm.rs` | game-resolve | **ported** |
| neoforge.mjs | `neoforge.rs` | game-resolve | **ported** |
| launch.mjs | `launch.rs` | game-launch | **ported** (incl. upstream Windows `stackFlags` and the agent launch options) |
| aot.mjs | `aot.rs` | game-launch | **ported** |
| game-sessions.mjs | `game_sessions.rs` | game-launch | **ported** |
| version-change.mjs | `version_change.rs` | game-launch | **ported** |
| instances.mjs | `instances.rs` | content | **ported** (incl. the `capture_tools` toggle) |
| mods.mjs | `mods.rs` | content | **ported** (incl. capture tools: Replay Mod pins, Iris/Sodium pins, shader packs) |
| mrpack.mjs | `mrpack.rs` | content | **ported** |
| import.mjs | `import.rs` | content | **ported** |
| servers.mjs | `servers.rs` | content | **ported** |
| accounts.mjs | `accounts.rs` | accounts-skins | **ported** |
| msauth.mjs | `msauth.rs` | accounts-skins | **ported** |
| skins.mjs | `skins.rs` | accounts-skins | **ported** |
| cosmetics.mjs | `cosmetics.rs` | accounts-skins | **ported** |
| backgrounds.mjs | `backgrounds.rs` | accounts-skins | **ported** |
| identity.mjs | `identity.rs` | identity-discord | **ported** |
| grant.mjs | `grant.rs` | identity-discord | **ported** |
| discordAuth.mjs | `discord_auth.rs` | identity-discord | **ported** |
| discord.mjs | `discord.rs` | identity-discord | **ported** |
| presence.mjs | `presence.rs` | identity-discord | **ported** |
| bridge.mjs | `bridge.rs` | identity-discord | **ported** |
| client.mjs (+ suite-registry.json) | `client.rs` | identity-discord | **ported** |
| cosmetics-catalog.json | `include_str!` from `src/engine/` in `cosmetics.rs` | accounts-skins | — |
| *(new)* loopback HTTP listener | `loopback.rs` | identity-discord + agent | **ported** (axum: bridge/visible, discord/callback, `/api/agent/*`, `/api/bridge/agent/*`) |
| agent.mjs, routes/agent.mjs (upstream c22767f) | `agent.rs`, `api/agent.rs` (UI routes), `loopback.rs` (HTTP routes) | agent | **ported** |
| ffmpeg.mjs (upstream c22767f) | `ffmpeg.rs` | agent | **ported** |
| mcp.mjs (MCP shim) | `mcp.rs` + `mcp/tools.json` (`include_str!`, dumped verbatim from the JS `toolList()`) | agent | **ported** (stdio server run as `espectral-client --mcp`; no Node needed) |

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

| Route | Method | Cluster | Status |
|---|---|---|---|
| GET /api/health | `health` | core | ported |
| POST /api/shutdown | `request_shutdown` | core | ported |
| GET /api/events (SSE) | `subscribe` → `broadcast::Receiver<EngineEvent>` | core | ported |
| POST /api/open-folder | `open_folder(path)` | core | ported |
| GET / PUT /api/theme | `get_theme` / `set_theme` | core | ported |
| GET / PUT /api/accent | `get_accent` / `set_accent` | core | ported |
| GET / PATCH /api/config | `get_config` / `patch_config(ConfigPatch)` (`ConfigPatch::from_json` accepts `{patch}` envelope) | core | ported |
| POST /api/pick-file, /api/pick-folder | — not ported (UI uses `rfd`) | — | not ported (by design) |
| GET /api/versions | `list_versions` | game-resolve | ported |
| GET /api/versions/:id/resolve | `resolve_version_info(id)` | game-resolve | ported |
| GET /api/jvm | `jvm_info` | game-resolve | ported |
| POST /api/instances/:name/launch | `launch_instance(name, LaunchRequest)` → `LaunchOutcome` | game-launch | ported |
| POST /api/instances/:name/stop | `stop_instance` | game-launch | ported |
| POST /api/instances/:name/train | `train_instance` | game-launch | ported |
| GET /api/instances/:name/aot | `instance_aot_status` | game-launch | ported |
| GET /api/launch/:key/log?cursor | `launch_log(key, cursor)` | game-launch | ported |
| GET /api/launches | `list_launches` | game-launch | ported |
| GET /api/stats/launches?limit | `launch_stats(limit)` | game-launch | ported |
| POST /api/instances/:name/version-change/plan | `plan_version_change` | game-launch | ported |
| POST /api/instances/:name/version-change | `apply_version_change` | game-launch | ported |
| GET /api/instances | `list_instances` | content | ported |
| POST /api/instances | `create_instance(CreateInstanceRequest)` | content | ported |
| GET /api/instances/:name | `get_instance_detail` | content | ported |
| DELETE /api/instances/:name | `delete_instance` | content | ported |
| PATCH /api/instances/:name | `patch_instance(name, InstancePatch)` | content | ported |
| GET / PUT /api/instances/:name/servers | `instance_servers` / `put_instance_servers` | content | ported |
| GET /api/instances/:name/options | `instance_options` | content | ported |
| POST /api/instances/:name/options/import | `import_instance_options(name, source)` | content | ported |
| POST / DELETE / GET /api/instances/:name/icon | `upload_instance_icon` / `remove_instance_icon` / `instance_icon` | content | ported |
| GET /api/instances/:name/mods | `list_mods` | content | ported |
| GET /api/instances/:name/mods-dir | `mods_dir` | content | ported |
| POST …/mods/:filename/enable, /disable | `set_mod_enabled(name, filename, bool)` | content | ported |
| POST …/mods/install | `install_mod_preset(name, ModPreset)` | content | ported |
| GET …/mods/preset | `mod_preset_info` | content | ported |
| GET /api/modrinth/search | `search_modrinth(ModrinthSearchQuery)` | content | ported |
| POST …/mods/install-modrinth | `install_modrinth_mod(name, InstallModrinthRequest)` | content | ported |
| POST /api/instances/import-mrpack | `import_mrpack(ImportMrpackRequest)` | content | ported |
| GET /api/import/sources | `import_sources` | content | ported |
| POST /api/instances/:name/import | `import_profile(name, ImportProfileRequest)` | content | ported |
| GET /api/servers | `server_statuses` | content | ported |
| GET / PATCH /api/instances/:name/client | `get_client_info` / `patch_client(name, ClientPatch)` | content (logic in `client.rs`, identity-discord) | ported |
| GET / POST /api/accounts | `list_accounts` / `create_account(username)` | accounts-skins | ported |
| POST /api/accounts/active | `set_active_account` | accounts-skins | ported |
| POST / DELETE / GET /api/accounts/:u/avatar | `upload_account_avatar` / `remove_account_avatar` / `account_avatar` | accounts-skins | ported |
| POST /api/accounts/:u/avatar-color | `set_account_avatar_color` | accounts-skins | ported |
| GET / PUT /api/accounts/microsoft/client-id | `msa_client_id` / `set_msa_client_id` | accounts-skins | ported |
| POST /api/accounts/microsoft/device-code | `msa_device_code` | accounts-skins | ported |
| POST /api/accounts/microsoft/poll | `msa_poll(flow_id)` → `MsaPollOutcome` | accounts-skins | ported |
| POST /api/accounts/microsoft/logout | `msa_logout` | accounts-skins | ported |
| GET /api/accounts/:u/skin, skin.png | `account_skin` / `account_skin_png` | accounts-skins | ported |
| POST / DELETE /api/accounts/:u/skin | `upload_account_skin` / `reset_account_skin` | accounts-skins | ported |
| GET / POST /api/skins | `list_library_skins` / `save_library_skin` | accounts-skins | ported |
| GET /api/skins/:id.png | `library_skin_png` | accounts-skins | ported |
| PATCH / DELETE /api/skins/:id | `patch_library_skin` / `delete_library_skin` | accounts-skins | ported |
| POST /api/skins/:id/apply | `apply_library_skin(id, username)` | accounts-skins | ported |
| POST /api/skins/import-vanilla | `import_vanilla_skins` | accounts-skins | ported |
| GET /api/cosmetics, PUT /api/cosmetics/:uuid | `get_cosmetics` / `put_cosmetics` | accounts-skins | ported |
| POST /api/bridge/visible | `bridge_visible(token, uuids)` (HTTP via `loopback.rs`) | accounts-skins (logic in `bridge.rs`) | ported |
| GET / POST /api/backgrounds, POST …/frames | `list_backgrounds` / `import_image_background` / `import_frames_background` | accounts-skins | ported |
| DELETE /api/backgrounds/:id, GET …/image | `delete_background` / `background_image` | accounts-skins | ported |
| PATCH /api/instances/:name/personalization | `patch_personalization(name, PersonalizationPatch)` | accounts-skins | ported |
| GET /api/identity | `identity_state` | identity-discord | ported |
| POST /api/identity/:uuid/verify, /refresh | `verify_identity` / `identity_refresh` | identity-discord | ported |
| GET / DELETE /api/identity/:uuid/discord(/start) | `identity_discord_start` / `identity_discord_unlink` | identity-discord | ported |
| GET /api/discord/auth/start, /claim | `discord_auth_start` / `discord_claim(state)` | identity-discord | ported |
| GET /api/discord/callback | `discord_callback(query)` (HTTP via `loopback.rs`) | identity-discord | ported |
| GET /api/discord/me, POST /logout | `discord_me(session_id, mock)` / `discord_logout(session_id)` | identity-discord | ported |
| GET / PUT /api/agent-access, POST /api/agent-access/rotate | `agent_access` / `set_agent_access(AgentAccessPatch)` / `rotate_agent_token` | agent | ported |
| GET /api/agent/instances, /launches, /state, /logs | HTTP only (`loopback.rs`, Bearer token) | agent | ported |
| POST /api/agent/launch, /stop, /call | HTTP only (`loopback.rs`, Bearer token) | agent | ported |
| POST /api/bridge/agent/poll, /result, /state (mod relay) | HTTP only (`loopback.rs`, bridge token) | agent | ported |
| PATCH /api/instances/:name `capture_tools` | `patch_instance` (`InstancePatch.capture_tools`; 409 ALREADY_RUNNING while running) | content | ported |
| POST /api/instances/:name/launch agent options (`agent`, `join`, `window`) | `launch_instance(LaunchRequest { agent, join, window, .. })` | game-launch | ported |

(`PATCH /api/accounts/:username` is called by the UI but never existed in the
engine — the UI falls back to avatar-color; not a route.)

## Missing and partial

Everything that arrived in upstream `main` after the foundation commit
(67614f8, c22767f, 5fc98f7) is ported. What remains differs by design:

| Area | JS | Rust status |
|---|---|---|
| `POST /api/pick-file`, `/api/pick-folder` | routes/misc.mjs | not ported by design (UI uses `rfd`) |
| `/api/events` SSE, the general HTTP server, CORS | server.mjs, events.mjs | not ported by design (`Engine::subscribe`); the loopback listener keeps the Host / `x-espectral-client` guards for the routes it serves |
| `SKIN_RATE_LIMITED` `retry_after` field | skins/msauth | partial: seconds only in the message (`EngineError` has no `retry_after`) |

## Agent access

Port of c22767f + the MCP tool texts of 5fc98f7. Design decisions:

1. **Where the agent API is served.** The UI routes (`/api/agent-access*`)
   are in-process `Engine` methods like every other route (`api/agent.rs`).
   External processes need HTTP, so the agent API (`/api/agent/*`, Bearer
   token) and the mod relay (`/api/bridge/agent/*`, per-launch bridge token)
   are on the existing loopback router (`loopback.rs`, 127.0.0.1:4199 or
   `ESPECTRAL_PORT`). They are always routed, like the JS, and answer 403
   `AGENT_DISABLED` while agent access is off (before auth, so a disabled
   engine never confirms a token). Same Host / `x-espectral-client` guards and
   `{ error: { code, message } }` envelope as the JS server.
2. **MCP shim.** Rewritten in Rust (`mcp.rs`) instead of keeping `mcp.mjs`:
   the native app must not need Node. The app binary runs it with `--mcp`
   (dispatched first thing in `main()`, before the single-instance lock,
   tokio and Slint), so nothing extra is packaged. `Engine::mcp_entry()`
   returns `{ command: <current exe>, args: ["--mcp"], env: { ESPECTRAL_DATA_DIR,
   ESPECTRAL_PORT } }`. The release build's `windows_subsystem = "windows"`
   does not break it: MCP clients spawn it with pipes, which a GUI-subsystem
   process inherits. The shim's HTTP contract is the JS one, so
   `tests/mcp-shim.test.mjs` still describes it; `tools/list` is
   byte-identical to the JS `toolList()` (descriptors in `mcp/tools.json`).

Registry: `AgentState` (`Engine::agent_state()`) holds the per-launch command
queue. Agent launches are registered when the key is minted and again on
spawn (with the game dir), re-registered when an agent game is adopted after a
restart (`GameSession.agent`), and released (in-flight calls fail with 502
`GAME_GONE`, parked polls get `[]`) on every exit path. A parked poll whose
HTTP client hangs up is dropped by axum; its guard puts any handed-over batch
back in the queue. An `/api/agent/call` whose client hangs up drops its
still-queued command (the JS left it queued; nobody would read the result).

Tests: `engine/tests/agent_http.rs`, `agent_relay.rs`, `agent_launch.rs`
(tests/agent-api, agent-route, agent-launch), `capture_ffmpeg.rs`
(tests/ffmpeg + the capture parts of tests/mods), `mcp_shim.rs`
(tests/mcp-shim).

## Cross-cluster test seams

Tests that cross clusters must sandbox every cluster, otherwise background
work reaches the real network, PATH java or %APPDATA% (see
`engine/tests/cross_cluster.rs::sandbox`):

- game-resolve: `e.game_resolve_state().isolate()` and
  `set_endpoints(ResolveEndpoints::all_on(<mock or http://127.0.0.1:9>))`;
  `update_hooks(|h| h.path_java = ...)` + `set_probe_override(...)` for a fake JDK.
- content: `e.update_content_settings(|s| s.background_on_create = false)`
  (no library seeding / preset download after `create_instance`), plus
  `modrinth_api` / `mcsrvstat_api`.
- identity-discord: `e.identity_discord_state().set_overrides(...)`
  (identity_url, session_server, discord_mock, ipc_candidates).
- accounts-skins: `AccountsSkinsState.endpoints` / `no_browser` (crate-internal).
- game-launch: `Engine::set_launch_test_hooks` (doc-hidden).

The whole suite passes inside an offline network namespace with a throwaway
`HOME` (loopback only), which is how "no real network / no real home" was
checked at integration.

## Known risks

- Windows code (detached spawn flags, taskkill, PowerShell `Get-CimInstance`
  command lines, Discord named pipes, `%LOCALAPPDATA%`) is compile-checked
  and links (`cargo build --target x86_64-pc-windows-gnu`), but has never run
  on Windows. The `#[cfg(unix)]` fake-JDK / NeoForge-installer tests do not
  cover Windows.
- `api::game_launch::tests::reopened_engine_adopts_running_game_then_stop_kills_it`
  failed once in about eight full-suite runs (adopt found no live session);
  it passes alone and in reruns. Likely a timing issue reading
  `/proc/<pid>/cmdline` under load; not root-caused.
- The JS reference moved during the port (c22767f refactored
  routes/launch.mjs into `startLaunch` for the agent API). The plain launch
  path was spot-checked, not re-audited line by line against the new file.
- Packaging must stage `assets/branding` next to the binary (`<exe>/branding`
  or `<exe>/resources/branding`); otherwise branding falls back to the repo
  path baked in at compile time (`CARGO_MANIFEST_DIR`).
- `src/ui/src/lib/types.ts` marks some fields non-null that the JS (and now
  Rust) send as `null` (`ModEntry.project_slug/version_number/version_id/sha1/size`,
  `ServerStatus.ip/version/software`); the Slint UI must treat them as optional.

## Cross-cluster contract functions

Called across clusters (all implemented now; ✅ marks the ones the
foundation shipped). Keep the signatures. All take `&Engine` first.
Integration added: `identity::flags_info` (grant + validity, used by
`cosmetics::grant_view`), `cosmetics::presence_loadout`,
`presence::republish_now`, `bridge::visible_for_token`,
`loopback::bridge_url` (the bound port, used for `-Despectral.bridge.url`),
`mods::{supports_branding, qol_pins_by_version}`, `aot::{is_cache_stale,
aot_refusal_line}`.

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

Per cluster (from the porters' reports; details in each module's docs):

- **game-resolve**: PATH java is found by walking PATH natively (exec bit,
  PATHEXT) instead of spawning `which`/`where`; Temurin archives are
  extracted in-process (tar+flate2, zip) and downloaded through
  `download_file`; `fast_client_java` is `None` without APPDATA/USERPROFILE;
  `jvm_info` maps any discovery error to 503 UNAVAILABLE (cause logged); the
  natives meta mtime compares with a 1 µs tolerance. Mojang version JSON is
  cached flat at `versions/<id>.json` (as resolver.mjs does);
  `Paths::version_json(id)` (nested) holds only local profiles (NeoForge).
  Classpath strings are joined segment by segment, so on Windows they use
  backslashes byte-identical to Node `path.join` (AOT stamps depend on it).
- **game-launch**: a trainer spawn failure ends training at once; a failing
  mod-set hash records `mod_set_hash: null`; `train-done` omits a null
  `proof` and `AotProof` omits a null `refusal`; a game spawn failure is a
  synchronous error reported through the launch-exit path (no stats row);
  game output reaches `on_log` line by line.
- **content**: `install_preset` sends a descriptive note where the JS sent
  null when dynamic pins cannot be resolved; `localeCompare` is
  approximated; typed request structs replace some body validation
  (BAD_LOADER, BAD_OVERWRITE_POLICY, string memory_mb, null-body BAD_PATCH
  cannot occur); `delete_instance` returns `(deleted, trash)`. Ported as-is,
  probably a JS bug: profile import writes into the instance dir even when a
  custom `game_dir` is set.
- **accounts-skins**: a successful `msa_poll` keeps the fresh MC token in
  memory and `msa_logout` forgets it; cosmetics.json / skin-library.json are
  re-read per call and serialized with in-process locks; slugs fold Latin
  diacritics with a table; `MsDeviceFlow.expires_in` defaults to 900; skin
  endpoints use a 30 s timeout; the multipart body is built by hand.
- **identity-discord**: Discord IPC replies are matched by nonce string (the
  JS compared a number with a string and always timed out); extra Linux IPC
  socket candidates (Flatpak, Snap, $TMPDIR, abstract `@discord-ipc-0`); the
  loopback OAuth callback sets no cookie (the UI polls `discord_claim`,
  `identity_discord_start_with_state` returns `{url, state}`); macros that do
  not fit `ClientMacro` are dropped from the typed view but kept on disk.
- **integration**: `ServerEntry.hidden/accept_textures/has_icon` default to
  false when omitted (the JS PUT accepted them missing).
