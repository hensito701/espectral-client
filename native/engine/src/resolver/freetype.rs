//! minecraft2shell: FreeType's TrueType interpreter lets `SHZ` write past a
//! glyph's point buffer (FreeType issue #1419, fixed in FreeType 2.15.0). A
//! font in a server resource pack turns that into native code execution as
//! soon as the game hints one of its glyphs. Every Minecraft since 1.20.5
//! (24w03a) renders TrueType fonts through the FreeType that LWJGL bundles:
//! 2.13.2 (LWJGL 3.3.3, 1.20.5–1.21.11), 2.14.1 (3.4.1, 26.1–26.2) and 2.14.3
//! (3.4.3, 26.3).
//!
//! On 2026-11-24 Mojang republished 26.3 and the 26.4 snapshots with a
//! FreeType 2.15.0 build, `org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0`; the
//! older version JSONs still pin a vulnerable build. [`patch`] gives every
//! version JSON the resolver hands out Mojang's patched natives, and a cached
//! pre-republish 26.3 JSON gets them before its 6 h manifest refresh.
//!
//! Only the natives change. Each version keeps its own Java binding: the
//! patched library exports every symbol the 3.3.3, 3.4.1 and 3.4.3 bindings
//! resolve, and FreeType keeps its public ABI stable within 2.x. Those
//! bindings record the SHA-1 of their original library, so LWJGL prints
//! `Incompatible Java and native library versions detected` once at startup;
//! the message is a diagnostic and the library loads normally.

use std::collections::HashSet;

use serde_json::{json, Value};

/// The `lwjgl-freetype` build that carries the fix.
const PATCHED_VERSION: &str = "3.4.3-mojang-2.15.0";

const LIBRARIES_BASE: &str = "https://libraries.minecraft.net/";

/// Mojang's patched natives jars, `(classifier, sha1, size)`, as listed in the
/// republished 26.3 version JSON.
const PATCHED_NATIVES: [(&str, &str, u64); 5] = [
    ("natives-linux", "f20ae220999240d484896d839694b02c2a655608", 1_423_204),
    ("natives-macos", "6a46815640d9ce43533200986a330642a96e9590", 1_225_020),
    ("natives-macos-arm64", "95ef1d3420179187f0b5e1ba947329b539e73412", 1_125_234),
    ("natives-windows", "f85d0f5acaa594f9e93d657f8feff85d6a427b6a", 1_190_826),
    ("natives-windows-arm64", "5f475c76490bb368f235017985d8d9dc6b5b5121", 1_043_884),
];

/// `major.minor.patch`, missing components read as 0; None when a component
/// is not a number.
fn parse_version(v: &str) -> Option<[u32; 3]> {
    let mut out = [0u32; 3];
    let mut parts = v.split('.');
    for slot in &mut out {
        if let Some(p) = parts.next() {
            *slot = p.parse().ok()?;
        }
    }
    Some(out)
}

/// True when this `lwjgl-freetype` version bundles a FreeType older than
/// 2.15.0: every LWJGL release up to 3.4.3, and Mojang rebuilds
/// (`<lwjgl>-mojang-<freetype>`) of an older FreeType. A version that does not
/// parse counts as vulnerable.
fn bundles_vulnerable_freetype(version: &str) -> bool {
    if let Some((_, freetype)) = version.split_once("-mojang-") {
        return parse_version(freetype).is_none_or(|v| v < [2, 15, 0]);
    }
    let lwjgl = version.split('-').next().unwrap_or(version);
    parse_version(lwjgl).is_none_or(|v| v < [3, 4, 4])
}

/// The natives classifier of a vulnerable `org.lwjgl:lwjgl-freetype` natives
/// entry, None for anything else.
fn vulnerable_natives(lib: &Value) -> Option<&str> {
    let name = lib.get("name")?.as_str()?;
    let mut parts = name.split(':');
    let (group, artifact, version, classifier) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (group == "org.lwjgl"
        && artifact == "lwjgl-freetype"
        && classifier.starts_with("natives")
        && bundles_vulnerable_freetype(version))
    .then_some(classifier)
}

/// The patched jar for a natives classifier. 1.20.5–1.21.11 ship Intel macOS
/// as Mojang's `natives-macos-patch`; there is no patched 32-bit Windows build.
fn patched_build(classifier: &str) -> Option<(&'static str, &'static str, u64)> {
    let wanted = if classifier == "natives-macos-patch" { "natives-macos" } else { classifier };
    PATCHED_NATIVES.iter().copied().find(|(c, _, _)| *c == wanted)
}

