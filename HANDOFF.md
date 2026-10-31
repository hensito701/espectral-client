# Handoff: replace the Tauri/Node launcher with the Rust launcher (clean cutover)

Left by the Windows-side agent (devWIN2, `E:\NPv2\espectrallauncher`) on
2026-10-31. Pick this up in this checkout (`main` = `19b75d3`, clean). Delete
this file when the work is merged.

## Decision (made by the user)

- **Full replacement, clean cutover in one release** (proposed: **1.7.0**).
  The Rust app (`native/`) becomes the only launcher. `src/ui`, `src-tauri`,
  the Node engine and the JS tests go away in the same release — no
  side-by-side period, no shims.
- The work happens **here on devLIN**. Windows-only steps (signing,
  publishing, the real upgrade test) go back to the Windows machine (see
  "What must come back to Windows").

## Where things stand

- The user ran the Rust app on Windows with their real data
  (`%LOCALAPPDATA%\espectral-client`), built with
  `cargo build --release -p espectral-client --target x86_64-pc-windows-gnu -j 2`
  (22 min in WSL; only system DLL imports) and staged by hand as
  `%LOCALAPPDATA%\Espectral Client Rust\espectral-client.exe` + `branding\`
  (copy of `assets/branding`). Their verdict: "everything seems to work fine".
- **Not exercised on Windows yet:** stopping a game from the launcher,
  adopting a running game after a launcher restart, Discord Rich Presence,
  Microsoft device-code login, the update flow (doesn't exist yet), the
  agent MCP mode. `native/PORTING.md` "Known risks" lists the Windows code
  paths that had never run.
- `native/app/UI-PORTING.md` §12: everything is done except
  **Updater (lib/updater): missing** (`Shell.update-*` placeholders only).

## Work items

### 1. In-app updater (Rust) — blocking

Must be drop-in compatible with the existing update channel:

- Endpoint: `https://espectral.uhce.top/client/latest.json` (same as
  `src-tauri/tauri.conf.json` → `plugins.updater.endpoints`). It is the
  Tauri updater format (`version`, `notes`, `pub_date`,
  `platforms.{windows-x86_64,linux-x86_64}.{url,signature}`), written by
  `scripts/publish-release.mjs` (Windows, MERGES into `latest-tauri.json`)
  and `scripts/publish-linux-deb.mjs` (Linux). Keep that format so the same
  feed serves both the last Tauri clients and the new Rust clients.
- Verify the **minisign** signature against the **same public key** as
  `tauri.conf.json` → `plugins.updater.pubkey` (base64 of a minisign public
  key, key id `B26697BC340F675C`). Never install an unverified file.
- Windows: download the setup.exe, verify, run it, exit the app. Linux:
  mirror what the Tauri updater does for .deb today (read
  `tauri-plugin-updater` 2.10.1 source, the version in `src-tauri/Cargo.lock`).
- Wire `Shell.update-status/update-version/update-progress/install-update()`
  in the UI (ES + EN strings). Behaviour should match the Svelte
  `src/ui/src/lib/updater*` flow (read it before deleting it).

### 2. Windows installer for the Rust app — blocking, highest risk

Existing players run the Tauri 1.6.x launcher. **Their Tauri updater is the
only way they move to Rust**: it downloads the signed `setup.exe` from
`latest.json` and runs it. So the 1.7.0 `setup.exe` must:

- Be an NSIS installer (`makensis` is available on Linux) that accepts the
  arguments the Tauri 2.x updater passes when it launches an NSIS update.
  **Check them in `tauri-plugin-updater` 2.10.1 source** (install mode
  passive/quiet flags, `/UPDATE`, relaunch) — don't guess.
