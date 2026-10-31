//! In-app updater: the Rust successor of the Tauri updater (tauri-plugin-updater
//! 2.10.1) the 1.6.x launchers shipped with, kept drop-in compatible with the
//! release feed so one `latest.json` serves both the last Tauri clients and
//! this app.
//!
//! Feed: `GET` [`ENDPOINT`] (override: env [`ENDPOINT_ENV`]) answers the Tauri
//! updater format — `{ version, notes, pub_date, platforms: { "<os>-<arch>":
//! { url, signature } } }` (or the dynamic `{ version, url, signature }`
//! shape); `204 No Content` = no update. The platform entry is looked up as
//! `<os>-<arch>-<installer>` first, then `<os>-<arch>` (Tauri's order).
//!
//! Signature: `signature` is base64 of a minisign signature box made by
//! `tauri signer` / `scripts/sign-update.sh`; it is checked against [`PUBKEY`]
//! (key id `B26697BC340F675C`, the same key as the Tauri config) over the
//! downloaded bytes BEFORE anything touches disk. An unverified file is never
//! written, let alone run.
//!
//! Install ([`install`], blocking):
//! - Windows (NSIS): the setup.exe goes to a temp dir and runs with
//!   `/P /R /UPDATE` (passive, relaunch, keep shortcuts — what the Tauri
//!   updater passed); the caller then exits so the installer can replace the
//!   exe ([`Installed::ExitNow`]).
//! - Linux (.deb): like the Tauri updater — `pkexec dpkg -i`, then a zenity /
//!   kdialog password prompt piped to `sudo -S`; the caller exits and starts
//!   the new binary ([`Installed::RelaunchAfterExit`]).
//!
//! A copy that was not installed by one of those (dev build, portable exe,
//! hand-extracted files) has no [`Installer`]: the app does not offer updates.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use crate::download::HttpClient;
use crate::engine::VERSION;
use crate::error::{EngineError, EngineResult};

/// The release feed (same URL as the Tauri config's `plugins.updater.endpoints`).
pub const ENDPOINT: &str = "https://espectral.uhce.top/client/latest.json";

/// Env override for [`ENDPOINT`] (test feeds; the signature key is fixed).
pub const ENDPOINT_ENV: &str = "ESPECTRAL_UPDATE_ENDPOINT";

/// Base64 of the minisign public key (`plugins.updater.pubkey` of the Tauri config).
pub const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEIyNjY5N0JDMzQwRjY3NUMKUldSY1p3ODB2SmRtc3Q3dm91TmJJTkhsQXorV01TU0NrTklDd3JlY1d0RU9pK1FkR093elV4UU8K";

/// The Tauri updater's check timeout (the Svelte UI passed 15 s).
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// Download: header timeout and per-chunk stall timeout.
const STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Debian package name (`/var/lib/dpkg/info/<name>.list`).
const DEB_PACKAGE: &str = "espectral-client";
/// Where the .deb puts the binary.
const DEB_BINARY: &str = "/usr/bin/espectral-client";

/// How this copy was installed — which package the update must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    Nsis,
    Deb,
}

impl Installer {
    /// The `<installer>` suffix of a platform key (Tauri names).
    pub fn name(self) -> &'static str {
        match self {
            Installer::Nsis => "nsis",
            Installer::Deb => "deb",
        }
    }

    /// The installer that owns the running executable, if any.
    pub fn detect() -> Option<Installer> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(windows) {
            // The NSIS installer drops uninstall.exe next to the app.
            exe.parent().is_some_and(|d| d.join("uninstall.exe").is_file()).then_some(Installer::Nsis)
        } else if cfg!(target_os = "linux") {
            let dpkg = Path::new("/var/lib/dpkg/info");
            let listed = dpkg.join(format!("{DEB_PACKAGE}.list")).is_file()
                || dpkg.join(format!("{DEB_PACKAGE}:amd64.list")).is_file();
            (strip_deleted(&exe) == Path::new(DEB_BINARY) && listed).then_some(Installer::Deb)
        } else {
            None
        }
    }
}

/// `/proc/self/exe` reads `<path> (deleted)` once dpkg replaced the file.
fn strip_deleted(exe: &Path) -> PathBuf {
    let s = exe.to_string_lossy();
    PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s))
}