/// Point every vulnerable `lwjgl-freetype` natives entry at Mojang's patched
/// build (rules kept). An entry with no patched build is removed, so a JVM on
/// that platform fails to load FreeType instead of loading the vulnerable one.
pub(crate) fn patch(version_json: &mut Value) {
    let Some(Value::Array(libs)) = version_json.get_mut("libraries") else { return };
    if !libs.iter().any(|l| vulnerable_natives(l).is_some()) {
        return;
    }
    let mut names: HashSet<String> =
        libs.iter().filter_map(|l| l.get("name").and_then(Value::as_str)).map(str::to_string).collect();
    libs.retain_mut(|lib| {
        let Some(classifier) = vulnerable_natives(lib) else { return true };
        let Some((classifier, sha1, size)) = patched_build(classifier) else { return false };
        let name = format!("org.lwjgl:lwjgl-freetype:{PATCHED_VERSION}:{classifier}");
        if !names.insert(name.clone()) {
            return false;
        }
        let path = crate::rules::maven_path(&name);
        let url = format!("{LIBRARIES_BASE}{path}");
        lib["downloads"] = json!({ "artifact": { "path": path, "sha1": sha1, "size": size, "url": url } });
        lib["name"] = Value::String(name);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn natives_of(name: &str, os: &str) -> Value {
        let path = crate::rules::maven_path(name);
        json!({
            "name": name,
            "downloads": { "artifact": { "path": path, "sha1": "0000", "size": 1, "url": format!("{LIBRARIES_BASE}{path}") } },
            "rules": [{ "action": "allow", "os": { "name": os } }]
        })
    }

    fn natives(version: &str, classifier: &str, os: &str) -> Value {
        natives_of(&format!("org.lwjgl:lwjgl-freetype:{version}:{classifier}"), os)
    }

    fn lib(name: &str) -> Value {
        json!({ "name": name, "downloads": { "artifact": { "sha1": "1111" } } })
    }

    fn names(v: &Value) -> Vec<String> {
        v["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_string()).collect()
    }

    /// The 1.21.11 / 1.21.1 shape (LWJGL 3.3.3).
    fn mc_1_21_11() -> Value {
        json!({ "libraries": [
            lib("org.lwjgl:lwjgl-freetype:3.3.3"),
            natives("3.3.3", "natives-linux", "linux"),
            natives("3.3.3", "natives-macos-arm64", "osx"),
            natives("3.3.3", "natives-macos-patch", "osx"),
            natives("3.3.3", "natives-windows", "windows"),
            natives("3.3.3", "natives-windows-arm64", "windows"),
            natives("3.3.3", "natives-windows-x86", "windows"),
            natives_of("org.lwjgl:lwjgl:3.3.3:natives-linux", "linux"),
        ]})
    }

    #[test]
    fn vulnerable_natives_move_to_the_patched_build() {
        let mut v = mc_1_21_11();
        patch(&mut v);
        assert_eq!(
            names(&v),
            [
                "org.lwjgl:lwjgl-freetype:3.3.3",
                "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-linux",
                "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-macos-arm64",
                "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-macos",
                "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-windows",
                "org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-windows-arm64",
                "org.lwjgl:lwjgl:3.3.3:natives-linux",
            ]
        );
        let windows = &v["libraries"][4];
        assert_eq!(
            windows["downloads"]["artifact"],
            json!({
                "path": "org/lwjgl/lwjgl-freetype/3.4.3-mojang-2.15.0/lwjgl-freetype-3.4.3-mojang-2.15.0-natives-windows.jar",
                "sha1": "f85d0f5acaa594f9e93d657f8feff85d6a427b6a",
                "size": 1_190_826,
                "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl-freetype/3.4.3-mojang-2.15.0/lwjgl-freetype-3.4.3-mojang-2.15.0-natives-windows.jar"
            })
        );
        assert_eq!(windows["rules"], json!([{ "action": "allow", "os": { "name": "windows" } }]));
        // The Java binding and other LWJGL natives are untouched.
        assert_eq!(v["libraries"][0], lib("org.lwjgl:lwjgl-freetype:3.3.3"));
        assert_eq!(v["libraries"][6]["downloads"]["artifact"]["sha1"], "0000");
    }

    #[test]
    fn patched_and_newer_builds_are_left_alone() {
        for version in ["3.4.3-mojang-2.15.0", "3.4.3-mojang-2.16.1", "3.4.4", "3.5.0"] {
            let original = json!({ "libraries": [natives(version, "natives-windows", "windows")] });
            let mut v = original.clone();
            patch(&mut v);
            assert_eq!(v, original, "{version}");
        }
    }

    #[test]
    fn every_older_build_counts_as_vulnerable() {
        for version in ["3.3.2", "3.3.3", "3.4.1", "3.4.3", "3.4.3-snapshot", "3.4.3-mojang-2.14.9", "3.4"] {
            assert!(bundles_vulnerable_freetype(version), "{version}");
        }
        assert!(bundles_vulnerable_freetype("nightly"));
    }

    #[test]
    fn patching_twice_changes_nothing_more() {
        let mut once = mc_1_21_11();
        patch(&mut once);
        let mut twice = once.clone();
        patch(&mut twice);
        assert_eq!(once, twice);
    }

    #[test]
    fn an_existing_patched_entry_is_not_duplicated() {
        let mut v = json!({ "libraries": [
            natives(PATCHED_VERSION, "natives-linux", "linux"),
            natives("3.4.3", "natives-linux", "linux"),
        ]});
        patch(&mut v);
        assert_eq!(names(&v), ["org.lwjgl:lwjgl-freetype:3.4.3-mojang-2.15.0:natives-linux"]);
    }
}