- Install per-user into the **same folder** as today:
  `%LOCALAPPDATA%\Espectral Client\` (`productName` "Espectral Client",
  identifier `es.spectral.launcher`, NSIS currentUser). Replace
  `espectral-client.exe`, stage `branding\` next to it (the binary looks in
  `<exe>/branding` or `<exe>/resources/branding`, else falls back to the
  compile-time repo path — PORTING.md "Known risks"), and **remove the Tauri
  leftovers** (bundled `node-bin\`, `engine\`, built UI, any other Tauri
  resource dirs — list them from `scripts/stage-tauri-resources.mjs` and the
  `bundle.resources` in `tauri.conf.json`).
- Keep the Start-menu/desktop shortcut and the Apps & Features uninstall
  entry working (same registry key the Tauri NSIS used, so there is one
  entry, not two). The uninstaller must not delete user data
  (`%LOCALAPPDATA%\espectral-client`).
- Close a running launcher before replacing the exe; never kill a running
  Minecraft (the game is designed to outlive the launcher).
- No WebView2 bootstrapper any more (`webviewInstallMode` goes away).
- Signature: produce `setup.exe.sig` in the same minisign format as
  `tauri signer` (the build scripts read the private key from the gitignored
  `.keys/espectral.key`; **the key is NOT on devLIN** — leave signing to the
  Windows build box, see below).

### 3. Linux .deb for the Rust app

Replace `scripts/build-linux-deb.sh` (today: `tauri build` .deb). Same
package name/paths as the Tauri .deb so `apt`/the updater upgrade it in
place; ship `branding/`; desktop file + icon.

### 4. Build / publish scripts and CI

- `scripts/build-windows-exe.sh` → build the Rust app for
  `x86_64-pc-windows-gnu` (`-j 2`, peak ~13 GB), stage branding, run
  makensis, sign. Keep output paths/filenames that
  `scripts/publish-release.mjs` expects, or update that script; drop its
  bundled-Node guardrail (step 2 in its header) since there is no Node.
- `scripts/publish-linux-deb.mjs` → point at the new .deb.
- Remove `scripts/stage-tauri-resources.mjs`, `build-ui.mjs`,
  `free-port.mjs` if nothing else uses them.
- `.github/workflows/ci.yml`: replace `npm ci / npm test / npm run check /
  npm run build` with `cargo test --workspace` + `cargo clippy` (+ the
  windows-gnu check from PORTING.md "Verify").
  `.github/workflows/release.yml` uses `tauri-apps/tauri-action` — rewrite or
  remove it (releases are actually built locally, see above).
- `package.json`: drop the UI/Tauri/engine scripts and deps (svelte, vite,
  three, @tauri-apps/*, …). Keep what `deploy:mod`
  (`scripts/deploy-mod.mjs`) and the protected script below need. Version
  bump 1.7.0 in `native/` crates, `package.json`, and wherever the Rust app
  reads its version for the updater.

### 5. Delete the old launcher

`src/ui/`, `src-tauri/`, `src/engine/` (except below), `tests/*.test.mjs`
for deleted code, `src/engine/mcp.mjs` (Rust `--mcp` replaces it).

**Protected — do not break:** `scripts/import-lunar-account.mjs` must never
be deleted or modified (user's account-import exception path; the Lunar MSA
token chain must keep working). It imports
`src/engine/accounts.mjs` → `src/engine/config.mjs`, `src/engine/error.mjs`.
**Keep those three files** (and check `node scripts/import-lunar-account.mjs`
still runs against the data dir; the Rust engine reads the same
`config.json` accounts, so the imported account must show up in the Rust
app).

**Keep:** `branding-mod/`, `branding-mod-nf/`, `assets/branding/*.jar`
(committed on purpose; CI and packaging read them), `scripts/deploy-mod.mjs`
(update its "copy into the installed client" target to the new layout).

### 6. Docs

CHANGELOG `## [1.7.0]` entry in plain user terms (new launcher, lower RAM,
no Node/WebView2; agent-access MCP configs must be re-copied from Settings
because the command is now `espectral-client.exe --mcp` instead of
`node-bin\node.exe …\mcp.mjs`). Update README, `native/PORTING.md`
(the "reference implementation, never modify `src/engine`" rule no longer
applies after the cutover), `native/app/UI-PORTING.md`. The Windows checkout
has a gitignored `AGENTS.md`; tell the user which sections need rewriting
(commands table, gotchas 1/2/5/6/7, verification recipe) — don't try to
edit it from here.

## Status after devLIN (2026-10-31) — items 1–6 done

- **1 Updater**: `native/engine/src/updater.rs` (feed + minisign check with
  `minisign-verify`/`semver`, the crates the Tauri updater used) and
  `native/app/src/updater.rs` (first check 30 s after start, re-check every
  6 h, CTA progress, ES/EN error toast). Test feed override:
  `ESPECTRAL_UPDATE_ENDPOINT=<url>` (the key is fixed). Only copies installed
  by the installer (`uninstall.exe` next to the exe) or the .deb check.
  Verified here: the live 1.6.3 Windows/Linux signatures verify, tampered
  bytes are refused, a prod-signed .deb installed through the updater path
  (`pkexec dpkg -i`), CTA "Actualizar a 9.9.9" shown from a local test feed.
- **2 Installer**: `native/packaging/windows/installer.nsi` (stock NSIS, no
  nsis_tauri_utils). Same folder, same `HKCU\…\Uninstall\Espectral Client`
  key, same shortcuts, `.mrpack` association, deletes `node-bin\ engine\
  node_modules\ package.json WebView2Loader.dll` and
  `%LOCALAPPDATA%\es.spectral.launcher` (Tauri WebView2 profile); never touches
  `%LOCALAPPDATA%\espectral-client` or java. Tested under Wine only (fresh,
  simulated 1.6.3 upgrade with `/P /R /UPDATE /ARGS`, uninstall).
- **3 .deb**: `scripts/build-linux-deb.sh` → `build/linux/espectral-client_<ver>_amd64.deb`
  (16 MB vs 54 MB). Verified on devLIN: dpkg upgrade 1.6.3 (Tauri) → 1.7.0
  removes engine/node-bin/node_modules, the installed app finds
  `/usr/lib/Espectral Client/branding`, re-seeds the mod jar and launches 26.3.
- **4** `scripts/build-windows-exe.sh` (→ `build/windows/EspectralClient_<ver>_x64-setup.exe`),
  `scripts/sign-update.sh` (tauri-signer-compatible `.sig` via the minisign
  CLI), CI = cargo test/clippy + windows-gnu check + engine tests on Windows +
  `node --test`; `release.yml` removed; version 1.7.0.
- **5/6** old launcher deleted (`src/engine/{accounts,config,error}.mjs` kept
  for the Lunar import script, verified working); CHANGELOG/README/PORTING/
  UI-PORTING updated. AGENTS.md checklist: see "AGENTS.md (Windows)" below.

## What must come back to Windows — exact steps

The publish scripts are gitignored and were not on devLIN, so they still
point at Tauri outputs. Update them first:

- `scripts/publish-release.mjs`: take
  `build/windows/EspectralClient_<ver>_x64-setup.exe` + `.sig` (it may keep
  renaming the upload to `EspectralClient-<ver>-x64-setup.exe`); drop the
  bundled-Node guardrail (header step 2) — there is no Node in the bundle.
  It still MERGES the `windows-x86_64` entry into `latest-tauri.json` /
  `latest.json` (format unchanged).
- `scripts/publish-linux-deb.mjs`: take `build/linux/espectral-client_<ver>_amd64.deb` + `.sig`.

### 1. Build + sign (WSL on devWIN2)

```bash
git pull                                    # this commit
rustup target add x86_64-pc-windows-gnu     # once
sudo apt-get install -y minisign gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64 nsis   # once (or the ~/mingw ~/nsis trees)
scripts/build-windows-exe.sh                # cargo -j 2 (~13 GB peak, ~22 min) + makensis + .sig from .keys/espectral.key
sha256sum build/windows/EspectralClient_1.7.0_x64-setup.exe
```

Linux .deb: build it on the OLDEST supported distro (Ubuntu 22.04 / Debian 12
userland, e.g. a WSL distro or container of that release) —
`scripts/build-linux-deb.sh` prints the binary's glibc floor and warns above
2.36. A .deb built on devLIN (Fedora 44) needs glibc 2.43: do not publish it.

### 2. Upgrade test on the user's PC (before publishing)

1. Make sure 1.6.3 (Tauri) is installed and has been opened once (log in,
   instances visible). Note: shortcuts, Apps & Features entry, the folder
   `%LOCALAPPDATA%\Espectral Client\` with `node-bin\ engine\ node_modules\`.
2. Start a game from 1.6.3 and leave it running (it must survive).
3. Simulate the Tauri updater exactly (Tauri 1.6.3 cannot be pointed at a
   test feed: its endpoint is compiled in): close nothing, then run
   `EspectralClient_1.7.0_x64-setup.exe /P /R /UPDATE /ARGS`.
   Expect: progress bar only, the old launcher closes, the Rust launcher
   opens by itself, the game is still running.
4. Check: `%LOCALAPPDATA%\Espectral Client\` = `espectral-client.exe`,
   `uninstall.exe`, `branding\` only; `%LOCALAPPDATA%\es.spectral.launcher`
   gone; one Start-menu + one desktop shortcut (still working, Rust icon);
   one Apps & Features entry showing 1.7.0; accounts, instances, settings,
   Discord session intact; double-clicking a `.mrpack` opens the import.
   Also check the shortcut AppUserModelId (Wine could not):
   `Get-StartApps | ? Name -like 'Espectral*'` → AppID `es.spectral.launcher`.
5. Self-update test: bump to a throwaway version, build + sign a second
   installer, serve a test feed, point the installed 1.7.0 at it:
   ```bash
   # WSL, throwaway: don't commit
   sed -i 's/^version = "1.7.0"/version = "1.7.1-rc.1"/' native/Cargo.toml
   scripts/build-windows-exe.sh && git checkout native/Cargo.toml native/Cargo.lock
   mkdir -p /tmp/feed && cp build/windows/EspectralClient_1.7.1-rc.1_x64-setup.exe /tmp/feed/
   cat > /tmp/feed/latest.json <<EOF
   {"version":"1.7.1-rc.1","notes":"test","pub_date":"2026-11-01T00:00:00Z",
    "platforms":{"windows-x86_64":{"url":"http://127.0.0.1:8765/EspectralClient_1.7.1-rc.1_x64-setup.exe",
    "signature":"$(cat build/windows/EspectralClient_1.7.1-rc.1_x64-setup.exe.sig)"}}}
   EOF
   cd /tmp/feed && python3 -m http.server 8765
   ```
   then in PowerShell: close the launcher, `$env:ESPECTRAL_UPDATE_ENDPOINT="http://127.0.0.1:8765/latest.json"; & "$env:LOCALAPPDATA\Espectral Client\espectral-client.exe"`.
   After ~30 s the top bar shows "Actualizar a 1.7.1-rc.1"; click it → progress →
   the launcher closes, the installer runs passive, the launcher reopens and
   Apps & Features shows 1.7.1-rc.1. Then also: edit one byte of the served
   setup.exe, restart, click → a "No se pudo instalar la actualización…" toast
   and nothing installs. Reinstall 1.7.0 afterwards.
6. Fresh install on a clean user (or after uninstalling): GUI installer in
   Spanish and English, finish page "run" + desktop shortcut options.
7. Uninstall from Apps & Features: folder, shortcuts, `.mrpack` association
   and uninstall entry gone; `%LOCALAPPDATA%\espectral-client` intact.
8. Agent access: Settings → Agent access → copy the config; it must say
   `espectral-client.exe --mcp`.

### 3. Publish

Existing release checklist with the updated publish scripts: Windows + Linux
1.7.0, then `curl https://espectral.uhce.top/client/latest.json` shows 1.7.0
with both platforms, and the uploaded setup.exe's sha256 matches step 1. The
last Tauri clients (1.6.x) then upgrade through step 2.3's path.

### AGENTS.md (Windows)

Rewrite these sections of the gitignored `AGENTS.md`: commands table (all
npm/tauri rows → cargo + `scripts/build-windows-exe.sh`, `sign-update.sh`,
`deploy:mod`, `import-lunar-account.mjs`), gotchas 1/2/5/6/7 (no npm deps, no
dev server, i18n tables in `native/app/i18n` instead of svelte-check, outputs
in `native/target` + `build/`, RAM rules for cargo), verification recipe
(`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo check -p espectral-client --target x86_64-pc-windows-gnu`,
`node --test tests/*.test.mjs`), and every path mention of `src/ui`,
`src-tauri`, `src/engine/*.mjs`, `node-bin`, `tauri.conf.json`.

## devLIN limits

14 GB RAM with ~9 GB normally in use (5 GB free when this was written). The
app crate needs ~7.7 GB per rustc for check/clippy and 11.5–12.7 GB for
build/test/release. Use `-p espectral-engine` for engine work; for the app
crate use `-j 1`, `CARGO_INCREMENTAL=0`, one cargo at a time, and ask the user
to free memory if it gets killed. Delete `native/target/*/incremental` when
done.

## Standing user rules

- Don't launch Minecraft or drive the desktop without the user asking.
- Every visible string needs Spanish **and** English.
- Report to the user in plain, Spanish-friendly terms.
- Push to both remotes: `origin` (ovm3) and GitHub
  (`https://github.com/hensito701/espectral-client.git`).
