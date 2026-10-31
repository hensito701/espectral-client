//! Port of `src/engine/ffmpeg.mjs` — ffmpeg resolution for agent captures.
//!
//! Live recording and Replay Mod clip rendering both hand raw frames to
//! ffmpeg, and the game (not the engine) spawns it — so the engine tells the
//! JVM where the binary is via `-Despectral.ffmpeg=<abs path>`.
//!
//! - Linux / macOS: the system `ffmpeg` on PATH (resolved to an absolute path;
//!   the game's PATH is not guaranteed to match the launcher's, and a Windows
//!   install often has none at all).
//! - Windows: a pinned static build (gyan.dev essentials, GPL) downloaded on
//!   demand into `<dataDir>/runtimes/ffmpeg-<version>-win64/` — same shape as
//!   the Adoptium JDK provisioning (download → sha256 verify → extract →
//!   locate the exe), except the extraction commits with one rename so a
//!   failed attempt never leaves a half-populated install dir behind.
//!
//! Nothing here runs unless an agent launch or `capture_tools` needs it.
//!
//! **Owner: content (agent access).** Status: PORTED.
//!
//! CONTRACT (called by game-launch): [`resolve_ffmpeg_sync`] (dry runs),
//! [`ensure_ffmpeg`] (real agent launches; may provision).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::download::{DownloadOptions, FetchOptions, USER_AGENT};
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};

/// Pinned Windows build. gyan.dev keeps every release under a versioned URL,
/// so the pin never rots the way the `latest` aliases do.
///
/// `sha256` = the value the publisher's `<zip>.sha256` sidecar carries.
/// Provisioning re-fetches that sidecar and refuses the archive unless it
/// matches BOTH the pin and the bytes on disk: the pin alone would go stale
/// silently if the release were ever rebuilt in place, and the sidecar alone
/// would trust whatever the same origin happened to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfmpegWinPin {
    pub version: &'static str,
    pub url: &'static str,
    pub sha256_url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// JS `FFMPEG_WIN_PIN`.
pub const FFMPEG_WIN_PIN: FfmpegWinPin = FfmpegWinPin {
    version: "8.1.2",
    url: "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip",
    sha256_url: "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip.sha256",
    sha256: "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec",
    size: 109_728_040,
};

/// Download source of the pinned archive. Defaults to [`FFMPEG_WIN_PIN`];
/// tests point it at a loopback `MockServer` (and supply their own sha256,
/// which cannot match the pin for a generated archive).
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegSource {
    pub archive_url: String,
    pub sha256_url: String,
    pub sha256: String,
}

impl Default for FfmpegSource {
    fn default() -> Self {
        FfmpegSource {
            archive_url: FFMPEG_WIN_PIN.url.into(),
            sha256_url: FFMPEG_WIN_PIN.sha256_url.into(),
            sha256: FFMPEG_WIN_PIN.sha256.into(),
        }
    }
}

/// Marker written LAST inside a staged tree, before the one rename that makes
/// it the install dir. Its presence is what makes an install dir trustworthy:
/// `bin/ffmpeg.exe` alone exists part-way through an extraction, so an
/// "exe exists" test would accept a tree whose ffprobe/sidecars never landed.
const COMPLETE_MARKER: &str = ".espectral-install-complete";

/// Extraction is one ~110 MB unzip; a stalled entry chain must fail, not hang.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Download timeout for the pinned archive (JS 30 min).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Sidecar fetch timeout (JS 30 s).
const SIDECAR_TIMEOUT: Duration = Duration::from_secs(30);

/// Directory the Windows build is provisioned into (`<data>/runtimes/ffmpeg-<v>-win64`).
pub fn ffmpeg_install_dir(engine: &Engine) -> PathBuf {
    engine.paths().data_dir().join("runtimes").join(format!("ffmpeg-{}-win64", FFMPEG_WIN_PIN.version))
}

/// Absolute path of the provisioned Windows ffmpeg.exe.
pub fn provisioned_ffmpeg_path(engine: &Engine) -> PathBuf {
    ffmpeg_install_dir(engine).join("bin").join("ffmpeg.exe")
}

/// True only for a fully committed install dir (marker + exe).
pub fn ffmpeg_install_complete(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).exists() && dir.join("bin").join("ffmpeg.exe").exists()
}

/// First `ffmpeg` on PATH as an absolute path, else None (`which` / `where`).
pub fn path_ffmpeg() -> Option<PathBuf> {
    path_ffmpeg_in(std::env::var_os("PATH").as_deref())
}

