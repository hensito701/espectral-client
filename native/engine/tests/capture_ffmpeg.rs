//! ffmpeg provisioning + `capture_tools` (Replay Mod / shader packs), ported
//! from `tests/ffmpeg.test.mjs` and the capture-tools block of
//! `tests/mods.test.mjs` (+ the `capture_tools` guard of
//! `tests/agent-api.test.mjs`).
//!
//! Everything is sandboxed: temp data dirs, archives built in memory, a
//! loopback `MockServer` standing in for Modrinth + its CDN. Nothing touches
//! the real network or the user's home dir.

use std::path::{Path, PathBuf};
use std::time::Duration;

use espectral_engine::ffmpeg::{self, FfmpegSource, FFMPEG_WIN_PIN};
use espectral_engine::mods::{
    self, capture_shader_packs, default_shader_pack, fast_shader_pack, install_capture_tools,
    install_capture_tools_with, remove_capture_tools, replay_mod_pin, select_default_shader_pack, CaptureToolsOptions,
    ReplayModPin, ShaderPack, CAPTURE_SHADER_PACKS_LEN, REPLAY_MOD_PINNED_VERSIONS, REPLAY_MOD_PROJECT,
};
use espectral_engine::test_support::{temp_engine, MockRequest, MockResponse, MockServer};
use espectral_engine::types::{InstanceMeta, InstancePatch};
use espectral_engine::Engine;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha1_of(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

fn sha256_of(bytes: &[u8]) -> String {
    use sha2::Sha256;
    hex::encode(Sha256::digest(bytes))
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Sorted file names of a directory (empty when it does not exist).
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    };
    v.sort();
    v
}

fn code(e: &espectral_engine::EngineError) -> (u16, String) {
    let env = e.envelope();
    (env.status, env.code)
}

/// Minimal stored-entry ZIP writer (`buildZip` in the JS test).
struct ZipEntry {
    name: String,
    data: Vec<u8>,
    symlink: Option<String>,
    dir: bool,
}

fn entry(name: &str, data: &[u8]) -> ZipEntry {
    ZipEntry { name: name.into(), data: data.to_vec(), symlink: None, dir: false }
}

fn dir_entry(name: &str) -> ZipEntry {
    ZipEntry { name: name.into(), data: vec![], symlink: None, dir: true }
}

fn symlink_entry(name: &str, target: &str) -> ZipEntry {
    ZipEntry { name: name.into(), data: vec![], symlink: Some(target.into()), dir: false }
}

fn build_zip(entries: Vec<ZipEntry>) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for e in entries {
        let opts = zip::write::SimpleFileOptions::default();
        match e.symlink {
            Some(target) => w.add_symlink(e.name.clone(), target, opts).unwrap(),
            None if e.dir => w.add_directory(e.name.clone(), opts).unwrap(),
            None => {
                w.start_file(e.name.clone(), opts).unwrap();
                w.write_all(&e.data).unwrap();
            }
        }
    }
    w.finish().unwrap().into_inner()
}

/// A gyan.dev-shaped archive: everything nested under one versioned folder.
fn pinned_shaped_zip() -> Vec<u8> {
    let root = format!("ffmpeg-{}-essentials_build", FFMPEG_WIN_PIN.version);
    build_zip(vec![
        dir_entry(&format!("{root}/")),
        entry(&format!("{root}/README.txt"), b"gyan.dev build"),
        dir_entry(&format!("{root}/bin/")),
        entry(&format!("{root}/bin/ffmpeg.exe"), b"MZ-ffmpeg"),
        entry(&format!("{root}/bin/ffprobe.exe"), b"MZ-ffprobe"),
    ])
}

/// Write a ZIP built from `entries` under `dir`, returning its path.
fn write_zip(dir: &Path, name: &str, entries: Vec<ZipEntry>) -> PathBuf {
    let file = dir.join(format!("archive-{name}.zip"));
    std::fs::write(&file, build_zip(entries)).unwrap();
    file
}

// ---------------------------------------------------------------------------
// ffmpeg pin / layout
// ---------------------------------------------------------------------------

