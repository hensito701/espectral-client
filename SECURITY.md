# Security Policy

## Reporting a vulnerability

Please do not open a public issue for security problems.

Use GitHub Security Advisories: the repository's **Security** tab →
**Report a vulnerability**. Include what you found, how to reproduce it, and the affected version.

You can expect an acknowledgement within a few days.

## How your credentials are handled

Espectral Client stores account data locally, in the launcher's data directory. Nothing is sent to
any Espectral server as part of signing in — the launcher talks directly to Microsoft and Mojang.

- **Microsoft refresh token.** Persisted in `config.json` so you do not have to sign in again.
- **Minecraft session token.** Kept in memory and in `mc-token-cache.json` (so a restart does not
  refresh again), until it expires or you sign out of that account.
- **No client secrets.** The launcher uses a public application ID for the Microsoft device-code
  flow. There is no secret to leak.

Every credential file is written owner-only (mode 0600) and, on Linux and macOS, the data directory
itself is restricted to your user (0700). On Windows the data directory lives under
`%LOCALAPPDATA%`, which is private to your account. Tokens are not encrypted at rest: anyone who can
act as your user can use them. Signing out removes the account's refresh token, its cached Minecraft
token and its Espectral identity session.

## Local engine

The launcher is a single native process: the engine runs inside it, so there is no IPC surface
between a shell and a background server any more. Two small loopback listeners remain, both bound to
`127.0.0.1` only:

- the in-game mod bridge and the Discord OAuth redirect (`127.0.0.1:4199`, `ESPECTRAL_PORT`
  overrides), guarded by the Host allowlist and a required `x-espectral-client` header on non-GET
  requests. Each game gets a per-launch token through its environment (not its command line); the
  mod signs every request with it and only acts on answers the launcher signs back, so another
  program on the port can neither impersonate the game nor feed it commands. Discord sign-in only
  starts while the launcher itself owns that port;
- the agent API used by `espectral-client --mcp` and by an external MCP client, which additionally
  requires a bearer token from `<data dir>/agent-token` (0600) and is refused entirely while agent
  access is off. `--mcp` first makes the listener prove it knows that token and never sends the
  token to anything that cannot.

Requests are body-capped, so a web page you visit cannot drive your launcher.

One known limitation: like every Minecraft launcher, the session token is passed to the game as a
command-line argument, where other processes can read it (on Linux this includes other users'
processes unless `/proc` is mounted with `hidepid`). This is required by the game itself.
