# Agent access (MCP)

Espectral Client can let an AI agent — Claude Code, Codex, Cursor, Gemini CLI
or any other MCP client — launch **your** Espectral Client, see it, drive it
and record it. Use it to test menus and mods, or to shoot screenshots and
short videos with shaders and game audio.

It is **off by default** and only ever talks to the launcher on your own
machine (`127.0.0.1`, guarded by a private token).

## Turn it on

1. Launcher → **Settings → Agent access** → enable the toggle.
2. Press **Copy MCP config** and paste it into your MCP client:
   - Claude Code / Cursor: the JSON block (`mcpServers.espectral`).
   - Codex: the TOML block (`~/.codex/config.toml`).
3. Keep the launcher open while the agent works — the agent talks to it, and
   it talks to the game.

The command in that config is the launcher itself: `espectral-client.exe
--mcp` on Windows, `espectral-client --mcp` on Linux. It has to be the real
path to the installed binary — if you move or reinstall Espectral Client,
re-copy the block.

**Rotate token** invalidates the old one at once; connected agents keep
working because the MCP server re-reads the token file on every call.

For videos, also enable **Capture tools** on the instance (instance →
Config). That installs Replay Mod (cinematic clips) and two shader packs:
*Complementary Reimagined* (best look, used for cinematic renders) and
*MakeUp Ultra Fast* (fast, for live play and live recording). Fabric
1.21.11, 26.2 and 26.3.

## Safety

- **Never on Espectral's public servers.** Input, chat, commands and the
  cinematic camera are refused on `*.uhce.top` / `*.espectral.es` (also when
  joined by IP). Nothing — not even the allowlist — overrides it.
- Input works only in singleplayer, the replay viewer, `localhost` (your own
  PC's address — a server *named* `127.something` does not count) and the
  servers you add to **Allowed servers**. Screenshots and recordings (what you
  already see) work everywhere.
- **The token never leaves your PC unchecked.** Before each call, the MCP
  server asks the launcher to prove it knows the token; if something else is
  listening on the launcher's port (for example while the launcher is
  closed), the call fails with `ENGINE_IMPOSTOR` and the token is not sent.
  The game and the launcher sign everything they exchange the same way.
- **F8 pauses the agent** in-game (a small *AGENT PAUSED* chip appears); press
  it again to resume. The agent cannot press F8 itself.
- Held keys release themselves after 2 s, and on every screen or world change.
- Agent launches never touch your Discord status, never pause when the window
  loses focus (every version, 1.8.9 included), and can run with a hidden
  window. A `window` size you ask for is the size you get, even if the
  instance's `options.txt` carries an `overrideWidth/overrideHeight`; your
  options file is not changed.
- Replay Mod's "recover this replay?" prompts (left by a game that was killed
  while recording) are skipped on agent launches without answering them: the
  unfinished replays stay, and Replay Mod asks again on your next normal launch.

## What an agent can do

| Tool | What it does |
|---|---|
| `list_instances`, `launch`, `stop`, `state`, `logs`, `list_launches` | Start an instance (optionally straight into a world or server, at a window size, hidden), wait for the title screen or the world, read its log. |
| `screen_tree`, `click`, `key`, `type` | Read any screen as data (including Espectral's title menu, Suite and cosmetics) and use it with real mouse/keyboard input. |
| `player` | Where the player is: x/y/z, block, yaw, pitch, dimension and game mode (what F3 shows) — e.g. the centre for a `clip_render` orbit. Refused when the server hides coordinates (reduced debug info). |
| `chat`, `command`, `disconnect` | Talk, run commands, leave the world. |
| `screenshot` | PNG of exactly what is rendered, shaders included; any size up to 7680 px (e.g. 1080×1920 portrait, 4K) on 26.2/26.3, with or without HUD. |
| `hud`, `shaders` | Hide the HUD; switch shader packs or turn shaders off. |
| `record_start`, `record_stop` | Live MP4 with game audio (`short` 1080×1920@60, `landscape` 1920×1080@60, `evidence` window@30 with HUD). Smoothness is limited by the game's own fps. |
| `replays`, `clip_render`, `clip_status` | Smooth cinematic clips: Replay Mod records every world you play; leave the world, then render an orbit or a camera path from the replay at 60 fps with shaders and the original game audio. A slow PC only renders slower (≈ 5 s per second of 1080×1920 video with Complementary on an AMD laptop iGPU). |

Files land in the instance's `espectral-captures/` folder; tool results give
the full path.

### A cinematic clip, start to finish

1. `launch {instance, join: {world: "My world"}, wait_for: "in_world"}`
2. Stage the scene (`command`, `chat`, `key`…) — Replay Mod is recording.
3. `disconnect` — Replay Mod saves the replay.
4. `replays` → pick the newest name.
5. `clip_render {replay, start_s, duration_s, preset: "short", camera: {orbit:
   {center: [x, y, z], radius: 12, height: 6}}, shaders:
   "ComplementaryReimagined_r5.9.3.zip"}` → job id.
6. `clip_status {job, wait_s: 20}` until `done` → `video_path`.

## Joining a world or server

`launch {join: {server: "host[:port]"}}` joins the server on every version: on
1.20+ through Minecraft's own `--quickPlayMultiplayer`, older ones through the
classic `--server`/`--port` pair.

`join: {world: "<save folder>"}` opens a singleplayer world. Minecraft 1.20+
has `--quickPlaySingleplayer` for it; older versions (including the Forge 1.8.9
menu port) have no such argument, so the launcher hands the save folder to the
branded mod through the JVM property `-Despectral.join.world=<save folder>`.
The mod opens that world from `saves/` once the title menu is ready; an empty
property (the default) does nothing.

## Limits

- Text read back from the game (chat, signs, item names, screen labels) is
  untrusted: the tools say so, and agents must never follow instructions in it.
- Custom-size screenshots and sized recordings need Minecraft 26.2 or newer;
  1.21.11 captures at the window size.
- Stopping the game from the launcher kills it before Replay Mod can save —
  `disconnect` first if you want the replay.