#[test]
fn pin_url_sha256_and_size_agree_with_the_publisher() {
    let pin = FFMPEG_WIN_PIN;
    assert_eq!(pin.sha256.len(), 64);
    assert!(pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(pin.url.rsplit('/').next().unwrap(), format!("ffmpeg-{}-essentials_build.zip", pin.version));
    assert_eq!(pin.sha256_url, format!("{}.sha256", pin.url));
    assert!(pin.size > 0);
    // The install dir is keyed by version so a pin bump never reuses stale bytes.
    let (_d, e) = temp_engine();
    let dir = ffmpeg::ffmpeg_install_dir(&e);
    assert!(dir.ends_with(format!("ffmpeg-{}-win64", pin.version)));
    assert!(dir.starts_with(e.paths().data_dir()));
    assert!(ffmpeg::provisioned_ffmpeg_path(&e).ends_with(Path::new("bin").join("ffmpeg.exe")));
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extract_hoists_the_single_nested_build_folder() {
    let (d, _e) = temp_engine();
    let archive = d.path().join("hoist.zip");
    std::fs::write(&archive, pinned_shaped_zip()).unwrap();
    let dest = d.path().join("hoist-out");

    ffmpeg::extract_ffmpeg_archive(&archive, &dest).await.unwrap();

    assert_eq!(read(dest.join("bin").join("ffmpeg.exe")), "MZ-ffmpeg");
    assert_eq!(read(dest.join("bin").join("ffprobe.exe")), "MZ-ffprobe");
    assert!(dest.join("README.txt").exists(), "sibling files are hoisted too");
    let nested: Vec<String> =
        names(&dest).into_iter().filter(|n| n.starts_with("ffmpeg-") && n.ends_with("_build")).collect();
    assert!(nested.is_empty(), "the nested folder is gone, not duplicated: {nested:?}");
}

#[tokio::test]
async fn extract_rejects_an_archive_without_the_exe() {
    let (d, _e) = temp_engine();
    let archive = write_zip(d.path(), "noexe", vec![entry("README.txt", b"nothing here")]);
    let err = ffmpeg::extract_ffmpeg_archive(&archive, &d.path().join("noexe-out")).await.unwrap_err();
    assert!(err.to_string().contains("no bin/ffmpeg.exe"), "{err}");
}

#[tokio::test]
async fn extract_refuses_a_symlink_entry() {
    let (d, _e) = temp_engine();
    let archive = write_zip(d.path(), "symlink", vec![symlink_entry("bin", "/"), entry("bin/ffmpeg.exe", b"MZ")]);
    let err = ffmpeg::extract_ffmpeg_archive(&archive, &d.path().join("symlink-out")).await.unwrap_err();
    assert!(err.to_string().contains("symlink"), "{err}");
}

#[tokio::test]
async fn extract_refuses_traversal_and_absolute_entry_paths() {
    let (d, _e) = temp_engine();
    for (label, name) in [("dotdot", "../ffmpeg-escape.txt"), ("absolute", "/ffmpeg-escape.txt")] {
        let archive = write_zip(d.path(), label, vec![entry("bin/ffmpeg.exe", b"MZ"), entry(name, b"pwned")]);
        let dest = d.path().join(format!("{label}-out"));
        let err = ffmpeg::extract_ffmpeg_archive(&archive, &dest).await.unwrap_err();
        assert!(err.to_string().contains("escapes the install dir"), "{err}");
    }
    assert!(!d.path().join("ffmpeg-escape.txt").exists(), "nothing written outside the install dir");
}

// ---------------------------------------------------------------------------
// Staged install
// ---------------------------------------------------------------------------

#[tokio::test]
async fn install_commits_atomically_and_is_idempotent() {
    let (d, _e) = temp_engine();
    let archive = d.path().join("commit.zip");
    std::fs::write(&archive, pinned_shaped_zip()).unwrap();
    let dest = d.path().join("runtimes").join("commit-win64");

    ffmpeg::install_ffmpeg_archive(&archive, &dest).await.unwrap();
    assert_eq!(read(dest.join("bin").join("ffmpeg.exe")), "MZ-ffmpeg");

    // Idempotent: a second call keeps the committed tree and does no work.
    let before = std::fs::metadata(dest.join("bin").join("ffmpeg.exe")).unwrap().modified().unwrap();
    ffmpeg::install_ffmpeg_archive(&archive, &dest).await.unwrap();
    let after = std::fs::metadata(dest.join("bin").join("ffmpeg.exe")).unwrap().modified().unwrap();
    assert_eq!(before, after, "a committed tree is kept, no work done");
}

#[tokio::test]
async fn install_leaves_no_install_dir_and_no_staging_after_a_failed_extraction() {
    let (d, _e) = temp_engine();
    let archive = write_zip(d.path(), "broken", vec![entry("README.txt", b"no exe")]);
    let runtimes = d.path().join("runtimes");
    let dest = runtimes.join("failed-win64");

    assert!(ffmpeg::install_ffmpeg_archive(&archive, &dest).await.is_err());
    assert!(!dest.exists(), "a half-extracted tree must never look provisioned");
    let staging: Vec<String> = names(&runtimes).into_iter().filter(|n| n.starts_with("failed-win64.staging")).collect();
    assert!(staging.is_empty(), "staging cleaned up: {staging:?}");
}

#[tokio::test]
async fn install_replaces_an_unusable_leftover_install_dir() {
    let (d, _e) = temp_engine();
    let archive = d.path().join("replace.zip");
    std::fs::write(&archive, pinned_shaped_zip()).unwrap();
    let dest = d.path().join("runtimes").join("stale-win64");
    // Simulate a pre-staging crash: bin/ffmpeg.exe exists but the rest is gone.
    std::fs::create_dir_all(dest.join("bin")).unwrap();
    std::fs::write(dest.join("bin").join("ffmpeg.exe"), "truncated").unwrap();

    ffmpeg::install_ffmpeg_archive(&archive, &dest).await.unwrap();
    assert_eq!(read(dest.join("bin").join("ffmpeg.exe")), "MZ-ffmpeg");
    assert_eq!(read(dest.join("bin").join("ffprobe.exe")), "MZ-ffprobe");
}

#[tokio::test]
async fn concurrent_installs_converge_on_one_complete_tree() {
    let (d, _e) = temp_engine();
    let archive = d.path().join("race.zip");
    std::fs::write(&archive, pinned_shaped_zip()).unwrap();
    let dest = d.path().join("runtimes").join("race-win64");

    let (a, b, c) = tokio::join!(
        ffmpeg::install_ffmpeg_archive(&archive, &dest),
        ffmpeg::install_ffmpeg_archive(&archive, &dest),
        ffmpeg::install_ffmpeg_archive(&archive, &dest)
    );
    a.unwrap();
    b.unwrap();
    c.unwrap();
    assert_eq!(read(dest.join("bin").join("ffmpeg.exe")), "MZ-ffmpeg");
    assert_eq!(read(dest.join("bin").join("ffprobe.exe")), "MZ-ffprobe");
    let staging: Vec<String> =
        names(&d.path().join("runtimes")).into_iter().filter(|n| n.contains(".staging-")).collect();
    assert!(staging.is_empty(), "no staging leftovers: {staging:?}");
}

#[tokio::test]
async fn a_bare_exe_without_the_commit_marker_is_not_usable() {
    let (d, _e) = temp_engine();
    let dir = d.path().join("runtimes").join("marker-win64");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("bin").join("ffmpeg.exe"), "half-extracted").unwrap();
    assert!(!ffmpeg::ffmpeg_install_complete(&dir), "no marker -> not a finished install");

    let archive = d.path().join("marker.zip");
    std::fs::write(&archive, pinned_shaped_zip()).unwrap();
    ffmpeg::install_ffmpeg_archive(&archive, &dir).await.unwrap();

    assert!(ffmpeg::ffmpeg_install_complete(&dir));
    assert_eq!(read(dir.join("bin").join("ffmpeg.exe")), "MZ-ffmpeg");
    assert_eq!(read(dir.join("bin").join("ffprobe.exe")), "MZ-ffprobe");
}

// ---------------------------------------------------------------------------
// Provisioning against a mocked origin
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provisioning_downloads_verifies_extracts_and_cleans_up() {
    let (_d, e) = temp_engine();
    let body = pinned_shaped_zip();
    let sha = sha256_of(&body);
    let serve = body.clone();
    let sidecar = sha.clone();
    let srv = MockServer::start(move |req: &MockRequest| {
        if req.path.ends_with(".sha256") {
            MockResponse::ok(&format!("{sidecar}  ffmpeg-8.1.2-essentials_build.zip\n"))
        } else {
            MockResponse::bytes(serve.clone())
        }
    })
    .await;
    let src = FfmpegSource {
        archive_url: srv.url("/ffmpeg-8.1.2-essentials_build.zip"),
        sha256_url: srv.url("/ffmpeg-8.1.2-essentials_build.zip.sha256"),
        sha256: sha.clone(),
    };

    let exe = ffmpeg::provision_ffmpeg_with(&e, &src).await.unwrap();

    assert_eq!(exe, ffmpeg::provisioned_ffmpeg_path(&e));
    assert_eq!(read(&exe), "MZ-ffmpeg");
    assert!(ffmpeg::ffmpeg_install_complete(&ffmpeg::ffmpeg_install_dir(&e)));
    // The pinned archive never stays behind in the data dir.
    let archive = e.paths().data_dir().join("runtimes").join(format!("ffmpeg-{}-win64.zip", FFMPEG_WIN_PIN.version));
    assert!(!archive.exists(), "{}", archive.display());
}

#[tokio::test]
async fn provisioning_refuses_a_rebuilt_release_reported_by_the_sidecar() {
    let (_d, e) = temp_engine();
    let body = pinned_shaped_zip();
    let published = "f".repeat(64);
    let serve = body.clone();
    let srv = MockServer::start(move |req: &MockRequest| {
        if req.path.ends_with(".sha256") {
            MockResponse::ok(&published)
        } else {
            MockResponse::bytes(serve.clone())
        }
    })
    .await;
    let src = FfmpegSource {
        archive_url: srv.url("/ffmpeg.zip"),
        sha256_url: srv.url("/ffmpeg.zip.sha256"),
        sha256: sha256_of(&body),
    };

    let err = ffmpeg::provision_ffmpeg_with(&e, &src).await.unwrap_err();
    assert!(err.to_string().contains("no longer matches the publisher"), "{err}");
    assert_eq!(srv.hits(), 1, "no archive download after a stale pin is reported");
}

#[tokio::test]
async fn provisioning_rejects_bytes_that_do_not_match_the_pin() {
    let (_d, e) = temp_engine();
    let body = pinned_shaped_zip();
    let serve = body.clone();
    let srv = MockServer::start(move |req: &MockRequest| {
        if req.path.ends_with(".sha256") {
            // A sidecar the pin already disagrees with is reported as a stale
            // pin, so serve a *matching* sidecar and let the bytes be wrong.
            MockResponse::ok(&"0".repeat(64))
        } else {
            MockResponse::bytes(serve.clone())
        }
    })
    .await;
    let src = FfmpegSource {
        archive_url: srv.url("/ffmpeg.zip"),
        sha256_url: srv.url("/ffmpeg.zip.sha256"),
        sha256: "0".repeat(64),
    };

    let err = ffmpeg::provision_ffmpeg_with(&e, &src).await.unwrap_err();
    assert!(err.to_string().contains("sha256 mismatch"), "{err}");
    assert!(!ffmpeg::ffmpeg_install_complete(&ffmpeg::ffmpeg_install_dir(&e)));
    let archive = e.paths().data_dir().join("runtimes").join(format!("ffmpeg-{}-win64.zip", FFMPEG_WIN_PIN.version));
    assert!(!archive.exists(), "the rejected archive is removed");
}

#[tokio::test]
async fn ensure_ffmpeg_never_provisions_off_windows() {
    let (_d, e) = temp_engine();
    let resolved = ffmpeg::resolve_ffmpeg_sync(&e);
    assert!(resolved.as_ref().map(|p| p.is_absolute()).unwrap_or(true), "{resolved:?}");
    let ensured = ffmpeg::ensure_ffmpeg(&e).await.unwrap();
    assert_eq!(ensured, resolved, "POSIX never provisions");
    assert!(!ffmpeg::ffmpeg_install_dir(&e).exists());
}

#[tokio::test]
async fn concurrent_provisions_share_one_download() {
    let (_d, e) = temp_engine();
    let body = pinned_shaped_zip();
    let sha = sha256_of(&body);
    let serve = body.clone();
    let sidecar = sha.clone();
    // Slow answers keep the leader in flight while the followers join, which is
    // exactly the two-agent-launches race `provisionPromise` exists for.
    let srv = MockServer::start(move |req: &MockRequest| {
        let resp = if req.path.ends_with(".sha256") {
            MockResponse::ok(&format!("{sidecar}  ffmpeg-8.1.2-essentials_build.zip\n"))
        } else {
            MockResponse::bytes(serve.clone())
        };
        resp.with_delay(Duration::from_millis(50))
    })
    .await;
    let src = FfmpegSource {
        archive_url: srv.url("/ffmpeg.zip"),
        sha256_url: srv.url("/ffmpeg.zip.sha256"),
        sha256: sha.clone(),
    };

    let (a, b, c) = tokio::join!(
        ffmpeg::provision_once(&e, &src),
        ffmpeg::provision_once(&e, &src),
        ffmpeg::provision_once(&e, &src)
    );

    let exe = ffmpeg::provisioned_ffmpeg_path(&e);
    assert_eq!(a.as_deref(), Some(exe.as_path()));
    assert_eq!(b.as_deref(), Some(exe.as_path()), "followers get the leader's result");
    assert_eq!(c.as_deref(), Some(exe.as_path()));
    assert_eq!(srv.hits(), 2, "one sidecar + one archive for three callers");
}

#[tokio::test]
async fn a_failed_provision_leaves_no_stale_slot_behind() {
    let (_d, e) = temp_engine();
    let body = pinned_shaped_zip();
    let sha = sha256_of(&body);
    let good = body.clone();
    let sidecar = sha.clone();
    let srv = MockServer::start(move |req: &MockRequest| {
        if req.path.ends_with(".sha256") {
            MockResponse::ok(&format!("{sidecar}  ffmpeg-8.1.2-essentials_build.zip\n"))
        } else if req.path == "/bad.zip" {
            MockResponse::bytes(b"not the pinned archive".to_vec())
        } else {
            MockResponse::bytes(good.clone())
        }
    })
    .await;
    let bad = FfmpegSource {
        archive_url: srv.url("/bad.zip"),
        sha256_url: srv.url("/ffmpeg.zip.sha256"),
        sha256: sha.clone(),
    };
    let ok = FfmpegSource {
        archive_url: srv.url("/good.zip"),
        sha256_url: srv.url("/ffmpeg.zip.sha256"),
        sha256: sha.clone(),
    };

    assert!(ffmpeg::provision_once(&e, &bad).await.is_none(), "a failed provision reports no path");
    // The next caller starts a FRESH attempt instead of replaying the failure.
    assert_eq!(ffmpeg::provision_once(&e, &ok).await.as_deref(), Some(ffmpeg::provisioned_ffmpeg_path(&e).as_path()));
}

#[test]
fn path_lookup_resolves_only_existing_executables() {
    let d = tempfile::tempdir().unwrap();
    let bin = d.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path_var = std::env::join_paths([&bin]).unwrap();
    let found = ffmpeg::path_ffmpeg_in(Some(&path_var)).expect("found on PATH");
    assert!(found.is_absolute(), "{found:?}");
    assert_eq!(found.file_name().unwrap(), exe.file_name().unwrap());
    // No PATH value resolves nothing.
    assert!(ffmpeg::path_ffmpeg_in(None).is_none());
}

// ---------------------------------------------------------------------------
// Capture tools: fake Modrinth + CDN
// ---------------------------------------------------------------------------

const REPLAY_BYTES: &[u8] = b"the pinned replay jar";

fn replay_pin(version_id: &str) -> ReplayModPin {
    ReplayModPin {
        version_id: version_id.into(),
        sha1: sha1_of(REPLAY_BYTES),
        filename: "replaymod-26.3-2.6.28.jar".into(),
        version_number: "2.6.28".into(),
    }
}

fn pack_body(name: &str) -> String {
    format!("body of {name}")
}

fn pack_pin(name: &str, base: &str) -> ShaderPack {
    let body = pack_body(name);
    ShaderPack {
        slug: name.trim_end_matches(".zip").to_lowercase(),
        project_id: "testpack".into(),
        version_id: "v1".into(),
        version_number: "1.0".into(),
        filename: name.into(),
        sha1: sha1_of(body.as_bytes()),
        size: body.len() as u64,
        url: format!("{base}/cdn/{name}"),
    }
}

/// A temp engine with `modrinth` as the Modrinth base and a `capture`
/// instance (mods dir created) on `version`/`loader`.
fn capture_instance(modrinth: &str, version: &str, loader: &str) -> (tempfile::TempDir, Engine) {
    let (d, e) = temp_engine();
    e.update_content_settings(|s| {
        s.modrinth_api = modrinth.to_string();
        s.modrinth_retry_backoff = Duration::from_millis(1);
        s.background_on_create = false;
    });
    let dir = e.paths().instance_dir("capture");
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    std::fs::write(
        dir.join("instance.json"),
        json!({ "name": "capture", "version": version, "loader": loader }).to_string(),
    )
    .unwrap();
    (d, e)
}

fn capture_meta(version: &str, loader: &str) -> InstanceMeta {
    serde_json::from_value(json!({ "name": "capture", "version": version, "loader": loader })).unwrap()
}

fn game_dir(e: &Engine) -> PathBuf {
    e.paths().instance_dir("capture")
}

/// Fake Modrinth (`/v2/version/<id>`) + CDN (`/cdn/<file>`) covering the
/// replay pin and the 26.3 performance pins. `served` is what the CDN returns
/// for the replay jar; `claimed_sha1` is what the version metadata claims.
fn fake_origin(
    pin: ReplayModPin,
    served: &'static [u8],
    claimed_sha1: Option<String>,
) -> impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static {
    let claimed = claimed_sha1.unwrap_or_else(|| pin.sha1.clone());
    let perf: Vec<(String, String, String, String)> = mods::pins_by_version("26.3")
        .unwrap()
        .into_iter()
        .map(|p| (p.version_id, p.project_id, format!("{}-{}.jar", p.slug, p.version_number), p.version_number))
        .collect();
    let pin_json = json!({
        "id": pin.version_id,
        "project_id": REPLAY_MOD_PROJECT,
        "version_number": pin.version_number,
        "dependencies": [],
        "files": [{
            "primary": true,
            "filename": pin.filename,
            "url": "",
            "hashes": { "sha1": claimed },
            "size": 0,
        }],
    });
    move |req: &MockRequest| {
        let origin = format!("http://{}", req.header("host").unwrap_or_default());
        let path = req.path.clone();
        if let Some(name) = path.strip_prefix("/cdn/") {
            if name == pin.filename {
                return MockResponse::bytes(served.to_vec());
            }
            return MockResponse::bytes(pack_body(name).into_bytes());
        }
        let Some(id) = path.strip_prefix("/v2/version/") else {
            return MockResponse::ok("[]");
        };
        if id == pin.version_id {
            let mut v = pin_json.clone();
            v["files"][0]["url"] = json!(format!("{origin}/cdn/{}", pin.filename));
            return MockResponse::json(&v);
        }
        if let Some((_, project_id, filename, version_number)) = perf.iter().find(|(vid, ..)| vid == id) {
            let body = pack_body(filename);
            return MockResponse::json(&json!({
                "id": id,
                "project_id": project_id,
                "version_number": version_number,
                "dependencies": [],
                "files": [{
                    "primary": true,
                    "filename": filename,
                    "url": format!("{origin}/cdn/{filename}"),
                    "hashes": { "sha1": sha1_of(body.as_bytes()) },
                    "size": body.len(),
                }],
            }));
        }
        MockResponse::status(404)
    }
}

fn opts(pin: ReplayModPin, packs: Vec<ShaderPack>) -> CaptureToolsOptions {
    CaptureToolsOptions { packs: Some(packs), replay_pin: Some(pin) }
}

// ---------------------------------------------------------------------------
// Capture tools behaviour
// ---------------------------------------------------------------------------

#[tokio::test]
async fn install_capture_tools_installs_replay_performance_pins_and_packs() {
    let pin = replay_pin("cap-install-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");
    let pretty = pack_pin("PrettyPack-1.0.zip", &srv.url(""));
    let fast = pack_pin("FastPack-1.0.zip", &srv.url(""));

    let result = install_capture_tools_with(
        &e,
        &capture_meta("26.3", "fabric"),
        &opts(pin.clone(), vec![pretty.clone(), fast.clone()]),
    )
    .await
    .unwrap();

    assert_eq!(result.replay_mod, pin.filename);
    let mods_dir = game_dir(&e).join("mods");
    assert_eq!(read(mods_dir.join(&pin.filename)), String::from_utf8_lossy(REPLAY_BYTES));
    let shaders = game_dir(&e).join("shaderpacks");
    assert_eq!(read(shaders.join(&pretty.filename)), pack_body(&pretty.filename));
    assert_eq!(read(shaders.join(&fast.filename)), pack_body(&fast.filename));
    assert_eq!(result.shaderpacks, vec![pretty.filename.clone(), fast.filename.clone()]);
    // Iris/Sodium arrive with the performance preset.
    let jars = names(&mods_dir);
    assert!(jars.iter().any(|f| f.starts_with("iris-")), "iris installed, got {jars:?}");
    assert!(jars.iter().any(|f| f.starts_with("sodium-")), "sodium installed, got {jars:?}");
    // The prettiest (first) pack is what Iris is pointed at.
    let props = read(game_dir(&e).join("config").join("iris.properties"));
    assert!(props.contains(&format!("shaderPack={}", pretty.filename)), "{props}");
    assert!(props.contains("enableShaders=true"), "{props}");
    assert_eq!(result.iris_pack, pretty.filename);
}

#[tokio::test]
async fn install_capture_tools_discards_a_jar_with_the_wrong_sha1() {
    let pin = replay_pin("cap-tampered-v1");
    // Metadata claims the pin; the CDN serves different bytes.
    let srv = MockServer::start(fake_origin(pin.clone(), b"tampered bytes", None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");

    let err = install_capture_tools_with(&e, &capture_meta("26.3", "fabric"), &opts(pin, vec![])).await.unwrap_err();
    assert!(err.to_string().contains("sha1 mismatch"), "{err}");
    assert!(names(&game_dir(&e).join("mods")).is_empty(), "neither the jar nor a .part survives");
}

#[tokio::test]
async fn install_capture_tools_refuses_metadata_whose_sha1_disagrees_with_the_pin() {
    let pin = replay_pin("cap-rehashed-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, Some("f".repeat(40)))).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");

    let err = install_capture_tools_with(&e, &capture_meta("26.3", "fabric"), &opts(pin, vec![])).await.unwrap_err();
    assert!(err.to_string().contains("but the pin says"), "{err}");
    assert!(names(&game_dir(&e).join("mods")).is_empty());
}

#[tokio::test]
async fn a_second_enable_neither_duplicates_the_jar_nor_overrides_a_chosen_pack() {
    let pin = replay_pin("cap-idempotent-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");
    let inst = capture_meta("26.3", "fabric");
    let packs = vec![pack_pin("PrettyPack-1.0.zip", &srv.url("")), pack_pin("FastPack-1.0.zip", &srv.url(""))];

    install_capture_tools_with(&e, &inst, &opts(pin.clone(), packs.clone())).await.unwrap();
    // The player picks their own pack, then capture tools are enabled again.
    let cfg = game_dir(&e).join("config").join("iris.properties");
    std::fs::write(&cfg, "shaderPack=BSL_v10.1.8.zip\n").unwrap();
    let again = install_capture_tools_with(&e, &inst, &opts(pin.clone(), packs)).await.unwrap();

    let replay: Vec<String> =
        names(&game_dir(&e).join("mods")).into_iter().filter(|f| f.starts_with("replaymod")).collect();
    assert_eq!(replay, vec![pin.filename.clone()], "exactly one Replay Mod jar");
    assert!(read(&cfg).contains("shaderPack=BSL_v10.1.8.zip"), "user selection is never overwritten");
    assert_eq!(again.iris_pack, "BSL_v10.1.8.zip");
}

#[tokio::test]
async fn install_capture_tools_without_a_pin_is_a_clear_400() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "1.20.1", "fabric");
    let err = install_capture_tools(&e, &capture_meta("1.20.1", "fabric")).await.unwrap_err();
    assert_eq!(code(&err), (400, "CAPTURE_TOOLS_UNSUPPORTED".into()));
}

#[tokio::test]
async fn install_capture_tools_rejects_a_non_fabric_instance() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "vanilla");
    let err = install_capture_tools(&e, &capture_meta("26.3", "vanilla")).await.unwrap_err();
    assert_eq!(code(&err), (400, "CAPTURE_TOOLS_UNSUPPORTED".into()));
}

