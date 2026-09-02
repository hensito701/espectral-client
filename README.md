# Espectral Client

Open-source Minecraft launcher for [espectral.es](https://espectral.es). Windows desktop app
(Tauri 2 shell) around a plain-Node engine, with a Svelte 5 interface in Spanish and English.

Licensed **GPL-3.0** (see [LICENSE](LICENSE)).

---

## What it does

- Launches Minecraft: vanilla, Fabric and NeoForge, versions 1.15.2 through the current release.
- Downloads and verifies everything itself — versions, libraries, assets, natives — from Mojang's
  official manifests. No other launcher needs to be installed.
- Provisions the right Java automatically (Temurin 8 / 16 / 17 / 21 / 25 per version).
- Imports `.mrpack` modpacks and browses/installs mods from Modrinth with SHA-1 verification.
- Microsoft accounts (device-code sign-in) and offline accounts.
- Imports your existing settings — `options.txt`, server list — from a vanilla `.minecraft`
  or another launcher's profile.
- Discord presence, live launch progress, and per-instance mod management.

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
(`<instance>/config/espectral-client.json`). It also manages a few third-party quality-of-life jars
(Ok Zoomer, Gamma Utils, Clear Fog) by enabling or disabling them for the next launch.

It is deliberately small and still early — the launcher is the mature half of this project.

---

## Install

**Users:** download the installer from [espectral.es/client](https://espectral.es/client) and run it.
The app is self-contained; it bundles its own Node runtime and manages its own Java.

The installer is not code-signed, so Windows SmartScreen may warn on first run.

---

## Build from source

```bash
git clone https://github.com/hensito701/espectral-client.git
cd espectral-client
npm install
npm run dev          # engine on http://127.0.0.1:4199, opens the UI in your browser
```

The engine is plain Node ESM (`node:http`, one runtime dependency: `extract-zip`). Node 24+ is
recommended. `ESPECTRAL_PORT` overrides the port; `ESPECTRAL_DATA_DIR` overrides the data directory.

| Task | Command |
|---|---|
| Engine + browser UI | `npm run dev` |
| UI dev server (Vite) | `npm run ui` |
| Build the UI | `npm run build` |
| Desktop shell (dev) | `npm run tauri:dev` |
| Tests | `npm test` |
| Type/template check | `npm run check` |
| Client mod (Java 21) | `cd branding-mod && ./gradlew build` |

### Windows installer

`scripts/build-windows-exe.sh` cross-compiles the Windows binary and NSIS installer from Linux or
WSL. It needs, once: rustup with the `x86_64-pc-windows-gnu` target, mingw-w64, and NSIS 3.x.
Output lands in `src-tauri/target/x86_64-pc-windows-gnu/release/`.

The desktop shell targets Windows only — port reclaim shells out to `netstat`/`taskkill`, and
bundling is NSIS-only. The engine, UI and mod are portable.

---

## Architecture

```
┌────────────────────── Tauri 2 shell (src-tauri/, Rust) ──────────────────────┐
│ spawns and supervises the engine · single-instance lock · .mrpack handler    │
└──────────────┬───────────────────────────────────────────────────────────────┘
               │ 127.0.0.1:4199 — REST + SSE, loopback only
┌──────────────▼──────────────┐     ┌──────────────────────────────────────────┐
│ Node engine (src/engine/)   │◄────│ Svelte 5 UI (src/ui/)                    │
│ versions · mods · accounts  │HTTP │ Vite + TypeScript, ES/EN, live progress  │
│ launch · AOT · presence     │     └──────────────────────────────────────────┘
└──────────────┬──────────────┘
               │ spawns java
┌──────────────▼──────────────┐     ┌──────────────────────────────────────────┐
│ Minecraft (Fabric/NeoForge) │◄───►│ Espectral client mod (branding-mod/)     │
└─────────────────────────────┘     └──────────────────────────────────────────┘
```

- **`src/engine/`** — resolver and version handling, Modrinth and `.mrpack` support, mod management,
  accounts, launch and AOT training, Discord presence. One route module per area under `routes/`.
- **`src/ui/src/`** — pages, one typed fetch wrapper per endpoint in `lib/api.ts`.
- **`src-tauri/`** — thin Rust wrapper: engine lifecycle, port reclaim, window.
- **`branding-mod/`** — shared Java source with per-version overlays behind a compatibility shim.

---

## Data directory

`<repo>/data` in development, next to the installed engine in the desktop app. Override with
`ESPECTRAL_DATA_DIR`.

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

Issues and pull requests are welcome. Please run `npm test` and `npm run check` before opening a PR;
CI runs both on Linux and Windows.

## License

GPL-3.0 — see [LICENSE](LICENSE). Bundled third-party mods (Ok Zoomer, Gamma Utils, Clear Fog) keep
their own licenses; fabric-loader is Apache-2.0.
