//! Agent access (port of src/engine/agent.mjs, upstream c22767f): the
//! `agent.enabled` / `agent.allowed_servers` config, the rotatable
//! `<data_dir>/agent-token`, and Bearer authentication for `/api/agent/*`.
//!
//! Everything is inert unless `agent.enabled` is set: the agent routes answer
//! 403 `AGENT_DISABLED` and launches are byte-identical to plain ones.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rand::RngCore;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::ConfigStore;
use crate::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// Policy constants
// ---------------------------------------------------------------------------

/// Public Espectral hosts where agent *control* must never work, whatever the
/// allowlist says. Domain suffixes — the mod mirrors this list and also blocks
/// the IPs they resolve to.
pub const BLOCKED_HOSTS: [&str; 2] = ["uhce.top", "espectral.es"];

/// Token file name under the data dir.
pub const TOKEN_FILE: &str = "agent-token";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// `config.agent`, normalized (JS `agentConfig()`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    pub enabled: bool,
    pub allowed_servers: Vec<String>,
}

/// Current config view; defaults on a missing / malformed `agent` object.
pub fn agent_config(config: &ConfigStore) -> AgentConfig {
    config.with(|doc| {
        let raw = doc.get("agent").and_then(Value::as_object);
        AgentConfig {
            enabled: raw.and_then(|a| a.get("enabled")) == Some(&Value::Bool(true)),
            allowed_servers: normalize_allowed_servers(raw.and_then(|a| a.get("allowed_servers"))),
        }
    })
}

static ALLOWED_SERVER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\[[0-9a-f:.]+\]|[a-z0-9._-]+)(:\d{1,5})?$").expect("valid regex"));

/// `host[:port]` entries, lowercased, deduped, order preserved. Invalid
/// entries are dropped rather than failing the whole save (a typo in one row
/// must not discard the rest of the allowlist). Non-arrays -> `[]`.
pub fn normalize_allowed_servers(list: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = list else { return Vec::new() };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let Value::String(s) = item else { continue };
        let lower = s.trim().to_lowercase();
        let v = lower.strip_suffix('.').unwrap_or(&lower);
        if v.is_empty() || v.len() > 255 || !ALLOWED_SERVER_RE.is_match(v) {
            continue;
        }
        if seen.insert(v.to_string()) {
            out.push(v.to_string());
        }
    }
    out
}

/// UI patch for the agent config (`PUT /api/agent-access`); absent = keep.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentAccessPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_servers: Option<Vec<String>>,
}

/// Merge a patch into `config.agent` and persist it (JS `saveAgentConfig`).
/// Unknown keys inside `config.agent` are preserved.
pub fn save_agent_config(config: &ConfigStore, patch: &AgentAccessPatch) -> AgentConfig {
    config.update(|doc| {
        let mut agent: Map<String, Value> = Map::new();
        agent.insert("enabled".into(), Value::Bool(false));
        agent.insert("allowed_servers".into(), Value::Array(Vec::new()));
        if let Some(Value::Object(cur)) = doc.get("agent") {
            for (k, v) in cur {
                agent.insert(k.clone(), v.clone());
            }
        }
        if let Some(enabled) = patch.enabled {
            agent.insert("enabled".into(), Value::Bool(enabled));
        }
        if let Some(list) = &patch.allowed_servers {
            let raw = Value::Array(list.iter().cloned().map(Value::String).collect());
            let norm = normalize_allowed_servers(Some(&raw));
            agent.insert("allowed_servers".into(), Value::Array(norm.into_iter().map(Value::String).collect()));
        }
        doc.insert("agent".into(), Value::Object(agent));
    });
    agent_config(config)
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/// `<data_dir>/agent-token`.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

fn is_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The stored token; `None` when missing, unreadable or malformed. Never
/// logged, never returned to the UI.
pub fn read_token(data_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let t = raw.trim();
    is_token(t).then(|| t.to_string())
}

/// Write a fresh token (`64 hex + "\n"`, 0600 on Unix). Atomic replace: the
/// MCP shim re-reads the file on every call, so a truncate-in-place rotate
/// could be observed as a blank token. Rotating is how the user revokes access.
pub fn write_token(data_dir: &Path) -> EngineResult<String> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    let file = token_path(data_dir);
    std::fs::create_dir_all(data_dir)?;
    let mut suffix = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut suffix);
    let tmp = data_dir.join(format!("{TOKEN_FILE}.{}.{}.tmp", std::process::id(), hex::encode(suffix)));
    let result = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // `mode` only applies on creation and may be masked by the umask;
            // a no-op failure on FAT/exFAT is fine (the token still works).
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &file)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(token)
}

/// The token, creating it on first use (enabling agent access creates it).
pub fn ensure_token(data_dir: &Path) -> EngineResult<String> {
    match read_token(data_dir) {
        Some(t) => Ok(t),
        None => write_token(data_dir),
    }
}

/// Constant-time compare of a presented token against the stored one.
pub fn token_matches(data_dir: &Path, presented: &str) -> bool {
    if !is_token(presented) {
        return false;
    }
    let Some(stored) = read_token(data_dir) else { return false };
    let (a, b) = (stored.as_bytes(), presented.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 403 `AGENT_DISABLED` (the agent API and the mod relay answer it while off).
pub fn disabled_error() -> EngineError {
    EngineError::api(403, "AGENT_DISABLED", "agent access is disabled (enable it in Settings → Agent access)")
}

/// Bearer auth for every `/api/agent/*` route (JS `authenticate`).
/// `AGENT_DISABLED` comes before auth so a disabled engine does not confirm
/// a token's validity. `authorization` is the raw header value, if any.
pub fn authenticate(config: &ConfigStore, data_dir: &Path, authorization: Option<&str>) -> EngineResult<()> {
    if !agent_config(config).enabled {
        return Err(disabled_error());
    }
    let presented = authorization.and_then(|h| {
        let h = h.trim_start();
        let (scheme, rest) = h.split_at(h.find(char::is_whitespace)?);
        scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
    });
    match presented {
        Some(t) if !t.is_empty() && token_matches(data_dir, t) => Ok(()),
        _ => Err(EngineError::api(401, "AGENT_AUTH", "missing or invalid Authorization: Bearer <agent-token>")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allowlist_is_normalized_deduped_and_drops_invalid_rows() {
        let raw =
            json!([" Play.Example.NET. ", "play.example.net", "http://x", "a b", "[::1]:25565", 7, "10.0.0.2:25566"]);
        assert_eq!(normalize_allowed_servers(Some(&raw)), vec!["play.example.net", "[::1]:25565", "10.0.0.2:25566"]);
        assert!(normalize_allowed_servers(Some(&json!("x"))).is_empty());
    }

    #[test]
    fn token_rotation_replaces_the_file_and_old_token_stops_matching() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_token(dir.path()), None);
        let first = ensure_token(dir.path()).unwrap();
        assert_eq!(ensure_token(dir.path()).unwrap(), first);
        let raw = std::fs::read_to_string(token_path(dir.path())).unwrap();
        assert_eq!(raw, format!("{first}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let second = write_token(dir.path()).unwrap();
        assert_ne!(first, second);
        assert!(!token_matches(dir.path(), &first));
        assert!(token_matches(dir.path(), &second));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "no tmp files left: {leftovers:?}");
    }
}