#[tokio::test]
async fn remove_capture_tools_removes_only_replay_mod_jars() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "fabric");
    let mods_dir = game_dir(&e).join("mods");
    for f in [
        "replaymod-26.3-2.6.28.jar",
        "replaymod-26.2-2.6.27.jar.disabled",
        "replaymod.jar",
        "REPLAYMOD-1.0.JAR",
        "iris-mc26.3-1.11.7.jar",
        "sodium-mc26.3-0.9.2.jar",
        "notreplaymod-1.0.jar",
        "replaymodplus-1.0.jar",
    ] {
        std::fs::write(mods_dir.join(f), "x").unwrap();
    }
    let shaders = game_dir(&e).join("shaderpacks");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(shaders.join("BSL_v10.1.8.zip"), "pack").unwrap();

    let mut removed = remove_capture_tools(&e, &capture_meta("26.3", "fabric")).await.unwrap();
    removed.sort();
    assert_eq!(
        removed,
        vec!["REPLAYMOD-1.0.JAR", "replaymod-26.2-2.6.27.jar.disabled", "replaymod-26.3-2.6.28.jar", "replaymod.jar"]
    );
    assert_eq!(
        names(&mods_dir),
        vec!["iris-mc26.3-1.11.7.jar", "notreplaymod-1.0.jar", "replaymodplus-1.0.jar", "sodium-mc26.3-0.9.2.jar"]
    );
    assert!(shaders.join("BSL_v10.1.8.zip").exists(), "shader packs stay");
}