/// `<os>-<arch>` as the Tauri updater spells it.
fn target() -> Option<String> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "windows"
    } else {
        return None;
    };
    let arch = match std::env::consts::ARCH {
        "x86" => "i686",
        "arm" => "armv7",
        a @ ("x86_64" | "aarch64" | "riscv64") => a,
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

/// An update the feed offers (newer than the running version).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub url: String,
    /// Base64 minisign signature box (the `.sig` file content).
    pub signature: String,
}

#[derive(Deserialize)]
struct PlatformEntry {
    url: String,
    signature: String,
}

#[derive(Deserialize)]
struct Feed {
    #[serde(alias = "name")]
    version: String,
    notes: Option<String>,
    pub_date: Option<String>,
    platforms: Option<serde_json::Map<String, Value>>,
    url: Option<String>,
    signature: Option<String>,
}

/// Checks the feed and downloads verified packages.
#[derive(Debug, Clone)]
pub struct Updater {
    client: reqwest::Client,
    endpoint: String,
    pubkey: String,
    current: semver::Version,
    installer: Option<Installer>,
}

impl Updater {
    /// The production updater: [`ENDPOINT`] (or [`ENDPOINT_ENV`]), [`PUBKEY`],
    /// this build's version, the detected installer.
    pub fn new(http: &HttpClient) -> Self {
        let endpoint = std::env::var(ENDPOINT_ENV).ok().filter(|s| !s.trim().is_empty());
        Self::with(http.client().clone(), endpoint.as_deref().unwrap_or(ENDPOINT), PUBKEY, VERSION, Installer::detect())
            .expect("the crate version is valid semver")
    }

    /// Explicit construction (tests, test feeds).
    pub fn with(
        client: reqwest::Client,
        endpoint: &str,
        pubkey: &str,
        current_version: &str,
        installer: Option<Installer>,
    ) -> EngineResult<Self> {
        Ok(Updater {
            client,
            endpoint: endpoint.to_string(),
            pubkey: pubkey.to_string(),
            current: parse_version(current_version)?,
            installer,
        })
    }

