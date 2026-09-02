# Bring your own Microsoft client ID (power users)

Espectral Client signs Microsoft accounts in via the OAuth2 **device-code flow**, which needs an
Azure application (client ID). The launcher ships with Espectral's own registration baked in
(`071ffa4b-eafb-4b7a-aa71-d8beef4f4f2e` — a public client ID, not a secret). General sign-in with
that default ID activates once Mojang approves it ([aka.ms/mce-reviewappid](https://aka.ms/mce-reviewappid));
until then the device-code endpoint rejects it (`AADSTS700016`) and the UI shows a
"pending Mojang approval" banner.

The supported workaround is the standard open-source-launcher pattern: **register your own Azure
app and give the launcher its client ID**. Each community member can do this with their own
Microsoft account.

## 1. Register an Azure app

1. Go to [portal.azure.com](https://portal.azure.com) → **App registrations** → **New registration**.
2. Name it anything (e.g. "My Minecraft Launcher").
3. Supported account types: **Personal Microsoft accounts only**.
4. After creating it, open **Authentication** → add a **Mobile & desktop applications** platform
   (no redirect URI is required for device code; no client secret — leave certificates & secrets
   empty).
5. Copy the **Application (client) ID** — a GUID like `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
6. New apps additionally need Mojang's approval for the Minecraft API: submit the client ID at
   [aka.ms/mce-reviewappid](https://aka.ms/mce-reviewappid). Sign-in starts working once your app
   is approved.

## 2. Give the client ID to the launcher

Resolution order (first hit wins — see `clientId()` in `src/engine/msauth.mjs`):

1. **`msa_client_id` in `config.json`** — works everywhere, including the installed desktop app
   (the Tauri shell does not forward arbitrary env vars to the engine). The file lives at
   `<dataDir>/config.json` (`data/config.json` next to the repo in dev, or next to the installed
   engine; `ESPECTRAL_DATA_DIR` overrides the location). Add the key at the top level:

   ```jsonc
   {
     "msa_client_id": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
     // …rest of the config is untouched
   }
   ```

2. **`ESPECTRAL_MSA_CLIENT_ID` environment variable** — convenient for the dev/browser flow:

   ```powershell
   $env:ESPECTRAL_MSA_CLIENT_ID = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
   npm run dev
   ```

3. Baked default (Espectral's app — pending approval).

Then sign in normally from the Account page: the device-code flow now runs against your app.

## Good to know

- **Refresh tokens are bound to the client ID that issued them.** If you switch client IDs later
  (or the default app gets approved and you move back to it), accounts created under the old ID
  can't refresh — remove and re-add them.
- Accounts **imported from Lunar Client** keep refreshing through Lunar's legacy client ID
  regardless of this setting (`microsoft.lunar: true`).
- Never share your refresh token; it is the only durable credential the launcher stores
  (see [SECURITY.md](../SECURITY.md)).
- Your client ID is not a secret — it's fine for it to sit in `config.json`. What must stay
  private is the refresh token Microsoft issues for it.