#[tokio::test]
async fn remove_capture_tools_is_a_no_op_without_a_mods_dir() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "fabric");
    std::fs::remove_dir_all(game_dir(&e).join("mods")).unwrap();
    assert!(remove_capture_tools(&e, &capture_meta("1.20.1", "fabric")).await.unwrap().is_empty());
}

#[tokio::test]
async fn install_capture_tools_rolls_back_the_jar_when_a_later_step_fails() {
    let pin = replay_pin("cap-rollback-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");
    // The shader-pack download fails: its pin needs different bytes.
    let broken = ShaderPack {
        filename: "BrokenPack.zip".into(),
        url: srv.url("/cdn/BrokenPack.zip"),
        sha1: "e".repeat(40),
        size: 1,
        ..pack_pin("BrokenPack.zip", &srv.url(""))
    };

    let err = install_capture_tools_with(&e, &capture_meta("26.3", "fabric"), &opts(pin.clone(), vec![broken]))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("downloadAll"), "{err}");
    assert!(!game_dir(&e).join("mods").join(&pin.filename).exists(), "no Replay Mod left with capture_tools false");
    let replay: Vec<String> =
        names(&game_dir(&e).join("mods")).into_iter().filter(|f| f.starts_with("replaymod")).collect();
    assert!(replay.is_empty(), "{replay:?}");
}

