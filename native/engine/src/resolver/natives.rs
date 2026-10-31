//! Natives extraction (resolver.mjs `extractNatives`): stage-dir extraction
//! with a timeout + 3 attempts, `extract.exclude` prefixes, freshness markers
//! `.extract-ok-<sha1(jarPath)>` / `.extract-meta-<sha1(jarPath)>`
//! (`{"size":…,"mtimeMs":…}`, Node mtime semantics), and the raw-native-lib
//! fallback (a non-zip "jar" is copied into the natives dir as-is).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::download::sha1_hex;
use crate::error::{EngineError, EngineResult};
use crate::jvm::node_mtime_ms;

static STAGE_SEQ: AtomicU64 = AtomicU64::new(0);

const EXTRACT_TIMEOUT: Duration = Duration::from_secs(60);

/// JS number -> JSON (integers without a trailing `.0`, like JSON.stringify).
pub(crate) fn js_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// mtimeMs equality as JS `===` meant it, tolerant of the last-ULP drift of
/// serde_json's (non-`float_roundtrip`) float parser: 1 µs.
fn same_mtime(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn jar_stat(jar: &Path) -> Option<(u64, f64)> {
    let m = std::fs::metadata(jar).ok()?;
    Some((m.len(), node_mtime_ms(&m)))
}

/// Cheap zip sniff: local-file/empty-zip signatures start with 'PK'.
fn is_zip_file(file: &Path) -> bool {
    use std::io::Read;
    let mut b = [0u8; 4];
    match std::fs::File::open(file) {
        Ok(mut f) => f.read(&mut b).map(|n| n >= 2 && b[0] == 0x50 && b[1] == 0x4b).unwrap_or(false),
        Err(_) => false,
    }
}

fn write_markers(marker: &Path, meta: &Path, stat: Option<(u64, f64)>) {
    if let Some((size, mtime)) = stat {
        let _ = std::fs::write(marker, "");
        let mut m = serde_json::Map::new();
        m.insert("size".into(), Value::from(size));
        m.insert("mtimeMs".into(), js_number(mtime));
        let _ = std::fs::write(meta, Value::Object(m).to_string());
    }
}

fn unzip_into(jar: &Path, stage: &Path) -> EngineResult<()> {
    let f = std::fs::File::open(jar)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| EngineError::other(format!("{}: {e}", jar.display())))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| EngineError::other(format!("{}: {e}", jar.display())))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = stage.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut w)?;
    }
    Ok(())
}

fn retry_on_busy(mut f: impl FnMut() -> std::io::Result<()>, attempts: u32) -> std::io::Result<()> {
    let mut last = None;
    for _ in 0..attempts {
        match f() {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("retry failed")))
}

fn move_staged(dir: &Path, stage: &Path, natives_dir: &Path, exclude: &[String]) -> EngineResult<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let abs = ent.path();
        let rel = abs
            .strip_prefix(stage)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| ent.file_name().to_string_lossy().into_owned());
        let rel_slash = format!("{rel}/");
        let excluded = exclude
            .iter()
            .any(|p| rel.starts_with(p.as_str()) || (p.ends_with('/') && rel_slash.starts_with(p.as_str())));
        if excluded {
            continue;
        }
        let dst = crate::resolver::join_rel(natives_dir, &rel);
        if ent.file_type()?.is_dir() {
            std::fs::create_dir_all(&dst)?;
            move_staged(&abs, stage, natives_dir, exclude)?;
        } else {
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p)?;
            }
            retry_on_busy(
                || {
                    match std::fs::remove_file(&dst) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e),
                    }
                    std::fs::rename(&abs, &dst)
                },
                5,
            )?;
        }
    }
    Ok(())
}

