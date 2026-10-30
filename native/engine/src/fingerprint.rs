//! Port of `src/engine/fingerprint.mjs` — the launch "resolved" fingerprint.
//!
//! `<data>/instances/<name>/resolved.json` records what a verified launch
//! resolved (`expected`: version, loader, asset index, client sha1, library
//! set, natives count, fabric libs) plus cheap sentinel stats of the client
//! jar, the asset index and the natives dir (`files`). A later launch whose
//! expected block is byte-identical (JSON string comparison, like the JS) and
//! whose sentinels are fresh skips the full library/asset verification.
//!
//! The document format is unchanged so JS-written fingerprints stay valid.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::paths::Paths;
use crate::platform;
use crate::rules;
use crate::types::{FabricProfile, InstanceMeta};

/// One sentinel entry (`{ relPath, size, mtimeMs, isDir?, names? }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentinelFile {
    #[serde(rename = "relPath")]
    pub rel_path: String,
    pub size: u64,
    #[serde(rename = "mtimeMs")]
    pub mtime_ms: f64,
    #[serde(rename = "isDir", default, skip_serializing_if = "Option::is_none")]
    pub is_dir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<String>>,
}

/// checkFingerprint() result.
#[derive(Debug, Clone, PartialEq)]
pub struct FingerprintCheck {
    pub hit: bool,
    /// 'no fingerprint' | 'manifest changed' | 'no sentinel' | 'sentinel changed' (None on hit)
    pub reason: Option<&'static str>,
    pub fingerprint: Option<Value>,
}

/// Raw stored fingerprint document, if readable.
pub fn read_fingerprint(paths: &Paths, instance_name: &str) -> Option<Value> {
    let s = std::fs::read_to_string(paths.instance_fingerprint(instance_name)).ok()?;
    serde_json::from_str(&s).ok()
}

/// The `expected` block (key order identical to the JS so string comparison
/// against JS-written documents works).
pub fn build_expected(instance: &InstanceMeta, version_json: &Value, fabric: Option<&FabricProfile>) -> Value {
    let mut libraries = Vec::new();
    let mut natives_count = 0u64;
    if let Some(Value::Array(libs)) = version_json.get("libraries") {
        for lib in libs {
            let name = match lib.get("name") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => "undefined".to_string(),
            };
            let parts: Vec<&str> = name.split(':').collect();
            let is_natives = parts.get(3).map(|c| c.starts_with("natives")).unwrap_or(false);
            if !rules::rules_allow(lib) {
                continue;
            }
            if is_natives {
                natives_count += 1;
            } else {
                let mut row = Map::new();
                if let Some(n) = lib.get("name") {
                    row.insert("name".into(), n.clone());
                }
                let sha1 = lib.pointer("/downloads/artifact/sha1").cloned().unwrap_or(Value::Null);
                row.insert("sha1".into(), sha1);
                libraries.push(Value::Object(row));
            }
        }
    }
    let asset_index = match version_json.get("assetIndex") {
        Some(ai) if crate::config::json_truthy(Some(ai)) => {
            let mut m = Map::new();
            if let Some(id) = ai.get("id") {
                m.insert("id".into(), id.clone());
            }
            m.insert("sha1".into(), ai.get("sha1").cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null));
            Value::Object(m)
        }
        _ => Value::Null,
    };
    let client = match version_json.pointer("/downloads/client") {
        Some(c) if crate::config::json_truthy(Some(c)) => {
            json!({ "sha1": c.get("sha1").cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null) })
        }
        _ => Value::Null,
    };
    let fabric_libs: Vec<Value> = fabric
        .map(|f| f.libraries.iter().map(|l| json!({ "relPath": l.rel_path, "sha1": l.sha1 })).collect())
        .unwrap_or_default();
    json!({
        "version": instance.version,
        "loader": instance.loader.as_str(),
        "fabric_loader_version": fabric.map(|f| Value::from(f.loader_version.clone())).unwrap_or(Value::Null),
        "asset_index": asset_index,
        "client": client,
        "libraries": libraries,
        "natives_count": natives_count,
        "fabric_libs": fabric_libs,
    })
}

/// JS `JSON.stringify(a) === JSON.stringify(b)` (order-sensitive).
pub fn expected_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if !a.is_null() && !b.is_null() => {
            serde_json::to_string(a).ok() == serde_json::to_string(b).ok()
        }
        _ => false,
    }
}