/// [`path_ffmpeg`] against an explicit PATH value (tests: a temp dir).
#[doc(hidden)]
pub fn path_ffmpeg_in(path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let found = crate::jvm::find_on_path("ffmpeg", path_var)?;
    let abs = if found.is_absolute() { found } else { std::env::current_dir().ok()?.join(found) };
    abs.exists().then_some(abs)
}

/// Best-effort ffmpeg path, synchronously — what a launch wants (argv building
/// cannot download). Windows: the already-provisioned build only; a missing
/// one is provisioned by [`ensure_ffmpeg`] ahead of the launch.
pub fn resolve_ffmpeg_sync(engine: &Engine) -> Option<PathBuf> {
    if cfg!(windows) {
        // Only a committed tree counts: a half-extracted install dir (marker
        // absent) must fall through to PATH rather than hand the game a
        // truncated binary, and be re-provisioned by `ensure_ffmpeg`.
        if ffmpeg_install_complete(&ffmpeg_install_dir(engine)) {
            return Some(provisioned_ffmpeg_path(engine));
        }
        return path_ffmpeg();
    }
    path_ffmpeg()
}

/// In-flight provisioning (JS module-level `provisionPromise`): concurrent
/// callers await one shared attempt instead of each downloading ~110 MB.
///
/// Keyed by data dir rather than being one global: JS has exactly one engine
/// per process, Rust tests have many, and a test must never join another
/// engine's attempt (or inherit its failure).
static PROVISION: std::sync::LazyLock<tokio::sync::Mutex<std::collections::HashMap<PathBuf, Inflight>>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// One shared provisioning attempt: the leader writes the settled value into
/// `cell` and wakes every follower through `tx` (same critical section that
/// clears the slot, so a receiver can never miss the wake-up).
#[derive(Clone)]
struct Inflight {
    cell: std::sync::Arc<std::sync::OnceLock<Option<PathBuf>>>,
    tx: tokio::sync::watch::Sender<()>,
}

impl Inflight {
    fn new() -> (Inflight, tokio::sync::watch::Receiver<()>) {
        let (tx, rx) = tokio::sync::watch::channel(());
        (Inflight { cell: Default::default(), tx }, rx)
    }

    fn settled(&self) -> Option<PathBuf> {
        self.cell.get().cloned().flatten()
    }
}

/// ffmpeg path for a launch that NEEDS it, provisioning on Windows when the
/// machine has none. Returns None when nothing is available — callers treat a
/// missing ffmpeg as "captures unavailable", never as a launch error.
pub async fn ensure_ffmpeg(engine: &Engine) -> EngineResult<Option<PathBuf>> {
    ensure_ffmpeg_with(engine, &FfmpegSource::default()).await
}

/// [`ensure_ffmpeg`] against an explicit download source (tests).
#[doc(hidden)]
pub async fn ensure_ffmpeg_with(engine: &Engine, src: &FfmpegSource) -> EngineResult<Option<PathBuf>> {
    if let Some(existing) = resolve_ffmpeg_sync(engine) {
        return Ok(Some(existing));
    }
    // POSIX never provisions: the packaged app relies on the system binary.
    if !cfg!(windows) {
        return Ok(None);
    }
    Ok(provision_once(engine, src).await)
}

/// One provisioning attempt shared by every concurrent caller. A failure is
/// reported to all of them and the next call starts a fresh attempt.
#[doc(hidden)]
pub async fn provision_once(engine: &Engine, src: &FfmpegSource) -> Option<PathBuf> {
    let key = engine.paths().data_dir().to_path_buf();
    let (inflight, rx, leader) = {
        let mut slot = PROVISION.lock().await;
        match slot.get(&key) {
            Some(i) => {
                // Subscribe under the lock: a wake-up can never happen before
                // this receiver exists.
                let rx = i.tx.subscribe();
                (i.clone(), rx, false)
            }
            None => {
                let (i, rx) = Inflight::new();
                slot.insert(key.clone(), i.clone());
                (i, rx, true)
            }
        }
    };
    if !leader {
        let mut rx = rx;
        let _ = rx.changed().await;
        return inflight.settled();
    }
    let value = match provision_ffmpeg_with(engine, src).await {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!("[ffmpeg] provisioning failed: {e}");
            None
        }
    };
    let _ = inflight.cell.set(value.clone());
    {
        let mut slot = PROVISION.lock().await;
        let _ = inflight.tx.send(());
        slot.remove(&key);
    }
    value
}

