//! Port of `src/engine/mrpack.mjs` — Modrinth modpack (.mrpack or unpacked
//! dir) import: index parse, instance creation, background pack-file
//! download (host allowlist / SSRF guard), overrides extraction.
//!
//! **Owner: content.** Status: STUB.
//!
//! JS exports to port: parseMrpackIndex resolveMrpackInput
//! parseMrpackIndexFromDir sanitizePackName uniqueInstanceName
//! findExistingPackInstance safeJoin installMrpackFiles extractOverrides
//! copyOverridesFromDir importMrpack. Tests: tests/mrpack.test.mjs
//! (fixture: mrpack-mini.mrpack). Zip access: crate::ziputil.
//! Events: import-progress / import-done; file downloads report
//! mod-progress kind 'mrpack' through HttpClient::download_all.