#[tokio::test]
async fn install_capture_tools_replaces_a_pre_existing_jar_with_the_wrong_bytes() {
    let pin = replay_pin("cap-stale-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");
    std::fs::write(game_dir(&e).join("mods").join(&pin.filename), "stale bytes from an older build").unwrap();

    install_capture_tools_with(&e, &capture_meta("26.3", "fabric"), &opts(pin.clone(), vec![])).await.unwrap();
    assert_eq!(read(game_dir(&e).join("mods").join(&pin.filename)), String::from_utf8_lossy(REPLAY_BYTES));
}

#[test]
fn capture_pins_match_the_shipped_builds() {
    assert_eq!(REPLAY_MOD_PROJECT, "Nv2fQJo5");
    assert_eq!(REPLAY_MOD_PINNED_VERSIONS.len(), 3);
    for v in REPLAY_MOD_PINNED_VERSIONS {
        let pin = replay_mod_pin(v).expect(v);
        assert_eq!(pin.sha1.len(), 40, "{v}");
        assert!(pin.filename.ends_with(".jar"), "{v}");
    }
    assert!(replay_mod_pin("1.20.1").is_none());
    let packs = capture_shader_packs();
    assert_eq!(packs.len(), CAPTURE_SHADER_PACKS_LEN);
    assert_eq!(packs[0], default_shader_pack());
    assert_eq!(packs[1], fast_shader_pack());
    assert_eq!(default_shader_pack().filename, "ComplementaryReimagined_r5.9.3.zip");
    assert_eq!(fast_shader_pack().filename, "MakeUp-UltraFast-9.6.zip");
}