/// Download the pinned archive (JS `downloadPinnedFfmpeg`).
async fn download_pinned_ffmpeg(engine: &Engine, src: &FfmpegSource, archive: &Path) -> EngineResult<PathBuf> {
    let opts = DownloadOptions {
        headers: vec![("User-Agent".into(), USER_AGENT.into())],
        timeout: Some(DOWNLOAD_TIMEOUT),
        ..DownloadOptions::default()
    };
    engine.http().download_file(&src.archive_url, archive, &opts).await
}

/// The publisher's published sha256 for the pinned archive
/// (JS `publishedFfmpegSha256`): the first whitespace-separated token of the
/// sidecar, lowercased, which must be 64 hex digits.
async fn published_ffmpeg_sha256(engine: &Engine, src: &FfmpegSource) -> EngineResult<String> {
    let opts = FetchOptions { headers: vec![("User-Agent".into(), USER_AGENT.into())], timeout: SIDECAR_TIMEOUT };
    let text = engine.http().fetch_text(&src.sha256_url, &opts).await?;
    let hash = text.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(EngineError::other("publisher published no usable sha256 for the ffmpeg archive"));
    }
    Ok(hash)
}

/// Download + extract the pinned Windows build; idempotent.
pub async fn provision_ffmpeg_with(engine: &Engine, src: &FfmpegSource) -> EngineResult<PathBuf> {
    let dir = ffmpeg_install_dir(engine);
    let runtimes = dir.parent().map(Path::to_path_buf).unwrap_or_else(|| engine.paths().data_dir().join("runtimes"));
    tokio::fs::create_dir_all(&runtimes).await?;
    // The archive lives BESIDE the install dir: the install dir only ever
    // appears through one rename (below), so it must never hold transient
    // files. Kept only for the length of this call (an already-cached,
    // sha256-verified archive saves the 110 MB download) and removed after.
    let archive = runtimes.join(format!("ffmpeg-{}-win64.zip", FFMPEG_WIN_PIN.version));
    let outcome = async {
        ensure_pinned_archive(engine, src, &archive).await?;
        install_ffmpeg_archive(&archive, &dir).await?;
        Ok::<(), EngineError>(())
    }
    .await;
    // The pin makes the archive re-downloadable, so never leave 110 MB in the
    // data dir. A concurrent provisioner may still hold it open (Windows
    // EBUSY): that is cosmetic — the next run re-verifies the same bytes.
    let _ = tokio::fs::remove_file(&archive).await;
    outcome?;
    let exe = dir.join("bin").join("ffmpeg.exe");
    if !exe.exists() {
        return Err(EngineError::other(format!("ffmpeg extracted but no bin/ffmpeg.exe under {}", dir.display())));
    }
    Ok(exe)
}

/// Ensure `archive` holds exactly the pinned bytes, downloading it when not.
/// The publisher's sidecar is consulted first so a silently rebuilt release is
/// reported as a stale pin instead of a download failure.
async fn ensure_pinned_archive(engine: &Engine, src: &FfmpegSource, archive: &Path) -> EngineResult<PathBuf> {
    if archive.exists() {
        let cached = sha256_file(archive).await.unwrap_or_default();
        if cached == src.sha256 {
            return Ok(archive.to_path_buf());
        }
    }
    let _ = tokio::fs::remove_file(archive).await;
    let mut published = None;
    match published_ffmpeg_sha256(engine, src).await {
        Ok(h) => published = Some(h),
        Err(e) => tracing::warn!("[ffmpeg] could not read the publisher checksum: {e}"),
    }
    if let Some(published) = published {
        if published != src.sha256 {
            return Err(EngineError::other(format!(
                "pinned ffmpeg checksum no longer matches the publisher (pin {}, published {published}); \
                 update FFMPEG_WIN_PIN after re-verifying the archive",
                src.sha256
            )));
        }
    }
    download_pinned_ffmpeg(engine, src, archive).await?;
    let got = sha256_file(archive).await.map_err(EngineError::from)?;
    if got != src.sha256 {
        let _ = tokio::fs::remove_file(archive).await;
        return Err(EngineError::other(format!(
            "sha256 mismatch for pinned ffmpeg build (got {got}, expected {})",
            src.sha256
        )));
    }
    Ok(archive.to_path_buf())
}

