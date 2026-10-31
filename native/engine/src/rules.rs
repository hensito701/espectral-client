//! Mojang library helpers shared by resolver (game-resolve), fingerprint
//! (foundation) and launch (game-launch): rule evaluation, maven paths,
//! natives classifiers, download-info resolution.
//!
//! Ported from resolver.mjs (`rulesAllow`, `mavenPath`, `nativesClassifier`,
//! `libraryDownload`) into the foundation so fingerprint.rs does not depend
//! on an unported module. game-resolve must USE these rather than re-port them.

use serde_json::{Map, Value};

use crate::types::LibraryDownload;

/// Maven Central (libraries without a download url or base).
pub const MAVEN_CENTRAL: &str = "https://repo1.maven.org/maven2";

/// Platform to evaluate rules against. Defaults to this machine with no
/// launcher features enabled (JS default `{ os: osName(), arch: process.arch, features: {} }`).
#[derive(Debug, Clone)]
pub struct RuleEnv<'a> {
    pub os: &'a str,
    pub arch: &'a str,
    pub features: Option<&'a Map<String, Value>>,
}

impl Default for RuleEnv<'_> {
    fn default() -> Self {
        RuleEnv { os: crate::platform::os_name(), arch: crate::platform::node_arch(), features: None }
    }
}

fn truthy(v: Option<&Value>) -> bool {
    crate::config::json_truthy(v)
}

/// Mojang rule evaluation: no rules -> allowed; otherwise the LAST matching
/// rule wins. A rule matches when its `os.name` / `os.arch` (when present)
/// match and its feature requirements equal the enabled features (truthiness).
pub fn rules_allow_with(lib: &Value, env: &RuleEnv<'_>) -> bool {
    let rules = match lib.get("rules") {
        Some(Value::Array(r)) if !r.is_empty() => r,
        _ => return true,
    };
    let mut allowed = false;
    for rule in rules {
        let mut os_ok = true;
        if let Some(os) = rule.get("os").filter(|o| truthy(Some(o))) {
            let name = os.get("name").and_then(Value::as_str).filter(|s| !s.is_empty());
            let arch = os.get("arch").and_then(Value::as_str).filter(|s| !s.is_empty());
            if let Some(n) = name {
                if n != env.os {
                    os_ok = false;
                }
            }
            if os_ok {
                if let Some(a) = arch {
                    if a != env.arch {
                        os_ok = false;
                    }
                }
            }
        }
        let mut feat_ok = true;
        if let Some(Value::Object(req)) = rule.get("features") {
            for (k, v) in req {
                let enabled = truthy(env.features.and_then(|f| f.get(k)));
                if enabled != truthy(Some(v)) {
                    feat_ok = false;
                    break;
                }
            }
        }
        if os_ok && feat_ok {
            allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
        }
    }
    allowed
}

/// [`rules_allow_with`] for this machine, no features.
pub fn rules_allow(lib: &Value) -> bool {
    rules_allow_with(lib, &RuleEnv::default())
}

/// Maven coordinates -> relative library path (forward slashes).
/// `org.lwjgl:lwjgl:3.3.3:natives-windows` -> `org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar`.
pub fn maven_path(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    let group = parts.first().copied().unwrap_or("");
    let artifact = parts.get(1).copied().unwrap_or("undefined");
    let version = parts.get(2).copied().unwrap_or("undefined");
    let classifier = parts.get(3).copied();
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    format!("{}/{artifact}/{version}/{file}", group.replace('.', "/"))
}

/// Classifier when the 4th coordinate starts with `natives`, else None.
pub fn natives_classifier(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.split(':').collect();
    match parts.get(3) {
        Some(c) if c.starts_with("natives") => Some((*c).to_string()),
        _ => None,
    }
}