// ---------------------------------------------------------------------------
// Iris selection
// ---------------------------------------------------------------------------

#[test]
fn select_default_shader_pack_keeps_comments_and_resolves_duplicates_java_style() {
    let (d, _e) = temp_engine();
    let cfg = d.path().join("config").join("iris.properties");
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    std::fs::write(&cfg, "# Iris config, hand-edited\nshaderPack=OFF\ncolorSpace=SRGB\nshaderPack=\n").unwrap();

    let res = select_default_shader_pack(d.path(), &default_shader_pack()).unwrap();
    assert!(res.applied);
    let out = read(&cfg);
    assert!(out.lines().any(|l| l == "# Iris config, hand-edited"), "comments survive: {out}");
    assert!(out.lines().any(|l| l == "colorSpace=SRGB"), "unrelated keys survive: {out}");
    assert_eq!(out.lines().filter(|l| l.starts_with("shaderPack=")).count(), 1, "{out}");
    assert!(out.lines().any(|l| l == format!("shaderPack={}", default_shader_pack().filename)), "{out}");
    assert!(out.lines().any(|l| l == "enableShaders=true"), "{out}");
    let tmp: Vec<String> = names(cfg.parent().unwrap()).into_iter().filter(|f| f.ends_with(".tmp")).collect();
    assert!(tmp.is_empty(), "no temp file left behind: {tmp:?}");
}

