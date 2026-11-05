# Espectral Client

Open-source Minecraft launcher for [Espectral](https://espectral.uhce.top). A native desktop app
written in Rust, with a [Slint](https://slint.dev) interface in Spanish and English — no WebView, no
Node.js runtime.

Licensed **GPL-3.0** (see [LICENSE](LICENSE)).

---

## What it does

- Launches Minecraft: vanilla, Fabric, NeoForge and Forge, versions 1.15.2 through the current
  release. Forge is offered for 1.8.9, 1.12.2, 1.21.x and 26.x (the classic modpack platforms).
- Downloads and verifies everything itself — versions, libraries, assets, natives — from Mojang's
  official manifests. No other launcher needs to be installed.
- Provisions the right Java automatically (Temurin 8 / 16 / 17 / 21 / 25 per version).
- Imports `.mrpack` modpacks and browses/installs mods from Modrinth with SHA-1 verification.
- Microsoft accounts (device-code sign-in) and offline accounts.
- Imports your existing settings — `options.txt`, server list — from a vanilla `.minecraft`
  or another launcher's profile.
- Discord presence, live launch progress, per-instance mod management and the in-game Espectral
  Suite.
- Optional **agent access**: a local MCP server (`espectral-client --mcp`) an AI agent can drive.
  See [docs/agent-access.md](docs/agent-access.md).

### Faster boot on Java 25

On instances that run on the Java 25 tier, the launcher trains and reuses an Ahead-of-Time cache
([JEP 483](https://openjdk.org/jeps/483) / [JEP 514](https://openjdk.org/jeps/514)), keyed by game
version, Java build and OS/architecture. A cache is only used after a proof log confirms the JVM
actually linked it, and it is retrained automatically when the classpath changes.

Measured on a single Windows machine, this cut time-to-main-menu by roughly 13–19% compared to the
same instance without AOT. Boot time depends heavily on your hardware, disk and installed mods —
treat that figure as one data point, not a promise.

### Espectral client mod

`branding-mod/` is a small Fabric mod, built from this repo, that provides a title-screen hook, an
in-game toggle menu, and a versioned config file shared with the launcher
(`<instance>/config/espectral-client.json`). Zoom, fullbright and nofog are built into the mod and
apply live — no third-party jars, no restart. (Gamma Utils / Clear Fog jars left from older
versions are inert leftovers; the QoL preset stays installable but is no longer required.)

Plus `branding-mod-nf/`, a brand-only NeoForge companion (MC 1.21.1) that reports `espectral:neoforge:<version>` on the `minecraft:brand` handshake — no menu port, same version wiring, and `branding-mod-forge189/`, the Espectral Menu port to Forge 1.8.9 (suite features, no cosmetics/recording yet).

It is deliberately small and still early — the launcher is the mature half of this project.

---

## Install

**Users:** download the installer from [espectral.uhce.top/client](https://espectral.uhce.top/client)
and run it (Windows `EspectralClient_<version>_x64-setup.exe`) or the `.deb` (Linux). The app is
self-contained: it needs neither Node.js nor WebView2, and it manages its own Java.

The installer is not code-signed, so Windows SmartScreen may warn on first run.

---

## Build from source

Requirements: a Rust toolchain (1.80+, [rustup](https://rustup.rs)) and, on Linux, the Slint/winit
development packages (`libfontconfig1-dev libxkbcommon-dev libwayland-dev libx11-dev
libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev`).

```bash
git clone https://github.com/hensito701/espectral-client.git
cd espectral-client
cd native
cargo run --release -p espectral-client      # builds and starts the launcher
```

| Task | Command |
|---|---|
| Build + run the launcher | `cargo run --release -p espectral-client` (from `native/`) |
| All tests | `cargo test --workspace` |
| Lint (warning-free gate) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Windows cross-check | `cargo check -p espectral-client --target x86_64-pc-windows-gnu` |
| JS tests (config/accounts + Suite registry) | `node --test tests/*.test.mjs` |
| Client mod (Java 21) | `cd branding-mod && ./gradlew build` |
| Brand companion, NeoForge (Java 21) | `cd branding-mod-nf && ./gradlew build` |
| Espectral Menu, Forge 1.8.9 (Gradle on JDK 21, Java 8 toolchain) | `cd branding-mod-forge189 && ./gradlew build -Porg.gradle.java.installations.paths=<jdk8>` |
| Rebuild + ship the mod jars | `npm run deploy:mod` |

`ESPECTRAL_DATA_DIR` overrides the data directory, `SLINT_BACKEND=winit-software` falls back to the
software renderer (the launcher also switches to it by itself on a PC where OpenGL fails), and
`SLINT_BACKEND=winit-femtovg` forces OpenGL. The launcher logs to `<data>/logs/launcher.log`
(`ESPECTRAL_LOG=debug` for more detail); on Windows the data directory is
`%LOCALAPPDATA%\espectral-client`.

### Packaging

Releases are built locally (not in CI) from this Linux checkout:

| Platform | Script | Output |
|---|---|---|
| Windows | `scripts/build-windows-exe.sh` | `build/windows/EspectralClient_<ver>_x64-setup.exe` (+ `.sig`) |
| Linux | `scripts/build-linux-deb.sh` | `build/linux/espectral-client_<ver>_amd64.deb` (+ `.sig`) |
| Signing | `scripts/sign-update.sh <file>` | `<file>.sig` (minisign, `tauri signer` format) |

The Windows script cross-compiles for `x86_64-pc-windows-gnu` (mingw-w64 + NSIS 3.x) and needs the
`x86_64-pc-windows-gnu` rustup target. The Linux `.deb` installs `/usr/bin/espectral-client` and
`/usr/lib/Espectral Client/{espectral-client,branding/}`. Both stage `assets/branding/` into a
`branding/` folder next to the binary — that is where the launcher looks first.

Both installers are drop-in replacements for the 1.6.x Tauri build: same Windows install folder,
same Apps & Features entry, same update feed and signature format.

---

## Architecture

```
┌────────────────────────────── native/ (Cargo workspace) ────────────────────────────┐
│  app/      espectral-client — the binary: Slint UI, pages, theme, i18n (ES/EN)      │
│            ↓ links directly (no IPC, no HTTP, no webview)                           │
│  engine/   espectral-engine — resolver, downloads, accounts, launch, AOT, mods,     │
│            instances, presence, updater, agent API/MCP server                       │
│  skin3d/   espectral-skin3d — CPU 3D skin renderer used by the UI                   │
└──────────────┬──────────────────────────────────────────────────────────────────────┘
               │ spawns java (the game outlives the launcher)
┌──────────────▼──────────────┐     ┌─────────────────────────────────────────────────┐
│ Minecraft (Fabric/NeoForge) │◄───►│ Espectral client mod (branding-mod/)            │
└─────────────────────────────┘     └─────────────────────────────────────────────────┘
```

- **`native/engine/`** (`espectral-engine`) — everything that is not UI: version resolution, Modrinth
  and `.mrpack` support, mod management, accounts, launch and AOT training, Discord presence,
  game-session adoption, the updater, and the agent/MCP surface. One module per area; the API
  facade lives in `src/api/`.
- **`native/app/`** (`espectral-client`) — the Slint UI. Pages live in `ui/pages/`, strings in
  `i18n/*.json` (every key in both `es` and `en`; `build.rs` lints them), platform glue in `src/`.
  `--mcp` turns the same binary into a stdio MCP server.
- **`native/skin3d/`** (`espectral-skin3d`) — software rasterizer for the 3D skin previews, so no
  GPU-specific code is needed for them.
- **`branding-mod/`** — shared Java source with per-version overlays behind a compatibility shim.
- **`branding-mod-nf/`** — brand-only NeoForge companion (MC 1.21.1): same handshake contract, no menu.
- **`branding-mod-forge189/`** — the Espectral Menu ported to Forge 1.8.9 (Espectral Suite features,
  agent relay; cosmetics, emotes and recording come later).

Porting notes and the original JS→Rust conventions are kept in [native/PORTING.md](native/PORTING.md)
and [native/app/UI-PORTING.md](native/app/UI-PORTING.md).

---

## Data directory

`%LOCALAPPDATA%\espectral-client` on Windows, `$XDG_DATA_HOME/espectral-client` (usually
`~/.local/share/espectral-client`) on Linux. `<repo>/data` in development. Override with
`ESPECTRAL_DATA_DIR`. This is the same folder 1.6.x used, so accounts, instances and settings carry
over.

```
data/
├── config.json              # settings + accounts
├── instances/<name>/        # one directory per instance (its gameDir)
│   ├── config/espectral-client.json
│   ├── mods/                # .jar / .jar.disabled
│   └── options.txt
├── versions/  libraries/  assets/    # Mojang content
├── runtimes/jdk-<major>/    # downloaded Temurin JDKs
└── cache/                   # resolver + AOT caches (regenerable)
```

---

## Microsoft sign-in

The launcher ships with a public Azure application ID (a client ID is not a secret). Sign-in with
that default ID becomes available once Mojang approves the application; until then the UI explains
the pending state instead of showing a raw error. Offline accounts always work, and you can supply
your own client ID — see [docs/msa-byo-client-id.md](docs/msa-byo-client-id.md).

Account credentials stay on your machine. See [SECURITY.md](SECURITY.md).

---

## Contributing

Issues and pull requests are welcome. Before opening a PR run, from `native/`:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

CI runs those on Linux, cross-checks the Windows target, and runs the engine tests natively on
Windows.

## License

GPL-3.0 — see [LICENSE](LICENSE). Bundled third-party mods (Ok Zoomer, Gamma Utils, Clear Fog) keep
their own licenses; fabric-loader is Apache-2.0.
