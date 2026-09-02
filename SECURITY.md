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
- **Minecraft session token.** Kept in memory only, refreshed as needed, never written to disk.
- **No client secrets.** The launcher uses a public application ID for the Microsoft device-code
  flow. There is no secret to leak.

The refresh token is protected by your operating system's file permissions, not by encryption at
rest. Treat the launcher's data directory like any other credential store: anyone with read access
to your user profile can use it.

## Local engine

The launcher's engine listens on `127.0.0.1:4199` only. It requires a custom request header on
cross-origin requests, enforces a Host allowlist, and caps request bodies, so a web page you visit
cannot drive your launcher.

One known limitation: like every Minecraft launcher, the session token is passed to the game as a
command-line argument, where other processes running as your user can read it. This is required by
the game itself.