/// Natives-dir entries that churn per game boot must not invalidate the
/// fingerprint: our own `.stage-*` dirs and JNA's `jna<digits>.dll(.x)` temp natives.
pub fn stable_natives_entry(name: &str) -> bool {
    if name.starts_with(".stage-") {
        return false;
    }
    if let Some(rest) = name.strip_prefix("jna") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let tail = &rest[digits.len()..];
            if tail == ".dll" || tail == ".dll.x" {
                return false;
            }
        }
    }
    true
}

fn stable_names(dir: &Path) -> Option<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| stable_natives_entry(n))
        .collect();
    names.sort();
    Some(names)
}

/// Sentinel stats for the client jar, asset index and natives dir; None when
/// any is missing (no fingerprint is written then).
pub fn collect_sentinel_stats(
    paths: &Paths,
    instance: &InstanceMeta,
    version_json: &Value,
) -> Option<Vec<SentinelFile>> {
    let mut out = Vec::new();
    let jar = paths.client_jar(&instance.version);
    let st = std::fs::metadata(&jar).ok().filter(|m| m.is_file())?;
    out.push(SentinelFile {
        rel_path: format!("versions/{v}/{v}.jar", v = instance.version),
        size: st.len(),
        mtime_ms: platform::mtime_ms(&st),
        is_dir: None,
        names: None,
    });
    if let Some(id) = version_json.pointer("/assetIndex/id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let st = std::fs::metadata(paths.asset_index(id)).ok().filter(|m| m.is_file())?;
        out.push(SentinelFile {
            rel_path: format!("assets/indexes/{id}.json"),
            size: st.len(),
            mtime_ms: platform::mtime_ms(&st),
            is_dir: None,
            names: None,
        });
    }
    let nd = paths.instance_natives_dir(&instance.name);
    let st = std::fs::metadata(&nd).ok().filter(|m| m.is_dir())?;
    let names = stable_names(&nd)?;
    if names.is_empty() {
        return None;
    }
    out.push(SentinelFile {
        rel_path: format!("instances/{}/natives", instance.name),
        size: 0,
        mtime_ms: platform::mtime_ms(&st),
        is_dir: Some(true),
        names: Some(names),
    });
    Some(out)
}

/// True when every sentinel still matches (size + mtime within 5 s; natives
/// dir by stable entry names).
pub fn is_sentinel_fresh(paths: &Paths, files: &[SentinelFile]) -> bool {
    if files.is_empty() {
        return false;
    }
    for f in files {
        let abs = paths.data_dir().join(&f.rel_path);
        let st = match std::fs::metadata(&abs) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if f.is_dir == Some(true) {
            if !st.is_dir() {
                return false;
            }
            if let Some(names) = &f.names {
                match stable_names(&abs) {
                    Some(now) if &now == names => {}
                    _ => return false,
                }
            } else if (platform::mtime_ms(&st) - f.mtime_ms).abs() > 5000.0 {
                return false;
            }
        } else {
            if st.len() != f.size {
                return false;
            }
            if (platform::mtime_ms(&st) - f.mtime_ms).abs() > 5000.0 {
                return false;
            }
        }
    }
    true
}

/// Compare the stored fingerprint against what the launch would resolve now.
pub fn check_fingerprint(
    paths: &Paths,
    instance: &InstanceMeta,
    version_json: &Value,
    fabric: Option<&FabricProfile>,
) -> FingerprintCheck {
    let miss = |reason| FingerprintCheck { hit: false, reason: Some(reason), fingerprint: None };
    let stored = match read_fingerprint(paths, &instance.name) {
        Some(s) if !s.is_null() => s,
        _ => return miss("no fingerprint"),
    };
    let expected = build_expected(instance, version_json, fabric);
    if !expected_equal(Some(&expected), stored.get("expected")) {
        return miss("manifest changed");
    }
    let files: Vec<SentinelFile> = match stored.get("files") {
        Some(Value::Array(a)) if !a.is_empty() => {
            match a.iter().map(|v| serde_json::from_value(v.clone())).collect::<Result<Vec<_>, _>>() {
                Ok(f) => f,
                Err(_) => return miss("sentinel changed"),
            }
        }
        _ => return miss("no sentinel"),
    };
    if !is_sentinel_fresh(paths, &files) {
        return miss("sentinel changed");
    }
    FingerprintCheck { hit: true, reason: None, fingerprint: Some(stored) }
}