/// Extract a verified archive and commit it as `dir`, so `dir` only ever
/// exists complete.
///
/// Staging + one rename: a killed or failed extraction leaves `dir` absent
/// instead of half-populated. Concurrent provisioners race on two private
/// staging dirs and the rename; the loser finds the winner's `dir` in place
/// and keeps it.
pub async fn install_ffmpeg_archive(archive: &Path, dir: &Path) -> EngineResult<PathBuf> {
    if ffmpeg_install_complete(dir) {
        return Ok(dir.to_path_buf());
    }
    static STAGE_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = STAGE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let stage = {
        let mut s = dir.as_os_str().to_owned();
        s.push(format!(".staging-{}-{seq}", std::process::id()));
        PathBuf::from(s)
    };
    let _ = tokio::fs::remove_dir_all(&stage).await;
    tokio::fs::create_dir_all(&stage).await?;
    let outcome = async {
        extract_ffmpeg_archive(archive, &stage).await?;
        tokio::fs::write(stage.join(COMPLETE_MARKER), format!("{}\n", FFMPEG_WIN_PIN.sha256)).await?;
        let mut attempt = 1;
        loop {
            match tokio::fs::rename(&stage, dir).await {
                // stage is gone: the tree is the install dir now.
                Ok(()) => return Ok(dir.to_path_buf()),
                Err(err) => {
                    // A concurrent provisioner may have committed a usable tree first.
                    if ffmpeg_install_complete(dir) {
                        return Ok(dir.to_path_buf());
                    }
                    // Otherwise an unusable leftover is in the way; replace it
                    // once, then give up rather than deleting forever.
                    if attempt >= 2 {
                        return Err(EngineError::from(err));
                    }
                    attempt += 1;
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
            }
        }
    }
    .await;
    // No-op after a successful commit; removes the staged tree otherwise.
    let _ = tokio::fs::remove_dir_all(&stage).await;
    outcome
}

/// Unpack `archive` into `dest` so `dest/bin/ffmpeg.exe` exists.
///
/// Zip-slip: entries are rejected on symlink mode and on a containment check
/// of their resolved path (`bin -> /` plus a later `bin/x` write would land
/// outside `dest`). Only a pinned, sha256-verified archive is ever extracted;
/// this is the second lock on that door.
pub async fn extract_ffmpeg_archive(archive: &Path, dest: &Path) -> EngineResult<()> {
    let archive = archive.to_path_buf();
    let root = dest.to_path_buf();
    let task = tokio::task::spawn_blocking(move || extract_blocking(&archive, &root));
    match tokio::time::timeout(EXTRACT_TIMEOUT, task).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => Err(EngineError::other(format!("ffmpeg extraction failed: {e}"))),
        Err(_) => Err(EngineError::other("ffmpeg extraction timed out")),
    }
}

fn extract_blocking(archive: &Path, dest: &Path) -> EngineResult<()> {
    std::fs::create_dir_all(dest)?;
    let root = std::path::absolute(dest).map_err(EngineError::from)?;
    let file = std::fs::File::open(archive)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| EngineError::other(format!("ffmpeg extraction failed: {e}")))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| EngineError::other(format!("ffmpeg extraction failed: {e}")))?;
        let name = entry.name().to_string();
        if let Some(mode) = entry.unix_mode() {
            if (mode & 0o170000) == 0o120000 {
                return Err(EngineError::other(format!("ffmpeg archive entry is a symlink: {name}")));
            }
        }
        let escapes = || EngineError::other(format!("ffmpeg archive entry escapes the install dir: {name}"));
        if name.contains('\\') || name.split('/').any(|seg| seg == "..") {
            return Err(escapes());
        }
        // `enclosed_name()` refuses absolute paths, drive letters and `..`
        // segments (yauzl-shaped); the explicit checks above keep the exact
        // JS message for the `..`/backslash cases.
        let Some(rel) = entry.enclosed_name() else {
            return Err(escapes());
        };
        let abs = root.join(&rel);
        if abs != root && !abs.starts_with(&root) {
            return Err(escapes());
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&abs)?;
            continue;
        }
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&abs)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    // gyan.dev zips nest everything under `ffmpeg-<version>-essentials_build/`;
    // hoist that folder so bin\ffmpeg.exe is canonical (same trick as the JDK).
    let nested = if root.join("bin").join("ffmpeg.exe").exists() {
        None
    } else {
        std::fs::read_dir(&root)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir() && p.join("bin").join("ffmpeg.exe").exists())
    };
    if let Some(inner) = nested {
        for entry in std::fs::read_dir(&inner)? {
            let entry = entry?;
            std::fs::rename(entry.path(), root.join(entry.file_name()))?;
        }
        std::fs::remove_dir_all(&inner)?;
    }
    if !root.join("bin").join("ffmpeg.exe").exists() {
        return Err(EngineError::other("ffmpeg archive has no bin/ffmpeg.exe"));
    }
    Ok(())
}

/// sha256 of a file (streamed, off the async runtime).
pub async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let p = path.to_path_buf();
    tokio::task::spawn_blocking(move || sha256_file_blocking(&p)).await.map_err(std::io::Error::other)?
}

fn sha256_file_blocking(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