/// Extract one natives jar into `natives_dir` (see module docs).
pub(crate) async fn extract_natives(jar: &Path, natives_dir: &Path, extract_exclude: &[String]) -> EngineResult<()> {
    let id_hash = sha1_hex(jar.to_string_lossy().as_bytes());
    let marker = natives_dir.join(format!(".extract-ok-{id_hash}"));
    let meta_path = natives_dir.join(format!(".extract-meta-{id_hash}"));
    let stat = jar_stat(jar);
    if let Some((size, mtime)) = stat {
        if marker.exists() {
            if let Some(meta) =
                std::fs::read_to_string(&meta_path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok())
            {
                if meta.get("size").and_then(Value::as_u64) == Some(size)
                    && meta.get("mtimeMs").and_then(Value::as_f64).map(|m| same_mtime(m, mtime)).unwrap_or(false)
                {
                    return Ok(());
                }
            }
        }
    }
    let exclude: Vec<String> = extract_exclude
        .iter()
        .map(|e| {
            let e = e.replace('\\', "/");
            e.strip_prefix("./").or_else(|| e.strip_prefix('/')).map(str::to_string).unwrap_or(e)
        })
        .collect();
    let base = jar.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let base = base.strip_suffix(".jar").map(str::to_string).unwrap_or(base);
    let stage: PathBuf = natives_dir.join(format!(
        ".stage-{base}-{}-{}",
        std::process::id(),
        STAGE_SEQ.fetch_add(1, Ordering::Relaxed) + 1
    ));

    let result: EngineResult<()> = async {
        let mut last_err: Option<EngineError> = None;
        for _attempt in 1..=3 {
            let _ = std::fs::remove_dir_all(&stage);
            std::fs::create_dir_all(&stage)?;
            let (j, s) = (jar.to_path_buf(), stage.clone());
            let task = tokio::task::spawn_blocking(move || unzip_into(&j, &s));
            match tokio::time::timeout(EXTRACT_TIMEOUT, task).await {
                Ok(Ok(Ok(()))) => {
                    last_err = None;
                    break;
                }
                Ok(Ok(Err(e))) => last_err = Some(e),
                Ok(Err(e)) => last_err = Some(EngineError::other(e.to_string())),
                Err(_) => {
                    last_err = Some(EngineError::other(format!(
                        "natives extraction timed out: {}",
                        jar.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                    )))
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        let (s, n, ex) = (stage.clone(), natives_dir.to_path_buf(), exclude.clone());
        tokio::task::spawn_blocking(move || move_staged(&s, &s, &n, &ex))
            .await
            .map_err(|e| EngineError::other(e.to_string()))?
    }
    .await;
    let _ = std::fs::remove_dir_all(&stage);

    match result {
        Ok(()) => {
            write_markers(&marker, &meta_path, stat);
            Ok(())
        }
        Err(err) => {
            // Legacy natives entries are sometimes raw native libs, not
            // archives: drop the file in as-is. Genuine zip failures surface.
            if !is_zip_file(jar) {
                let name = jar.file_name().map(|s| s.to_os_string()).unwrap_or_default();
                std::fs::copy(jar, natives_dir.join(name))?;
                write_markers(&marker, &meta_path, stat);
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_jar(path: &Path, entries: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(name.trim_end_matches('/'), zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    #[tokio::test]
    async fn extracts_zip_honours_exclude_and_marks() {
        let d = tempfile::tempdir().unwrap();
        let jar = d.path().join("libs").join("lwjgl-3.3.3-natives-linux.jar");
        make_jar(
            &jar,
            &[
                ("META-INF/", b""),
                ("META-INF/MANIFEST.MF", b"m"),
                ("linux/x64/org/lwjgl/liblwjgl.so", b"so"),
                ("liblwjgl.so.sha1", b"h"),
            ],
        );
        let natives = d.path().join("natives");
        std::fs::create_dir_all(&natives).unwrap();
        extract_natives(&jar, &natives, &["META-INF/".to_string()]).await.unwrap();
        assert!(natives.join("linux/x64/org/lwjgl/liblwjgl.so").exists());
        assert!(natives.join("liblwjgl.so.sha1").exists());
        assert!(!natives.join("META-INF").exists());
        let names: Vec<String> = std::fs::read_dir(&natives)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.iter().filter(|n| n.starts_with(".extract-ok-")).count(), 1);
        assert_eq!(names.iter().filter(|n| n.starts_with(".stage-")).count(), 0, "stage dir removed");
        let meta_name = names.iter().find(|n| n.starts_with(".extract-meta-")).unwrap();
        let raw = std::fs::read_to_string(natives.join(meta_name)).unwrap();
        assert!(raw.starts_with("{\"size\":"), "{raw}");
    }

    #[tokio::test]
    async fn corrupt_zip_surfaces_error() {
        let d = tempfile::tempdir().unwrap();
        let jar = d.path().join("bad.jar");
        std::fs::write(&jar, b"PK\x03\x04garbage").unwrap();
        let natives = d.path().join("natives");
        std::fs::create_dir_all(&natives).unwrap();
        assert!(extract_natives(&jar, &natives, &[]).await.is_err());
    }

    #[test]
    fn js_number_formats_like_json_stringify() {
        assert_eq!(js_number(1234.0).to_string(), "1234");
        assert_eq!(js_number(1234.5).to_string(), "1234.5");
    }
}