#[test]
fn select_default_shader_pack_treats_off_as_unset_and_a_real_pack_as_chosen() {
    let (d, _e) = temp_engine();
    let cfg = d.path().join("config").join("iris.properties");
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    std::fs::write(&cfg, "shaderPack=OFF\n").unwrap();
    assert!(select_default_shader_pack(d.path(), &default_shader_pack()).unwrap().applied);
    std::fs::write(&cfg, "shaderPack=   \n").unwrap();
    assert!(select_default_shader_pack(d.path(), &default_shader_pack()).unwrap().applied, "blank is unset");
    std::fs::write(&cfg, "shaderPack=Photon_v1.4.zip\n").unwrap();
    let kept = select_default_shader_pack(d.path(), &default_shader_pack()).unwrap();
    assert!(!kept.applied);
    assert_eq!(kept.pack, "Photon_v1.4.zip");
}

// ---------------------------------------------------------------------------
// PATCH capture_tools
// ---------------------------------------------------------------------------

/// Point the capture pins of `version` at the mock origin (`serve` synthesises
/// a pin whose jar the fake CDN serves).
fn use_capture_pins(e: &Engine, version: &str, pin: ReplayModPin, packs: Vec<ShaderPack>) {
    let version = version.to_string();
    e.update_content_settings(move |s| {
        s.capture_pins = Some(vec![(version, pin)]);
        s.capture_shader_packs = Some(packs);
    });
}

