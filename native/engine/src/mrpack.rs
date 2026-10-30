//! Port of `src/engine/mrpack.mjs` — Modrinth modpack (.mrpack or unpacked
//! dir) import: index parse, instance creation, background pack-file
//! download (host allowlist / SSRF guard), overrides extraction.
//!
//! **Owner: content.** Status: PORTED.
//!
//! import_mrpack() returns once the instance exists (validated pack, unique
//! name, loader seeded by createInstance); pack files + overrides land in the
//! background and report over `import-progress` / `import-done` — never as an
//! error to the caller. File downloads report `mod-progress` kind 'mrpack'
//! through `HttpClient::download_all`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::download::{DownloadAllOptions, DownloadItem, ItemOutcome};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};
use crate::events::{EngineEvent, ImportDoneEvent, ImportProgressEvent, ProgressChannel, ProgressEvent};
use crate::types::{CreateInstanceRequest, InstanceSummary, Loader};

const OVERRIDES_PREFIX: &str = "overrides/";

/// Hosts a pack index may download from (SSRF guard; every redirect hop is checked).
pub const PACK_FILE_HOSTS: [&str; 4] =
    ["cdn.modrinth.com", "github.com", "raw.githubusercontent.com", "objects.githubusercontent.com"];
const MAX_PACK_FILE_REDIRECTS: usize = 5;

fn bad_path(msg: impl Into<String>) -> EngineError {
    EngineError::bad_request("BAD_PATH", msg)
}

// ---------------------------------------------------------------------------
// Index parsing
// ---------------------------------------------------------------------------

/// The parsed modrinth.index.json.
#[derive(Debug, Clone, PartialEq)]
pub struct MrpackIndex {
    pub name: Option<Value>,
    pub version_id: Option<Value>,
    pub summary: Option<Value>,
    pub dependencies: serde_json::Map<String, Value>,
    pub files: Vec<Value>,
    pub format_version: Option<Value>,
}

impl MrpackIndex {
    fn from_object(o: &serde_json::Map<String, Value>) -> MrpackIndex {
        MrpackIndex {
            name: o.get("name").cloned(),
            version_id: o.get("versionId").cloned(),
            summary: o.get("summary").cloned(),
            dependencies: o.get("dependencies").and_then(Value::as_object).cloned().unwrap_or_default(),
            files: o.get("files").and_then(Value::as_array).cloned().unwrap_or_default(),
            format_version: o.get("formatVersion").cloned(),
        }
    }

    /// `index.name` when it is a string.
    pub fn name_str(&self) -> Option<&str> {
        self.name.as_ref().and_then(Value::as_str)
    }
}

/// Parse modrinth.index.json inside a .mrpack zip (400 NOT_MRPACK, 502 BAD_MRPACK).
pub fn parse_mrpack_index(zip_path: &Path) -> EngineResult<MrpackIndex> {
    let not_mrpack = || {
        EngineError::bad_request(
            "NOT_MRPACK",
            format!("'{}' no es un .mrpack válido (sin modrinth.index.json)", zip_path.display()),
        )
    };
    let buf = crate::ziputil::read_zip_entry(zip_path, "modrinth.index.json").map_err(|_| not_mrpack())?;
    let buf = buf.ok_or_else(not_mrpack)?;
    let bad =
        || EngineError::api(502, "BAD_MRPACK", format!("modrinth.index.json corrupto en '{}'", zip_path.display()));
    match serde_json::from_slice::<Value>(&buf) {
        Ok(Value::Object(o)) => Ok(MrpackIndex::from_object(&o)),
        _ => Err(bad()),
    }
}

/// Parse an unpacked pack folder's modrinth.index.json (502 BAD_MRPACK).
pub fn parse_mrpack_index_from_dir(dir: &Path) -> EngineResult<MrpackIndex> {
    let bad = || EngineError::api(502, "BAD_MRPACK", format!("modrinth.index.json corrupto en '{}'", dir.display()));
    let raw = std::fs::read(dir.join("modrinth.index.json")).map_err(|_| bad())?;
    match serde_json::from_slice::<Value>(&raw) {
        Ok(Value::Object(o)) => Ok(MrpackIndex::from_object(&o)),
        _ => Err(bad()),
    }
}

/// What an import path points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrpackInput {
    Zip(PathBuf),
    Dir(PathBuf),
}