/// Write `resolved.json` after a successful verification. Ok(None) when the
/// sentinels are not all present yet.
pub fn write_fingerprint_file(
    paths: &Paths,
    instance: &InstanceMeta,
    version_json: &Value,
    fabric: Option<&FabricProfile>,
) -> std::io::Result<Option<Value>> {
    let expected = build_expected(instance, version_json, fabric);
    let files = match collect_sentinel_stats(paths, instance, version_json) {
        Some(f) => f,
        None => return Ok(None),
    };
    let doc = json!({
        "version": 1,
        "created_at": platform::now_iso(),
        "instance": { "name": instance.name, "version": instance.version, "loader": instance.loader.as_str() },
        "expected": expected,
        "files": files,
    });
    crate::paths::write_json_atomic(&paths.instance_fingerprint(&instance.name), &doc)?;
    Ok(Some(doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LibraryDownload, Loader};

    fn instance() -> InstanceMeta {
        InstanceMeta { name: "fp".into(), version: "1.21.1".into(), loader: Loader::Fabric, ..Default::default() }
    }

    fn version_json() -> Value {
        json!({
            "assetIndex": { "id": "17", "sha1": "aa" },
            "downloads": { "client": { "sha1": "cc" } },
            "libraries": [
                { "name": "a:b:1", "downloads": { "artifact": { "sha1": "s1" } } },
                { "name": "org.lwjgl:lwjgl:3:natives-linux" },
                { "name": "org.lwjgl:lwjgl:3:natives-windows" },
                { "name": "never:here:1", "rules": [{ "action": "allow", "os": { "name": "nonexistent-os" } }] }
            ]
        })
    }

    fn fabric() -> FabricProfile {
        FabricProfile {
            mc_version: "1.21.1".into(),
            loader_version: "0.16.5".into(),
            loader_maven: None,
            intermediary: None,
            main_class: "K".into(),
            jvm_args: vec![],
            libraries: vec![LibraryDownload {
                name: "n".into(),
                rel_path: "x/y.jar".into(),
                url: "u".into(),
                sha1: None,
                size: None,
            }],
            min_java_version: None,
            library_count: None,
            seeded: None,
            downloaded: None,
        }
    }

    #[test]
    fn expected_block_has_js_key_order_and_counts() {
        let e = build_expected(&instance(), &version_json(), Some(&fabric()));
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(
            s,
            r#"{"version":"1.21.1","loader":"fabric","fabric_loader_version":"0.16.5","asset_index":{"id":"17","sha1":"aa"},"client":{"sha1":"cc"},"libraries":[{"name":"a:b:1","sha1":"s1"}],"natives_count":2,"fabric_libs":[{"relPath":"x/y.jar","sha1":null}]}"#
        );
    }

    #[test]
    fn stable_entries() {
        assert!(!stable_natives_entry(".stage-123"));
        assert!(!stable_natives_entry("jna123.dll"));
        assert!(!stable_natives_entry("jna9.dll.x"));
        assert!(stable_natives_entry("jna.dll"));
        assert!(stable_natives_entry("jnaX1.dll"));
        assert!(stable_natives_entry("lwjgl.dll"));
    }

    #[test]
    fn write_then_check_hits_and_detects_changes() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        let inst = instance();
        let vj = version_json();
        assert_eq!(check_fingerprint(&p, &inst, &vj, None).reason, Some("no fingerprint"));
        // sentinels missing -> nothing written
        assert!(write_fingerprint_file(&p, &inst, &vj, None).unwrap().is_none());
        std::fs::create_dir_all(p.client_jar("1.21.1").parent().unwrap()).unwrap();
        std::fs::write(p.client_jar("1.21.1"), b"jar").unwrap();
        std::fs::create_dir_all(p.asset_index("17").parent().unwrap()).unwrap();
        std::fs::write(p.asset_index("17"), b"{}").unwrap();
        std::fs::create_dir_all(p.instance_natives_dir("fp")).unwrap();
        std::fs::write(p.instance_natives_dir("fp").join("lwjgl.so"), b"x").unwrap();
        assert!(write_fingerprint_file(&p, &inst, &vj, None).unwrap().is_some());
        assert!(check_fingerprint(&p, &inst, &vj, None).hit);
        // volatile JNA natives do not invalidate
        std::fs::write(p.instance_natives_dir("fp").join("jna4242.dll"), b"x").unwrap();
        assert!(check_fingerprint(&p, &inst, &vj, None).hit);
        // a different fabric profile changes the manifest
        assert_eq!(check_fingerprint(&p, &inst, &vj, Some(&fabric())).reason, Some("manifest changed"));
        // a changed client jar is a stale sentinel
        std::fs::write(p.client_jar("1.21.1"), b"jar-but-longer").unwrap();
        assert_eq!(check_fingerprint(&p, &inst, &vj, None).reason, Some("sentinel changed"));
    }
}