    pub fn installer(&self) -> Option<Installer> {
        self.installer
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Whether the feed's endpoint was overridden ([`ENDPOINT_ENV`]).
    pub fn is_test_feed(&self) -> bool {
        self.endpoint != ENDPOINT
    }

    /// Ask the feed; `Some` only when it offers a newer version for this platform.
    pub async fn check(&self) -> EngineResult<Option<Update>> {
        let res = self
            .client
            .get(&self.endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .timeout(CHECK_TIMEOUT)
            .send()
            .await
            .map_err(|e| EngineError::other(format!("update check failed: {e}")))?;
        if res.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        if !res.status().is_success() {
            return Err(EngineError::other(format!("update check failed: HTTP {}", res.status().as_u16())));
        }
        let body = res.bytes().await.map_err(|e| EngineError::other(format!("update check failed: {e}")))?;
        self.parse_feed(&body)
    }

    fn parse_feed(&self, body: &[u8]) -> EngineResult<Option<Update>> {
        let feed: Feed =
            serde_json::from_slice(body).map_err(|e| EngineError::other(format!("invalid update feed: {e}")))?;
        let version = parse_version(&feed.version)?;
        if version <= self.current {
            return Ok(None);
        }
        let (url, signature) = match &feed.platforms {
            Some(platforms) => {
                let base = target().ok_or_else(|| EngineError::other("updates are not supported on this platform"))?;
                let mut keys = Vec::with_capacity(2);
                if let Some(i) = self.installer {
                    keys.push(format!("{base}-{}", i.name()));
                }
                keys.push(base);
                let entry = keys
                    .iter()
                    .find_map(|k| {
                        platforms.get(k).and_then(|v| serde_json::from_value::<PlatformEntry>(v.clone()).ok())
                    })
                    .ok_or_else(|| {
                        EngineError::other(format!("the update feed has no entry for {}", keys.join(" / ")))
                    })?;
                (entry.url, entry.signature)
            }
            None => match (feed.url, feed.signature) {
                (Some(u), Some(s)) => (u, s),
                _ => return Err(EngineError::other("invalid update feed: no platforms and no url/signature")),
            },
        };
        Ok(Some(Update { version: version.to_string(), notes: feed.notes, pub_date: feed.pub_date, url, signature }))
    }

    /// Download the package and verify its signature. Returns the verified
    /// bytes; `on_progress(received, total)` runs per chunk.
    pub async fn download(
        &self,
        update: &Update,
        mut on_progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> EngineResult<Vec<u8>> {
        let fail = |e: &dyn std::fmt::Display| EngineError::other(format!("update download failed: {e}"));
        let res = tokio::time::timeout(
            STALL_TIMEOUT,
            self.client.get(&update.url).header(reqwest::header::ACCEPT, "application/octet-stream").send(),
        )
        .await
        .map_err(|_| fail(&"timed out"))?
        .map_err(|e| fail(&e))?;
        if !res.status().is_success() {
            return Err(fail(&format!("HTTP {}", res.status().as_u16())));
        }
        let total = res.content_length();
        let mut buf = Vec::with_capacity(total.unwrap_or(0).min(512 * 1024 * 1024) as usize);
        let mut stream = res.bytes_stream();
        loop {
            match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
                Err(_) => return Err(fail(&"stalled")),
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(fail(&e)),
                Ok(Some(Ok(chunk))) => {
                    buf.extend_from_slice(&chunk);
                    on_progress(buf.len() as u64, total);
                }
            }
        }
        verify_signature(&buf, &update.signature, &self.pubkey)?;
        Ok(buf)
    }
}

fn parse_version(v: &str) -> EngineResult<semver::Version> {
    semver::Version::parse(v.trim().trim_start_matches('v'))
        .map_err(|e| EngineError::other(format!("invalid version '{v}': {e}")))
}

/// Verify `data` against a base64 minisign signature box and a base64 public
/// key box (the Tauri `.sig` / `pubkey` encodings). Legacy (non-prehashed)
/// signatures are accepted like the Tauri updater does.
pub fn verify_signature(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> EngineResult<()> {
    use base64::Engine as _;
    let decode = |what: &str, s: &str| -> EngineResult<String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map_err(|e| EngineError::other(format!("update {what} is not base64: {e}")))?;
        String::from_utf8(bytes).map_err(|_| EngineError::other(format!("update {what} is not UTF-8")))
    };
    let pk = minisign_verify::PublicKey::decode(&decode("public key", pubkey_b64)?)
        .map_err(|e| EngineError::other(format!("invalid update public key: {e}")))?;
    let sig = minisign_verify::Signature::decode(&decode("signature", signature_b64)?)
        .map_err(|e| EngineError::other(format!("invalid update signature: {e}")))?;
    pk.verify(data, &sig, true)
        .map_err(|e| EngineError::other(format!("the update's signature does not match ({e}); not installing it")))
}

/// What the app must do once [`install`] returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// Windows: the installer is running and relaunches the app; exit now.
    ExitNow,
    /// Linux: the package is installed; exit, then start this binary.
    RelaunchAfterExit(PathBuf),
}

/// Install verified package bytes (from [`Updater::download`]). Blocking:
/// run it off the async runtime.
pub fn install(update: &Update, installer: Installer, bytes: &[u8]) -> EngineResult<Installed> {
    match installer {
        Installer::Nsis => install_nsis(update, bytes),
        Installer::Deb => install_deb(bytes),
    }
}

fn temp_dir(prefix: &str) -> EngineResult<PathBuf> {
    Ok(tempfile::Builder::new().prefix(prefix).tempdir()?.keep())
}

fn install_nsis(update: &Update, bytes: &[u8]) -> EngineResult<Installed> {
    if !bytes.starts_with(b"MZ") {
        return Err(EngineError::other("the update is not a Windows installer"));
    }
    let dir = temp_dir(&format!("espectral-client-{}-updater-", update.version))?;
    let setup = dir.join(format!("EspectralClient_{}_x64-setup.exe", update.version));
    std::fs::write(&setup, bytes)?;
    // Passive UI + relaunch + update mode: exactly what the Tauri updater ran.
    Command::new(&setup)
        .args(["/P", "/R", "/UPDATE"])
        .spawn()
        .map_err(|e| EngineError::other(format!("could not start the installer: {e}")))?;
    Ok(Installed::ExitNow)
}

fn install_deb(bytes: &[u8]) -> EngineResult<Installed> {
    if !bytes.starts_with(b"!<arch>\ndebian-binary") {
        return Err(EngineError::other("the update is not a Debian package"));
    }
    let exe = std::env::current_exe().map(|p| strip_deleted(&p)).unwrap_or_else(|_| PathBuf::from(DEB_BINARY));
    let dir = tempfile::Builder::new().prefix("espectral-client-deb-update").tempdir()?;
    let pkg = dir.path().join("package.deb");
    std::fs::write(&pkg, bytes)?;

    // 1. pkexec (polkit's graphical prompt)
    if Command::new("pkexec").arg("dpkg").arg("-i").arg(&pkg).status().is_ok_and(|s| s.success()) {
        return Ok(Installed::RelaunchAfterExit(exe));
    }
    // 2. zenity / kdialog password prompt -> sudo -S
    if let Some(password) = ask_password() {
        if sudo_install(&pkg, &password)? {
            return Ok(Installed::RelaunchAfterExit(exe));
        }
    }
    Err(EngineError::other("the update package could not be installed (administrator authorization failed)"))
}

fn ask_password() -> Option<String> {
    let prompts: [(&str, &[&str]); 2] = [
        (
            "zenity",
            &[
                "--password",
                "--title=Espectral Client",
                "--text=Contraseña para instalar la actualización / Password to install the update:",
            ],
        ),
        ("kdialog", &["--password", "Contraseña para instalar la actualización / Password to install the update:"]),
    ];
    prompts.iter().find_map(|(cmd, args)| {
        let out = Command::new(cmd).args(*args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_string())
    })
}

fn sudo_install(pkg: &Path, password: &str) -> EngineResult<bool> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "dpkg", "-i"])
        .arg(pkg)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{password}")?;
    }
    Ok(child.wait()?.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MockResponse, MockServer};
    use serde_json::json;

    // Throwaway Tauri-format key pair and a signature made with the minisign
    // CLI exactly like scripts/sign-update.sh (prehashed, tauri comments).
    const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY2NjQ2N0FGOEE1MUU3QzAKUldUQTUxR0tyMmRrOW9yU0haQjdsQngrTGdvWitCckkxczg5VnN6UzR2a3dSZ1g1VG54QitEUGoK";
    const TEST_PAYLOAD: &[u8] = b"espectral-update-test-payload";
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUQTUxR0tyMmRrOXFLNHhsYkM3eUptck5zREUva3ZpdUdHOHNrTG9OaXNZOUw5Y1VUSGh3dWlWSVVqODVSTytHRXpOc0E3NFNJUldLN2NUbS9VbmhKQW1RMDdyT2dmcmdJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzYxOTAwMDAwCWZpbGU6dHYuYmluCjRxSVpkSE5CcGwxczZoN24xSVNDS1dLVkEwckJ3bFlYTUNPNkhta3dSN2dNMVNpTlpHeW9XM2NjMkRkTUtvanlGMFRMZHF2MlFzQS8waEI4dkJPVEF3PT0K";

    fn updater(endpoint: &str, current: &str, installer: Option<Installer>) -> Updater {
        Updater::with(reqwest::Client::new(), endpoint, TEST_PUBKEY, current, installer).unwrap()
    }

    fn base() -> String {
        target().unwrap()
    }

    #[test]
    fn signature_accepts_the_signed_bytes_only() {
        verify_signature(TEST_PAYLOAD, TEST_SIG, TEST_PUBKEY).unwrap();
        assert!(verify_signature(b"espectral-update-test-payloaD", TEST_SIG, TEST_PUBKEY).is_err());
        // right signature, wrong key (the production key)
        assert!(verify_signature(TEST_PAYLOAD, TEST_SIG, PUBKEY).is_err());
        assert!(verify_signature(TEST_PAYLOAD, "not base64!", TEST_PUBKEY).is_err());
    }

    #[test]
    fn production_key_decodes() {
        use base64::Engine as _;
        let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(PUBKEY).unwrap()).unwrap();
        assert!(text.starts_with("untrusted comment: minisign public key: B26697BC340F675C"));
        minisign_verify::PublicKey::decode(&text).unwrap();
    }

    #[test]
    fn feed_offers_only_newer_versions() {
        let u = updater("http://x", "1.7.0", None);
        let feed = |v: &str| {
            json!({ "version": v, "platforms": { base(): { "url": "https://h/p", "signature": "s" } } }).to_string()
        };
        assert_eq!(u.parse_feed(feed("1.7.0").as_bytes()).unwrap(), None);
        assert_eq!(u.parse_feed(feed("1.6.3").as_bytes()).unwrap(), None);
        assert_eq!(u.parse_feed(feed("1.7.1-beta.1").as_bytes()).unwrap().unwrap().version, "1.7.1-beta.1");
        let up = u.parse_feed(feed("v1.8.0").as_bytes()).unwrap().unwrap();
        assert_eq!((up.version.as_str(), up.url.as_str(), up.signature.as_str()), ("1.8.0", "https://h/p", "s"));
        // a prerelease of the running version is older
        assert_eq!(updater("http://x", "1.7.0", None).parse_feed(feed("1.7.0-rc.1").as_bytes()).unwrap(), None);
    }

    #[test]
    fn installer_specific_entry_wins_then_generic() {
        let feed = json!({
            "version": "2.0.0",
            "platforms": {
                base(): { "url": "https://h/generic", "signature": "g" },
                format!("{}-deb", base()): { "url": "https://h/deb", "signature": "d" },
            }
        })
        .to_string();
        let deb = updater("http://x", "1.7.0", Some(Installer::Deb)).parse_feed(feed.as_bytes()).unwrap().unwrap();
        assert_eq!(deb.url, "https://h/deb");
        let nsis = updater("http://x", "1.7.0", Some(Installer::Nsis)).parse_feed(feed.as_bytes()).unwrap().unwrap();
        assert_eq!(nsis.url, "https://h/generic");
        let other = json!({ "version": "2.0.0", "platforms": { "plan9-mips": { "url": "u", "signature": "s" } } });
        let err = updater("http://x", "1.7.0", None).parse_feed(other.to_string().as_bytes()).unwrap_err();
        assert!(err.to_string().contains(&base()), "{err}");
    }

    #[test]
    fn dynamic_feed_shape() {
        let feed = json!({ "version": "9.0.0", "url": "https://h/x", "signature": "s", "notes": "n" }).to_string();
        let up = updater("http://x", "1.7.0", None).parse_feed(feed.as_bytes()).unwrap().unwrap();
        assert_eq!((up.url.as_str(), up.notes.as_deref()), ("https://h/x", Some("n")));
    }

    #[tokio::test]
    async fn check_then_download_verified_package() {
        // package server, then a feed pointing at it
        let srv = MockServer::start(|req| match req.path.as_str() {
            "/pkg" => MockResponse::bytes(TEST_PAYLOAD.to_vec()),
            "/tampered" => MockResponse::bytes(b"evil".to_vec()),
            _ => MockResponse::status(404),
        })
        .await;
        let pkg = srv.url("/pkg");
        let feed_srv = MockServer::start(move |_| {
            MockResponse::json(&json!({
                "version": "1.7.1",
                "notes": "fixes",
                "pub_date": "2026-11-01T00:00:00Z",
                "platforms": { target().unwrap(): { "url": pkg, "signature": TEST_SIG } }
            }))
        })
        .await;
        let u = updater(&feed_srv.url("/latest.json"), "1.7.0", None);
        let up = u.check().await.unwrap().unwrap();
        assert_eq!(up.version, "1.7.1");
        let mut seen = vec![];
        let bytes = u.download(&up, |r, t| seen.push((r, t))).await.unwrap();
        assert_eq!(bytes, TEST_PAYLOAD);
        assert_eq!(seen.last(), Some(&(TEST_PAYLOAD.len() as u64, Some(TEST_PAYLOAD.len() as u64))));

        let tampered = Update { url: srv.url("/tampered"), ..up.clone() };
        let err = u.download(&tampered, |_, _| {}).await.unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
        let missing = Update { url: srv.url("/nope"), ..up };
        assert!(u.download(&missing, |_, _| {}).await.unwrap_err().to_string().contains("404"));

        // up to date / 204 / server error
        assert_eq!(updater(&feed_srv.url("/"), "1.7.1", None).check().await.unwrap(), None);
        let none = MockServer::start(|_| MockResponse::status(204)).await;
        assert_eq!(updater(&none.url("/"), "1.7.0", None).check().await.unwrap(), None);
        let down = MockServer::start(|_| MockResponse::status(503)).await;
        assert!(updater(&down.url("/"), "1.7.0", None).check().await.is_err());
    }

    #[test]
    fn install_refuses_the_wrong_package_kind() {
        let up =
            Update { version: "9.9.9".into(), notes: None, pub_date: None, url: "u".into(), signature: "s".into() };
        assert!(install(&up, Installer::Nsis, b"!<arch>\ndebian-binary").is_err());
        assert!(install(&up, Installer::Deb, b"MZ\x90\x00").is_err());
    }

    #[test]
    fn deleted_suffix_is_stripped() {
        assert_eq!(strip_deleted(Path::new("/usr/bin/espectral-client (deleted)")), Path::new(DEB_BINARY));
        assert_eq!(strip_deleted(Path::new(DEB_BINARY)), Path::new(DEB_BINARY));
    }
}