/// Concrete download info for a library entry (Mojang or Fabric shape):
/// `downloads.artifact` when present, else `<lib.url>/<relPath>`, else Maven Central.
pub fn library_download(lib: &Value) -> LibraryDownload {
    let name = lib.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let rel_path = maven_path(&name);
    let str_of = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let u64_of = |v: Option<&Value>| v.and_then(Value::as_u64);
    if let Some(a) = lib.get("downloads").and_then(|d| d.get("artifact")).filter(|a| a.is_object()) {
        return LibraryDownload {
            name,
            rel_path,
            url: str_of(a.get("url")).unwrap_or_default(),
            sha1: str_of(a.get("sha1")),
            size: u64_of(a.get("size")),
        };
    }
    if let Some(base) = lib.get("url").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
        return LibraryDownload {
            url: format!("{base}{rel_path}"),
            name,
            rel_path,
            sha1: str_of(lib.get("sha1")),
            size: u64_of(lib.get("size")),
        };
    }
    LibraryDownload {
        url: format!("{MAVEN_CENTRAL}/{rel_path}"),
        name,
        rel_path,
        sha1: str_of(lib.get("sha1")),
        size: u64_of(lib.get("size")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env<'a>(os: &'a str, arch: &'a str, features: Option<&'a Map<String, Value>>) -> RuleEnv<'a> {
        RuleEnv { os, arch, features }
    }

    #[test]
    fn no_rules_always_allowed() {
        assert!(rules_allow(&json!({})));
        assert!(rules_allow(&json!({ "name": "example.jar" })));
        assert!(rules_allow(&json!({ "rules": [] })));
    }

    #[test]
    fn os_name_and_arch() {
        let lib = json!({ "rules": [{ "action": "allow", "os": { "name": "windows" } }] });
        assert!(rules_allow_with(&lib, &env("windows", "x64", None)));
        assert!(!rules_allow_with(&lib, &env("linux", "x64", None)));
        assert!(!rules_allow_with(&lib, &env("osx", "x64", None)));
        let lib = json!({ "rules": [{ "action": "allow", "os": { "arch": "x86_64" } }] });
        assert!(rules_allow_with(&lib, &env("windows", "x86_64", None)));
        assert!(!rules_allow_with(&lib, &env("windows", "arm64", None)));
        let lib = json!({ "rules": [{ "action": "allow", "os": { "name": "windows", "arch": "x86_64" } }] });
        assert!(rules_allow_with(&lib, &env("windows", "x86_64", None)));
        assert!(!rules_allow_with(&lib, &env("windows", "arm64", None)));
        assert!(!rules_allow_with(&lib, &env("linux", "x86_64", None)));
    }

    #[test]
    fn disallow_and_last_match_wins() {
        let lib = json!({ "rules": [{ "action": "disallow", "os": { "name": "osx" } }] });
        assert!(!rules_allow_with(&lib, &env("windows", "x64", None)));
        assert!(!rules_allow_with(&lib, &env("osx", "x64", None)));
        let lib = json!({ "rules": [
            { "action": "disallow", "os": { "name": "osx" } },
            { "action": "allow", "os": { "name": "windows" } }
        ] });
        assert!(rules_allow_with(&lib, &env("windows", "x64", None)));
        assert!(!rules_allow_with(&lib, &env("osx", "x64", None)));
        assert!(!rules_allow_with(&lib, &env("linux", "x64", None)));
    }

    #[test]
    fn features() {
        let demo = json!({ "rules": [{ "action": "allow", "features": { "is_demo_user": true } }] });
        let on = json!({ "is_demo_user": true });
        let empty = Map::new();
        assert!(rules_allow_with(&demo, &env("linux", "x64", on.as_object())));
        assert!(!rules_allow_with(&demo, &env("linux", "x64", Some(&empty))));
        assert!(!rules_allow_with(&demo, &env("linux", "x64", None)));
        let rejects = json!({ "rules": [{ "action": "allow", "features": { "has_custom_resolution": false } }] });
        let custom = json!({ "has_custom_resolution": true });
        assert!(rules_allow_with(&rejects, &env("linux", "x64", Some(&empty))));
        assert!(!rules_allow_with(&rejects, &env("linux", "x64", custom.as_object())));
    }

    #[test]
    fn maven_paths_and_classifiers() {
        assert_eq!(maven_path("org.lwjgl:lwjgl:3.3.3"), "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar");
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows"),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(natives_classifier("org.lwjgl:lwjgl:3.3.3:natives-linux").as_deref(), Some("natives-linux"));
        assert_eq!(natives_classifier("org.lwjgl:lwjgl:3.3.3"), None);
        assert_eq!(natives_classifier("a:b:c:sources"), None);
    }

    #[test]
    fn library_download_shapes() {
        let mojang = json!({ "name": "a.b:c:1", "downloads": { "artifact": { "url": "https://x/c-1.jar", "sha1": "ab", "size": 3 } } });
        let d = library_download(&mojang);
        assert_eq!(
            (d.url.as_str(), d.sha1.as_deref(), d.size, d.rel_path.as_str()),
            ("https://x/c-1.jar", Some("ab"), Some(3), "a/b/c/1/c-1.jar")
        );
        let fabric = json!({ "name": "net.fabricmc:intermediary:1.21", "url": "https://maven.fabricmc.net" });
        assert_eq!(
            library_download(&fabric).url,
            "https://maven.fabricmc.net/net/fabricmc/intermediary/1.21/intermediary-1.21.jar"
        );
        let bare = json!({ "name": "x:y:2", "sha1": "s" });
        let d = library_download(&bare);
        assert_eq!(d.url, "https://repo1.maven.org/maven2/x/y/2/y-2.jar");
        assert_eq!(d.sha1.as_deref(), Some("s"));
    }
}
