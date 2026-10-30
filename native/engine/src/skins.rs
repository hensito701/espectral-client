//! Port of `src/engine/skins.mjs` — skin texture cache (`<data>/skins`), skin
//! gallery (`<data>/skin-library` + `skin-library.json`), vanilla launcher
//! skin import, skin-head avatar derivation (PNG decode/encode with flate2).
//!
//! **Owner: accounts-skins.** Status: STUB.
//!
//! JS exports to port: SKIN_VARIANTS skinsDir libraryDir libraryIndexPath
//! skinPngDimensions isValidSkinPng assertSkinVariant decodeSkinPng
//! readSkinCache writeSkinCache clearSkinCache resolveSkinAccount
//! loadLibraryIndex publicSkinEntry saveLibrarySkin getLibraryEntry
//! updateLibrarySkin deleteLibrarySkin readLibraryPng defaultVanillaSkinsPath
//! importVanillaSkins extractHeadPng shouldSyncAvatarFromSkin
//! syncAccountAvatarFromSkin clearAccountAutoAvatar.
//! Tests: tests/skins.test.mjs, skin-avatar.test.mjs, skin.test.mjs.
//! PNG upload validation: crate::util::decode_png_image (foundation).
//!
//! The 3D skin viewer stays in the UI (three.js in a small webview or a
//! native renderer — UI phase decision); the engine only serves PNG bytes.