#[tokio::test]
async fn patch_capture_tools_on_installs_the_jar_then_persists_the_flag() {
    let pin = replay_pin("cap-patch-on-v1");
    let srv = MockServer::start(fake_origin(pin.clone(), REPLAY_BYTES, None)).await;
    let (_t, e) = capture_instance(&srv.url("/v2"), "26.3", "fabric");
    use_capture_pins(
        &e,
        "26.3",
        pin.clone(),
        vec![pack_pin("PrettyPack-1.0.zip", &srv.url("")), pack_pin("FastPack-1.0.zip", &srv.url(""))],
    );

    let summary = e
        .patch_instance("capture", InstancePatch { capture_tools: Some(Some(true)), ..Default::default() })
        .await
        .unwrap();

    assert!(summary.capture_tools);
    assert!(game_dir(&e).join("mods").join(&pin.filename).exists(), "the jar flips with the flag");
    let raw: Value = serde_json::from_str(&read(game_dir(&e).join("instance.json"))).unwrap();
    assert_eq!(raw["capture_tools"], json!(true));
}

#[tokio::test]
async fn patch_capture_tools_failure_leaves_the_flag_off() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "fabric");
    let err = e
        .patch_instance("capture", InstancePatch { capture_tools: Some(Some(true)), ..Default::default() })
        .await
        .unwrap_err();
    assert!(err.status() >= 400, "{err}");
    let raw: Value = serde_json::from_str(&read(game_dir(&e).join("instance.json"))).unwrap();
    assert!(raw.get("capture_tools").is_none(), "{raw}");
    assert!(names(&game_dir(&e).join("mods")).is_empty());
}

#[tokio::test]
async fn patch_capture_tools_off_removes_only_the_replay_mod() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "fabric");
    let mods_dir = game_dir(&e).join("mods");
    std::fs::write(mods_dir.join("replaymod-26.3-2.6.28.jar"), "jar").unwrap();
    std::fs::write(mods_dir.join("iris-mc26.3-1.11.7.jar"), "jar").unwrap();
    let shaders = game_dir(&e).join("shaderpacks");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(shaders.join("BSL_v10.1.8.zip"), "pack").unwrap();
    let mut raw = serde_json::from_str::<Value>(&read(game_dir(&e).join("instance.json"))).unwrap();
    raw["capture_tools"] = json!(true);
    std::fs::write(game_dir(&e).join("instance.json"), raw.to_string()).unwrap();

    let summary = e
        .patch_instance("capture", InstancePatch { capture_tools: Some(Some(false)), ..Default::default() })
        .await
        .unwrap();

    assert!(!summary.capture_tools);
    assert_eq!(names(&mods_dir), vec!["iris-mc26.3-1.11.7.jar"]);
    assert!(shaders.join("BSL_v10.1.8.zip").exists(), "shader packs stay");
}

#[tokio::test]
async fn patch_capture_tools_is_refused_while_the_instance_runs() {
    let (_t, e) = capture_instance("http://127.0.0.1:9/v2", "26.3", "fabric");
    e.active_instances().insert("capture");
    let err = e
        .patch_instance("capture", InstancePatch { capture_tools: Some(Some(true)), ..Default::default() })
        .await
        .unwrap_err();
    assert_eq!(code(&err), (409, "ALREADY_RUNNING".into()));
    assert!(err.to_string().contains("stop it before changing capture tools"), "{err}");

    // Unrelated fields stay patchable while the game runs.
    let ok =
        e.patch_instance("capture", InstancePatch { memory_mb: Some(Some(2048)), ..Default::default() }).await.unwrap();
    assert_eq!(ok.memory_mb, 2048);
    e.active_instances().remove("capture");
}

#[test]
fn instance_meta_reads_the_capture_tools_flag_leniently() {
    let meta: InstanceMeta =
        serde_json::from_value(json!({ "name": "x", "version": "26.3", "capture_tools": true })).unwrap();
    assert!(meta.capture_tools);
    let off: InstanceMeta =
        serde_json::from_value(json!({ "name": "x", "version": "26.3", "capture_tools": "yes" })).unwrap();
    assert!(!off.capture_tools, "JS `=== true`");
    let absent: InstanceMeta = serde_json::from_value(json!({ "name": "x", "version": "26.3" })).unwrap();
    assert!(!absent.capture_tools);
    // Off is not serialized back into instance.json (the raw object keeps it).
    assert!(serde_json::to_value(&absent).unwrap().get("capture_tools").is_none());
}