/// A file (any extension) = zip; a folder with modrinth.index.json = dir; a
/// folder with exactly one top-level .mrpack = that zip.
pub fn resolve_mrpack_input(input: &str) -> EngineResult<MrpackInput> {
    if input.trim().is_empty() {
        return Err(bad_path("path is required and must be a non-empty string"));
    }
    let p = PathBuf::from(input);
    let st = match std::fs::metadata(&p) {
        Ok(st) => st,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(EngineError::not_found("FILE_NOT_FOUND", format!("'{input}' no existe")));
        }
        Err(e) => return Err(e.into()),
    };
    if !st.is_dir() {
        return Ok(MrpackInput::Zip(p));
    }
    match std::fs::metadata(p.join("modrinth.index.json")) {
        Ok(_) => return Ok(MrpackInput::Dir(p)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut mrpacks: Vec<String> = std::fs::read_dir(&p)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().ends_with(".mrpack"))
        .collect();
    mrpacks.sort();
    match mrpacks.len() {
        1 => Ok(MrpackInput::Zip(p.join(&mrpacks[0]))),
        0 => Err(EngineError::bad_request(
            "NOT_MRPACK",
            format!("'{input}' es una carpeta sin modrinth.index.json ni archivos .mrpack"),
        )),
        _ => Err(EngineError::bad_request(
            "NOT_MRPACK",
            format!("'{input}' contiene varios archivos .mrpack — indica el archivo exacto"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Pack name -> valid instance name (trim, collapse whitespace, keep
/// `[A-Za-z0-9 ._-]`, strip edge dots, cap 40). Fallback 'Modpack'.
pub fn sanitize_pack_name(name: Option<&str>) -> String {
    let collapsed = name.unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    let kept: String =
        collapsed.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-')).collect();
    let mut cleaned = kept.trim_matches('.').trim().to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        cleaned = "Modpack".into();
    }
    cleaned.chars().take(40).collect()
}

/// sanitizePackName over a JSON value (`String(name ?? '')`).
fn sanitize_pack_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => sanitize_pack_name(None),
        Some(Value::String(s)) => sanitize_pack_name(Some(s)),
        Some(other) => sanitize_pack_name(Some(&other.to_string())),
    }
}

/// First unused variant: base, base-2, base-3, … (case-insensitive).
pub fn unique_instance_name(base: &str, existing: &[String]) -> String {
    let lower: std::collections::HashSet<String> = existing.iter().map(|n| n.to_lowercase()).collect();
    if !lower.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}-{i}")).find(|c| !lower.contains(&c.to_lowercase())).expect("unbounded")
}

/// Existing instance for the same modpack (sanitized, case-insensitive; never on 'Modpack').
pub fn find_existing_pack_instance<'a>(
    existing: &'a [InstanceSummary],
    pack_name: Option<&str>,
) -> Option<&'a InstanceSummary> {
    let target = sanitize_pack_name(pack_name);
    if target == "Modpack" {
        return None;
    }
    existing.iter().find(|s| {
        s.modpack
            .as_deref()
            .map(|m| sanitize_pack_name(Some(m)).to_lowercase() == target.to_lowercase())
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Join `rel` onto `base`, rejecting absolute paths (incl. drive letters),
/// `..` segments and empty input (400 BAD_PATH). `\` is treated as `/`.
pub fn safe_join(base: &Path, rel: &str) -> EngineResult<PathBuf> {
    if rel.is_empty() {
        return Err(bad_path("la ruta dentro del modpack está vacía"));
    }
    let norm = rel.replace('\\', "/");
    let b = norm.as_bytes();
    if norm.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return Err(bad_path(format!("ruta absoluta no permitida dentro del modpack: '{rel}'")));
    }
    let segments: Vec<&str> = norm.split('/').collect();
    if segments.contains(&"..") {
        return Err(bad_path(format!("la ruta '{rel}' se sale del directorio de la instancia")));
    }
    let mut out = base.to_path_buf();
    for s in segments.into_iter().filter(|s| !s.is_empty() && *s != ".") {
        out.push(s);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Pack-file URL validation
// ---------------------------------------------------------------------------

fn no_redirect_client(engine: &Engine) -> reqwest::Client {
    engine
        .content_state()
        .no_redirect_client
        .get_or_init(|| {
            let mut b = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(engine.http().options().user_agent.clone())
                .connect_timeout(std::time::Duration::from_secs(30));
            if !engine.http().options().use_system_proxy {
                b = b.no_proxy();
            }
            b.build().unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone()
}

/// Validate one pack-file URL against the host allowlist, following redirects
/// manually (every hop checked). Returns the final URL.
async fn resolve_pack_file_url(engine: &Engine, raw: &str) -> EngineResult<String> {
    let settings = engine.content_settings();
    let mut url = url::Url::parse(raw).map_err(|_| {
        EngineError::bad_request("BAD_PACK_URL", format!("modpack file url is not a valid URL: '{raw}'"))
    })?;
    let client = no_redirect_client(engine);
    for _ in 0..=MAX_PACK_FILE_REDIRECTS {
        let host = crate::download::url_host(url.as_str());
        let hostname = url.host_str().unwrap_or("").to_string();
        let scheme_ok = url.scheme() == "https" || (settings.pack_file_allow_http && url.scheme() == "http");
        let host_ok = settings.pack_file_hosts.iter().any(|h| *h == hostname || *h == host);
        if !scheme_ok || !host_ok {
            return Err(EngineError::bad_request(
                "PACK_HOST_NOT_ALLOWED",
                format!("modpack file host not allowed: '{hostname}'"),
            ));
        }
        let res =
            client.head(url.clone()).send().await.map_err(|e| EngineError::other(format!("fetch failed: {e}")))?;
        let loc = res.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
        if res.status().is_redirection() {
            if let Some(loc) = loc {
                url = url.join(&loc).map_err(|_| {
                    EngineError::bad_request("BAD_PACK_URL", format!("modpack file url is not a valid URL: '{loc}'"))
                })?;
                continue;
            }
        }
        return Ok(url.to_string());
    }
    Err(EngineError::bad_request(
        "PACK_HOST_NOT_ALLOWED",
        format!("modpack file url exceeded the redirect limit: '{raw}'"),
    ))
}

/// Resolve every pack-file URL (deduplicated, <= 8 concurrent). A failing URL
/// resolves to None and is reported through `failures`.
async fn resolve_pack_file_urls(
    engine: &Engine,
    raws: &[Option<String>],
    failures: &Mutex<Vec<String>>,
) -> Vec<Option<String>> {
    use futures_util::StreamExt;
    let mut unique: Vec<String> = vec![];
    for r in raws.iter().flatten() {
        if !unique.contains(r) {
            unique.push(r.clone());
        }
    }
    let resolved: std::collections::HashMap<String, Result<String, String>> = futures_util::stream::iter(unique)
        .map(|raw| async move {
            let r = resolve_pack_file_url(engine, &raw).await.map_err(|e| e.to_string());
            (raw, r)
        })
        .buffer_unordered(8)
        .collect()
        .await;
    raws.iter()
        .map(|raw| {
            let raw = raw.as_ref()?;
            match resolved.get(raw) {
                Some(Ok(u)) => Some(u.clone()),
                Some(Err(e)) => {
                    failures.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{raw}: {e}"));
                    None
                }
                None => None,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pack files + overrides
// ---------------------------------------------------------------------------

fn import_progress(
    engine: &Engine,
    instance: &str,
    phase: &str,
    index: Option<u64>,
    total: u64,
    filename: Option<String>,
) {
    engine.emit(EngineEvent::ImportProgress(ImportProgressEvent {
        instance: instance.into(),
        phase: phase.into(),
        index,
        total,
        filename,
    }));
}

struct Pending {
    index: usize,
    file: Value,
    dest: PathBuf,
    sha1: Option<String>,
}

/// Install every pack file into the instance game dir. Present files with a
/// matching sha1 are skipped; with `source_dir` (unpacked pack) local copies
/// are used unless their sha1 mismatches. Per-file failures are isolated and
/// returned; only a total failure errors.
pub async fn install_mrpack_files(
    engine: &Engine,
    instance_name: &str,
    files: &[Value],
    source_dir: Option<&Path>,
) -> EngineResult<Vec<String>> {
    let total = files.len() as u64;
    let dest_base = crate::instances::effective_game_dir_by_name(engine, instance_name);
    let json_path = engine.paths().instance_json(instance_name);
    let mut pending: Vec<Pending> = vec![];
    for (i, file) in files.iter().enumerate() {
        let filename = file.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        if filename.is_empty() {
            continue;
        }
        let dest = safe_join(&dest_base, &filename)?;
        if dest == json_path {
            return Err(bad_path(format!("la ruta '{filename}' colisiona con la metadata de la instancia")));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let sha1 = file.get("hashes").and_then(|h| h.get("sha1")).and_then(Value::as_str).map(str::to_string);
        if let Some(expected) = &sha1 {
            if dest.exists() && crate::download::sha1_file(&dest).await.ok().as_ref() == Some(expected) {
                import_progress(engine, instance_name, "files", Some(i as u64), total, Some(filename));
                continue;
            }
        }
        let pend = Pending { index: i, file: file.clone(), dest: dest.clone(), sha1: sha1.clone() };
        if let Some(src) = source_dir.map(|d| d.join(&filename)).filter(|p| p.exists()) {
            if let Some(expected) = &sha1 {
                if crate::download::sha1_file(&src).await.ok().as_ref() != Some(expected) {
                    pending.push(pend);
                    continue;
                }
            }
            if std::fs::copy(&src, &dest).is_err() {
                pending.push(pend);
                continue;
            }
            import_progress(engine, instance_name, "files", Some(i as u64), total, Some(filename));
            continue;
        }
        pending.push(pend);
    }
    if pending.is_empty() {
        return Ok(vec![]);
    }

    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let first_url =
        |p: &Pending| p.file.get("downloads").and_then(|d| d.get(0)).and_then(Value::as_str).map(str::to_string);
    let raws: Vec<Option<String>> = pending.iter().map(first_url).collect();
    let urls = resolve_pack_file_urls(engine, &raws, &failures).await;
    let mut items = vec![];
    for (p, url) in pending.iter().zip(urls) {
        let path = p.file.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        match url {
            Some(u) => items.push(
                DownloadItem::new(u, p.dest.clone())
                    .sha1(p.sha1.clone())
                    .size(p.file.get("fileSize").and_then(Value::as_u64))
                    .event(
                        ProgressChannel::Mod,
                        ProgressEvent::for_instance(instance_name).kind("mrpack").index(p.index as u64).filename(path),
                    ),
            ),
            None if first_url(p).is_none() => {
                failures.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{path}: no download url"));
            }
            None => {}
        }
    }
    let succeeded = Arc::new(Mutex::new(0usize));
    if !items.is_empty() {
        let (eng, name, fails, ok) = (engine.clone(), instance_name.to_string(), failures.clone(), succeeded.clone());
        let on_item = Arc::new(move |o: &ItemOutcome| {
            let ctx = o.item.event.as_ref().map(|e| e.context.clone()).unwrap_or_default();
            if o.ok {
                *ok.lock().unwrap_or_else(|p| p.into_inner()) += 1;
            } else {
                let f = ctx.filename.clone().unwrap_or_else(|| "?".into());
                let err = o.error.clone().unwrap_or_else(|| "unknown error".into());
                fails.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{f}: {err}"));
            }
            let index = ctx.index.as_ref().and_then(Value::as_u64);
            import_progress(&eng, &name, "files", index, total, ctx.filename.clone());
        });
        // Partial failures are collected per item; overrides must still run.
        let _ =
            engine.http().download_all(items, DownloadAllOptions { concurrency: None, on_item: Some(on_item) }).await;
    }
    let failures = failures.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let succeeded = *succeeded.lock().unwrap_or_else(|p| p.into_inner());
    if !failures.is_empty() && succeeded == 0 {
        return Err(EngineError::other(format!("could not download any modpack file: {}", failures[0])));
    }
    if !failures.is_empty() {
        tracing::warn!(
            "[mrpack] {} file(s) failed to download in '{instance_name}': {}",
            failures.len(),
            failures.join("; ")
        );
    }
    Ok(failures)
}

/// Shared override-writing loop (zip + folder sources).
fn write_override_files(
    engine: &Engine,
    instance_name: &str,
    rels: &[String],
    mut write_one: impl FnMut(&str, &Path) -> EngineResult<bool>,
    fail_message: &str,
) -> EngineResult<()> {
    let dest_base = crate::instances::effective_game_dir_by_name(engine, instance_name);
    let json_path = engine.paths().instance_json(instance_name);
    let total = rels.len() as u64;
    let mut written = 0;
    for (index, rel) in rels.iter().enumerate() {
        let dest = safe_join(&dest_base, rel)?;
        if dest == json_path {
            tracing::warn!("[mrpack] override '{rel}' collides with instance metadata; skipped");
            continue;
        }
        match write_one(rel, &dest) {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!("[mrpack] override '{rel}' not written: {e}"),
        }
        import_progress(engine, instance_name, "overrides", Some(index as u64), total, Some(rel.clone()));
    }
    if written == 0 && total > 0 {
        return Err(EngineError::other(fail_message));
    }
    Ok(())
}

/// Extract a .mrpack's `overrides/` tree into the instance game dir.
pub fn extract_overrides(engine: &Engine, zip_path: &Path, instance_name: &str) -> EngineResult<()> {
    let buf = std::fs::read(zip_path)?;
    let entries: Vec<String> = {
        let tmp = crate::ziputil::list_zip_entries(zip_path)?;
        tmp.into_iter()
            .filter(|e| e.starts_with(OVERRIDES_PREFIX) && !e.ends_with('/'))
            .map(|e| e[OVERRIDES_PREFIX.len()..].to_string())
            .filter(|r| !r.is_empty())
            .collect()
    };
    write_override_files(
        engine,
        instance_name,
        &entries,
        |rel, dest| {
            let Some(data) = crate::ziputil::read_zip_entry_from(&buf, &format!("{OVERRIDES_PREFIX}{rel}"))? else {
                return Ok(false);
            };
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(dest, data)?;
            Ok(true)
        },
        "no override could be extracted from the modpack",
    )
}

fn walk_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let ft = e.file_type()?;
        if ft.is_dir() {
            walk_files(root, &e.path(), out)?;
        } else if ft.is_file() {
            if let Ok(rel) = e.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    Ok(())
}

/// Copy an unpacked pack's `overrides/` folder into the instance game dir (missing = no-op).
pub fn copy_overrides_from_dir(engine: &Engine, source_dir: &Path, instance_name: &str) -> EngineResult<()> {
    let root = source_dir.join("overrides");
    let mut rels = vec![];
    match walk_files(&root, &root, &mut rels) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    write_override_files(
        engine,
        instance_name,
        &rels,
        |rel, dest| {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(root.join(rel), dest)?;
            Ok(true)
        },
        "no override could be copied from the modpack",
    )
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// importMrpack() result.
#[derive(Debug, Clone, PartialEq)]
pub struct MrpackImport {
    pub summary: InstanceSummary,
    pub already_exists: bool,
}

/// Import a modpack as a new instance (files land in the background).
pub async fn import_mrpack(
    engine: &Engine,
    file: &str,
    memory_mb: Option<u32>,
    jdk_path_override: Option<String>,
) -> EngineResult<MrpackImport> {
    let resolved = resolve_mrpack_input(file)?;
    let index = match &resolved {
        MrpackInput::Dir(d) => parse_mrpack_index_from_dir(d)?,
        MrpackInput::Zip(f) => parse_mrpack_index(f)?,
    };
    let deps = &index.dependencies;
    // JS truthiness: null / false / "" / 0 mean "not declared".
    let dep = |k: &str| {
        deps.get(k).filter(|v| match v {
            Value::Null | Value::Bool(false) => false,
            Value::String(s) => !s.is_empty(),
            Value::Number(n) => n.as_f64() != Some(0.0),
            _ => true,
        })
    };
    let mc = deps.get("minecraft").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let Some(mc) = mc else {
        return Err(EngineError::bad_request(
            "MC_VERSION_REQUIRED",
            "el modpack no declara una versión de Minecraft compatible",
        ));
    };
    let as_string = |v: &Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let (loader, loader_version) = if let Some(v) = dep("neoforge") {
        (Loader::Neoforge, Some(as_string(v)))
    } else if let Some(v) = dep("fabric-loader").or_else(|| dep("fabric")) {
        (Loader::Fabric, Some(as_string(v)))
    } else if dep("forge").is_some() {
        return Err(EngineError::bad_request(
            "FORGE_UNSUPPORTED",
            "Forge no está soportado; el modpack debe usar NeoForge",
        ));
    } else if dep("quilt-loader").or_else(|| dep("quilt")).is_some() {
        return Err(EngineError::bad_request("QUILT_UNSUPPORTED", "Quilt no está soportado"));
    } else {
        (Loader::Vanilla, None)
    };

    let info = crate::resolver::resolve_version(engine, &mc).await?;
    if !info.supported {
        return Err(EngineError::conflict(
            "JDK_UNSUPPORTED",
            format!("Minecraft {mc} no es compatible con este launcher (requiere JDK {})", info.java_major),
        ));
    }

    let instances = crate::instances::list_instances(engine).await?;
    let pack_name = index.name_str().map(str::to_string);
    if let Some(existing) = find_existing_pack_instance(&instances, pack_name.as_deref()) {
        engine.emit(EngineEvent::ImportDone(ImportDoneEvent {
            instance: existing.name.clone(),
            ok: true,
            error: None,
            already_exists: Some(true),
        }));
        return Ok(MrpackImport { summary: existing.clone(), already_exists: true });
    }
    let names: Vec<String> = instances.iter().map(|i| i.name.clone()).collect();
    let name = unique_instance_name(&sanitize_pack_value(index.name.as_ref()), &names);
    let summary = crate::instances::create_instance(
        engine,
        CreateInstanceRequest {
            name: name.clone(),
            version: mc,
            loader: Some(loader),
            loader_version,
            modpack: pack_name,
            modpack_version: index.version_id.as_ref().and_then(Value::as_str).map(str::to_string),
            memory_mb,
            jdk_path_override,
            ..Default::default()
        },
    )
    .await?;

    let bg = engine.clone();
    let files = index.files.clone();
    engine.spawn_background("mrpack", async move {
        let r = async {
            match &resolved {
                MrpackInput::Dir(d) => {
                    install_mrpack_files(&bg, &name, &files, Some(d)).await?;
                    copy_overrides_from_dir(&bg, d, &name)
                }
                MrpackInput::Zip(f) => {
                    install_mrpack_files(&bg, &name, &files, None).await?;
                    extract_overrides(&bg, f, &name)
                }
            }
        }
        .await;
        let done = match r {
            Ok(()) => ImportDoneEvent { instance: name.clone(), ok: true, error: None, already_exists: None },
            Err(e) => {
                tracing::warn!("[mrpack] background install failed for '{name}': {e}");
                ImportDoneEvent { instance: name.clone(), ok: false, error: Some(e.to_string()), already_exists: None }
            }
        };
        bg.emit(EngineEvent::ImportDone(done));
        Ok(())
    });
    Ok(MrpackImport { summary, already_exists: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixture, temp_engine, MockResponse, MockServer};
    use serde_json::json;

    fn summary(name: &str, modpack: Option<&str>) -> InstanceSummary {
        InstanceSummary {
            name: name.into(),
            version: "1.21.1".into(),
            loader: Loader::Vanilla,
            loader_version: None,
            modpack: modpack.map(str::to_string),
            modpack_version: None,
            memory_mb: 2048,
            memory_auto: true,
            hue: None,
            game_dir: None,
            has_icon: false,
            mod_count: 0,
            enabled_mod_count: 0,
            aot_key: None,
            aot_cache_exists: false,
            imported_from: None,
            created_at: None,
            running: None,
        }
    }

    #[test]
    fn parses_fixture_index_and_rejects_non_mrpacks() {
        let index = parse_mrpack_index(&fixture("mrpack-mini.mrpack")).unwrap();
        assert_eq!(index.name_str(), Some("Pack de Prueba"));
        assert_eq!(index.version_id, Some(json!("1.0")));
        assert_eq!(index.dependencies["minecraft"], "1.21.1");
        assert_eq!(index.dependencies["neoforge"], "21.1.242");
        assert_eq!(index.files.len(), 2);
        assert_eq!(index.files[0]["path"], "mods/prueba-1.0.jar");
        assert!(index.files[0]["hashes"]["sha1"].is_string());
        let pkg = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("package.json");
        assert!(parse_mrpack_index(&pkg).unwrap_err().has_code("NOT_MRPACK"));
        assert!(parse_mrpack_index(&fixture("not-a-zip.bin")).unwrap_err().has_code("NOT_MRPACK"));
    }

    #[test]
    fn sanitize_names() {
        let s = |n: &str| sanitize_pack_name(Some(n));
        assert_eq!(s("Keke Cliente 1.0"), "Keke Cliente 1.0");
        assert_eq!(s("  Keke   Cliente 1.0!! "), "Keke Cliente 1.0");
        assert_eq!(s(""), "Modpack");
        assert_eq!(sanitize_pack_name(None), "Modpack");
        assert_eq!(s(&"x".repeat(60)), "x".repeat(40));
        assert_eq!(s("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(s(".."), "Modpack");
        assert_eq!(s("..."), "Modpack");
        assert_eq!(s(".Keke"), "Keke");
        assert_eq!(s("Keke."), "Keke");
    }

    #[test]
    fn unique_names() {
        assert_eq!(unique_instance_name("Keke", &[]), "Keke");
        assert_eq!(unique_instance_name("Keke", &["keke".into()]), "Keke-2");
        assert_eq!(unique_instance_name("Keke", &["Keke".into(), "Keke-2".into()]), "Keke-3");
    }

    #[test]
    fn safe_join_rules() {
        let base = Path::new("C:/instances/Keke");
        assert_eq!(safe_join(base, "mods/a.jar").unwrap(), base.join("mods").join("a.jar"));
        assert_eq!(safe_join(base, "config/foo/bar.toml").unwrap(), base.join("config").join("foo").join("bar.toml"));
        assert_eq!(safe_join(base, "resourcepacks\\pack.zip").unwrap(), base.join("resourcepacks").join("pack.zip"));
        for bad in ["../evil", "a/../../b", "/abs", "C:/abs", "C:\\abs", "C:rel", "c:foo", "", ".."] {
            assert!(safe_join(base, bad).unwrap_err().has_code("BAD_PATH"), "should reject '{bad}'");
        }
    }

    #[test]
    fn resolve_input_variants() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("single");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.mrpack"), "").unwrap();
        assert_eq!(resolve_mrpack_input(dir.to_str().unwrap()).unwrap(), MrpackInput::Zip(dir.join("pack.mrpack")));
        let unpacked = d.path().join("unpacked");
        std::fs::create_dir_all(&unpacked).unwrap();
        std::fs::write(unpacked.join("modrinth.index.json"), r#"{"formatVersion":1,"name":"P"}"#).unwrap();
        assert_eq!(resolve_mrpack_input(unpacked.to_str().unwrap()).unwrap(), MrpackInput::Dir(unpacked.clone()));
        let neither = d.path().join("neither");
        std::fs::create_dir_all(&neither).unwrap();
        std::fs::write(neither.join("readme.txt"), "hi").unwrap();
        assert!(resolve_mrpack_input(neither.to_str().unwrap()).unwrap_err().has_code("NOT_MRPACK"));
        let two = d.path().join("two");
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(two.join("a.mrpack"), "").unwrap();
        std::fs::write(two.join("b.mrpack"), "").unwrap();
        assert!(resolve_mrpack_input(two.to_str().unwrap()).unwrap_err().has_code("NOT_MRPACK"));
        let missing = d.path().join("no-such-file.mrpack");
        assert!(resolve_mrpack_input(missing.to_str().unwrap()).unwrap_err().has_code("FILE_NOT_FOUND"));
        let f = d.path().join("pack.mrpack");
        std::fs::write(&f, "").unwrap();
        assert_eq!(resolve_mrpack_input(f.to_str().unwrap()).unwrap(), MrpackInput::Zip(f.clone()));
        for bad in ["", "   "] {
            assert!(resolve_mrpack_input(bad).unwrap_err().has_code("BAD_PATH"));
        }
    }

    #[test]
    fn existing_pack_matching() {
        let list = vec![
            summary("Alpha", Some("Keke Cliente 1.0")),
            summary("Beta", Some("Otro Pack")),
            summary("Gamma", None),
        ];
        assert_eq!(find_existing_pack_instance(&list, Some("Keke Cliente 1.0")).unwrap().name, "Alpha");
        assert_eq!(find_existing_pack_instance(&list, Some("keke cliente 1.0")).unwrap().name, "Alpha");
        assert_eq!(find_existing_pack_instance(&list, Some("  Keke   Cliente 1.0!! ")).unwrap().name, "Alpha");
        assert!(find_existing_pack_instance(&list, Some("Nada Que Ver")).is_none());
        assert!(find_existing_pack_instance(&[], Some("Cualquiera")).is_none());
        for bad in [Some(""), Some("!!"), Some("..."), Some("Modpack"), None] {
            assert!(find_existing_pack_instance(&list, bad).is_none(), "{bad:?}");
        }
    }

    fn make_instance(e: &Engine, name: &str) {
        crate::paths::write_json_atomic(&e.paths().instance_json(name), &json!({ "name": name, "version": "1.21.1" }))
            .unwrap();
    }

    #[tokio::test]
    async fn overrides_from_zip_and_dir() {
        let (d, e) = temp_engine();
        make_instance(&e, "pack");
        let mut rx = e.subscribe();
        extract_overrides(&e, &fixture("mrpack-mini.mrpack"), "pack").unwrap();
        let game = e.paths().instance_dir("pack");
        assert!(game.join("config").join("prueba.toml").is_file());
        assert!(game.join("mods").join("override-mod.jar").is_file());
        assert!(game.join("resourcepacks").join("override-pack").join("pack.mcmeta").is_file());
        let mut n = 0;
        while let Ok(EngineEvent::ImportProgress(p)) = rx.try_recv() {
            assert_eq!((p.phase.as_str(), p.total), ("overrides", 3));
            n += 1;
        }
        assert_eq!(n, 3);

        let src = d.path().join("unpacked");
        std::fs::create_dir_all(src.join("overrides").join("config").join("deep")).unwrap();
        std::fs::write(src.join("overrides").join("config").join("deep").join("x.toml"), "x").unwrap();
        make_instance(&e, "pack2");
        copy_overrides_from_dir(&e, &src, "pack2").unwrap();
        assert!(e.paths().instance_dir("pack2").join("config").join("deep").join("x.toml").is_file());
        copy_overrides_from_dir(&e, &d.path().join("none"), "pack2").unwrap();
    }

    #[tokio::test]
    async fn pack_files_download_with_host_guard_and_partial_failure() {
        let server = MockServer::start(|req| match req.path.as_str() {
            "/ok.jar" => MockResponse::ok("good jar"),
            "/redirect-out" => MockResponse::status(302).with_header("location", "http://evil.invalid/x"),
            _ => MockResponse::status(404),
        })
        .await;
        let (d, e) = temp_engine();
        e.update_content_settings(|s| {
            s.pack_file_hosts = vec![server.host()];
            s.pack_file_allow_http = true;
        });
        make_instance(&e, "pf");
        let good = crate::download::sha1_hex(b"good jar");
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("config")).unwrap();
        std::fs::write(src.join("config").join("local.toml"), "local").unwrap();
        let files = vec![
            json!({ "path": "mods/ok.jar", "downloads": [server.url("/ok.jar")], "hashes": { "sha1": good } }),
            json!({ "path": "mods/evil.jar", "downloads": ["https://evil.invalid/x.jar"] }),
            json!({ "path": "mods/redir.jar", "downloads": [server.url("/redirect-out")] }),
            json!({ "path": "mods/none.jar" }),
            json!({ "path": "config/local.toml" }),
            json!({ "downloads": ["x"] }),
        ];
        let failures = install_mrpack_files(&e, "pf", &files, Some(&src)).await.unwrap();
        let game = e.paths().instance_dir("pf");
        assert_eq!(std::fs::read_to_string(game.join("mods").join("ok.jar")).unwrap(), "good jar");
        assert_eq!(std::fs::read_to_string(game.join("config").join("local.toml")).unwrap(), "local");
        assert_eq!(failures.len(), 3, "{failures:?}");
        assert!(failures.iter().any(|f| f.contains("PACK_HOST_NOT_ALLOWED") || f.contains("not allowed")));
        assert!(failures.iter().any(|f| f.contains("no download url")));
        // already present with a matching sha1 -> skipped, nothing downloaded
        let hits = server.hits();
        install_mrpack_files(&e, "pf", &files[..1], None).await.unwrap();
        assert_eq!(server.hits(), hits);
        // every file failing is an error; escapes are rejected
        let err = install_mrpack_files(&e, "pf", &files[1..2], None).await.unwrap_err();
        assert!(err.to_string().contains("could not download any modpack file"));
        let esc = vec![json!({ "path": "../escape.jar", "downloads": ["x"] })];
        assert!(install_mrpack_files(&e, "pf", &esc, None).await.unwrap_err().has_code("BAD_PATH"));
        let meta = vec![json!({ "path": "instance.json" })];
        assert!(install_mrpack_files(&e, "pf", &meta, None).await.unwrap_err().has_code("BAD_PATH"));
    }

    #[tokio::test]
    async fn import_mrpack_validates_before_resolving() {
        let (d, e) = temp_engine();
        let dir = d.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |deps: Value| {
            std::fs::write(dir.join("modrinth.index.json"), json!({ "name": "P", "dependencies": deps }).to_string())
                .unwrap()
        };
        let path = dir.to_string_lossy().into_owned();
        write(json!({}));
        assert!(import_mrpack(&e, &path, None, None).await.unwrap_err().has_code("MC_VERSION_REQUIRED"));
        write(json!({ "minecraft": "1.20.1", "forge": "47" }));
        assert!(import_mrpack(&e, &path, None, None).await.unwrap_err().has_code("FORGE_UNSUPPORTED"));
        write(json!({ "minecraft": "1.20.1", "quilt-loader": "0.2" }));
        assert!(import_mrpack(&e, &path, None, None).await.unwrap_err().has_code("QUILT_UNSUPPORTED"));
        std::fs::write(dir.join("modrinth.index.json"), "[1]").unwrap();
        assert!(import_mrpack(&e, &path, None, None).await.unwrap_err().has_code("BAD_MRPACK"));
    }
}
