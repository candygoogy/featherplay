use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::sync::LockExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ResolveError;

// The releases a first run installs, pinned with their sha256. A checksum file fetched
// from the same "latest" release would only catch a broken download, not a replaced
// release. Each hash is the one in that release's own checksum file (yt-dlp's
// `SHA2-256SUMS`, deno's `.sha256sum`), and matched the downloaded binaries on 2026-11-03.
// yt-dlp then keeps itself current with `yt-dlp -U`, which fetches the build of its own
// variant (`yt-dlp_arm64.exe` for the ARM64 one), whatever the file is called here.
// Both architectures pin the same versions; `.github/workflows/sidecar-pins.yml` checks that.

/// The helpers one architecture's build downloads.
struct Pins {
    ytdlp_url: &'static str,
    ytdlp_sha256: &'static str,
    /// The zip's file name; its download is kept as `{DENO_ZIP}.part`.
    deno_zip: &'static str,
    deno_url: &'static str,
    deno_sha256: &'static str,
}

#[cfg(any(test, not(target_arch = "aarch64")))]
const X64: Pins = Pins {
    ytdlp_url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe",
    ytdlp_sha256: "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d",
    deno_zip: "deno-x86_64-pc-windows-msvc.zip",
    deno_url: "https://github.com/denoland/deno/releases/download/v2.9.8/deno-x86_64-pc-windows-msvc.zip",
    deno_sha256: "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf",
};

#[cfg(any(test, target_arch = "aarch64"))]
const ARM64: Pins = Pins {
    ytdlp_url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp_arm64.exe",
    ytdlp_sha256: "0066dbac3c124028e2dcae2a197fccee33cd91f456313d24f7af89fd54590e98",
    deno_zip: "deno-aarch64-pc-windows-msvc.zip",
    deno_url: "https://github.com/denoland/deno/releases/download/v2.9.8/deno-aarch64-pc-windows-msvc.zip",
    deno_sha256: "b413b92b735b50c198e8fe3b85af1aa50e29d5f72e229aab701d132286e14cef",
};

/// The native helpers of the architecture this exe is built for, so an ARM64 featherplay
/// doesn't run them under x64 emulation.
#[cfg(target_arch = "aarch64")]
const PINS: Pins = ARM64;
#[cfg(not(target_arch = "aarch64"))]
const PINS: Pins = X64;
const DENO_ZIP: &str = PINS.deno_zip;
/// GitHub sends release downloads on to a host under this domain
/// (`release-assets.githubusercontent.com` in Oct 2026).
const ASSET_DOMAIN: &str = ".githubusercontent.com";

/// Touched after every successful `yt-dlp -U`, and when yt-dlp is downloaded: the pinned
/// release is new enough for its first day, then `update_if_stale` keeps it current.
const UPDATE_STAMP: &str = "yt-dlp.updated";
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(120);
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that can't connect, or stops sending, for this long fails.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One file a first run downloads, and the sha256 it must have.
struct Asset {
    url: String,
    sha256: String,
}

/// Where the sidecars are downloaded from: the pinned GitHub releases, over HTTPS, with
/// redirects only to GitHub's download hosts. Tests use a local server instead.
pub struct Downloads {
    ytdlp: Asset,
    /// A zip with `deno.exe` in it.
    deno: Asset,
    /// The test server on 127.0.0.1, over plain HTTP.
    #[cfg(test)]
    local: bool,
}

impl Downloads {
    /// The releases pinned above.
    pub fn pinned() -> Downloads {
        Downloads {
            ytdlp: Asset {
                url: PINS.ytdlp_url.into(),
                sha256: PINS.ytdlp_sha256.into(),
            },
            deno: Asset {
                url: PINS.deno_url.into(),
                sha256: PINS.deno_sha256.into(),
            },
            #[cfg(test)]
            local: false,
        }
    }

    /// `{base}/yt-dlp.exe` and `{base}/deno.zip` from a test server on 127.0.0.1.
    #[cfg(test)]
    pub(crate) fn local(base: &str, ytdlp_sha256: &str, deno_sha256: &str) -> Downloads {
        Downloads {
            ytdlp: Asset {
                url: format!("{base}/yt-dlp.exe"),
                sha256: ytdlp_sha256.into(),
            },
            deno: Asset {
                url: format!("{base}/deno.zip"),
                sha256: deno_sha256.into(),
            },
            local: true,
        }
    }

    /// A URL these downloads may fetch, first or after a redirect.
    fn allows(&self, url: &reqwest::Url) -> bool {
        #[cfg(test)]
        if self.local {
            return url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
        }
        download_host(url)
    }

    /// HTTPS only, redirects only to GitHub's hosts, and timeouts so a dead connection
    /// fails the download instead of holding the resolves waiting on it. The client goes
    /// with the download, and its timers with it.
    fn client(&self) -> Result<reqwest::Client, ResolveError> {
        let builder = reqwest::Client::builder()
            .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT);
        #[cfg(test)]
        if self.local {
            return builder
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| sidecar_err("http client", e));
        }
        builder
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && download_host(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect away from GitHub refused")
                }
            }))
            .build()
            .map_err(|e| sidecar_err("http client", e))
    }
}

/// Downloads the missing sidecars one download at a time: a caller that comes while one
/// runs waits for it and shares its outcome instead of starting another. A failed download
/// is tried again by the next caller that comes after it, so there's no retry loop.
pub(crate) struct Installer {
    downloads: Downloads,
    running: tokio::sync::Mutex<()>,
    /// Downloads tried so far, so a caller that waited can tell one ran.
    tried: AtomicU64,
    /// How the last download failed.
    failed: Mutex<Option<ResolveError>>,
}

impl Installer {
    pub(crate) fn new(downloads: Downloads) -> Installer {
        Installer {
            downloads,
            running: tokio::sync::Mutex::new(()),
            tried: AtomicU64::new(0),
            failed: Mutex::new(None),
        }
    }

    /// Returns at once when both sidecars are there; else downloads them, or waits for the
    /// download running.
    pub(crate) async fn install(&self, sidecars: &Sidecars) -> Result<(), ResolveError> {
        if sidecars.installed() {
            return Ok(());
        }
        let before = self.tried.load(Ordering::SeqCst);
        let _held = self.running.lock().await;
        if sidecars.installed() {
            return Ok(());
        }
        if self.tried.load(Ordering::SeqCst) != before {
            // The download this call waited for failed: its error, not another try at once.
            return Err(self
                .failed
                .lock_or_recover()
                .clone()
                .unwrap_or_else(|| sidecar_err("downloading yt-dlp and deno", "failed")));
        }
        log::info!(
            "yt-dlp or deno is missing: downloading them into {}",
            sidecars.dir.display()
        );
        let started = Instant::now();
        let result = sidecars
            .install(&self.downloads, |name, done, total| {
                if done == 0 {
                    log::info!("downloading {name} ({:.1} MB)", total as f64 / 1e6);
                }
            })
            .await;
        match &result {
            Ok(()) => log::info!(
                "yt-dlp and deno downloaded in {:.1}s",
                started.elapsed().as_secs_f64()
            ),
            Err(e) => log::warn!("downloading yt-dlp and deno failed: {e}"),
        }
        *self.failed.lock_or_recover() = result.as_ref().err().cloned();
        self.tried.fetch_add(1, Ordering::SeqCst);
        result
    }
}

/// The yt-dlp and deno executables in `{cache}/sidecars`.
pub struct Sidecars {
    pub dir: PathBuf,
}

impl Sidecars {
    /// Downloads whichever sidecar is missing from the pinned releases and checks it
    /// against its pinned sha256. `progress(name, bytes_done, bytes_total)`; `bytes_total`
    /// is 0 when the server doesn't say.
    pub async fn ensure(&self, progress: impl Fn(&str, u64, u64)) -> Result<(), ResolveError> {
        self.install(&Downloads::pinned(), progress).await
    }

    /// Both sidecars are there.
    pub fn installed(&self) -> bool {
        self.ytdlp().is_file() && self.deno().is_file()
    }

    /// `ensure`, from `from`.
    pub(crate) async fn install(
        &self,
        from: &Downloads,
        progress: impl Fn(&str, u64, u64),
    ) -> Result<(), ResolveError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| sidecar_err("creating the sidecar dir", e))?;
        let http = from.client()?;

        if !self.ytdlp().is_file() {
            let part = self.dir.join("yt-dlp.exe.part");
            download(&http, from, &from.ytdlp.url, &part, "yt-dlp", &progress).await?;
            verify_or_delete(&part, &from.ytdlp.sha256).await?;
            tokio::fs::rename(&part, self.ytdlp())
                .await
                .map_err(|e| sidecar_err("installing yt-dlp.exe", e))?;
            // The pinned release counts as today's update: no `yt-dlp -U` right after it.
            touch(&self.dir.join(UPDATE_STAMP)).await;
        }

        if !self.deno().is_file() {
            let zip = self.dir.join(format!("{DENO_ZIP}.part"));
            download(&http, from, &from.deno.url, &zip, "deno", &progress).await?;
            verify_or_delete(&zip, &from.deno.sha256).await?;
            let exe_part = self.dir.join("deno.exe.part");
            let (z, e) = (zip.clone(), exe_part.clone());
            let extracted = tokio::task::spawn_blocking(move || extract_deno(&z, &e))
                .await
                .map_err(|e| sidecar_err("extracting deno", e))
                .and_then(|r| r);
            let _ = tokio::fs::remove_file(&zip).await;
            if let Err(e) = extracted {
                let _ = tokio::fs::remove_file(&exe_part).await;
                return Err(e);
            }
            tokio::fs::rename(&exe_part, self.deno())
                .await
                .map_err(|e| sidecar_err("installing deno.exe", e))?;
        }
        Ok(())
    }

    /// Runs `yt-dlp -U` if the last update was more than 24 h ago.
    pub async fn update_if_stale(&self) -> Result<(), ResolveError> {
        self.update(false).await
    }

    /// The yt-dlp and deno versions, from their `--version`; `None` for one that isn't
    /// installed or didn't answer.
    pub async fn versions(&self) -> (Option<String>, Option<String>) {
        (
            self.version_of(&self.ytdlp()).await,
            self.version_of(&self.deno()).await,
        )
    }

    async fn version_of(&self, exe: &Path) -> Option<String> {
        if !exe.is_file() {
            return None;
        }
        let mut cmd = self.command(exe);
        cmd.arg("--version");
        let out = tokio::time::timeout(VERSION_TIMEOUT, cmd.output())
            .await
            .ok()?
            .ok()?;
        if !out.status.success() {
            return None;
        }
        version_line(&String::from_utf8_lossy(&out.stdout))
    }

    pub fn ytdlp(&self) -> PathBuf {
        self.dir.join("yt-dlp.exe")
    }

    pub fn deno(&self) -> PathBuf {
        self.dir.join("deno.exe")
    }

    /// `{cache}`, the parent of the sidecar dir. yt-dlp's cache, deno's cache and `tmp/` live there.
    pub(crate) fn home(&self) -> PathBuf {
        match self.dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => self.dir.clone(),
        }
    }

    /// The last successful `yt-dlp -U` is under 24 h old.
    pub(crate) fn updated_recently(&self) -> bool {
        is_recent(&self.dir.join(UPDATE_STAMP))
    }

    /// `yt-dlp -U`, skipped when `force` is false and the stamp is under 24 h old.
    pub(crate) async fn update(&self, force: bool) -> Result<(), ResolveError> {
        let stamp = self.dir.join(UPDATE_STAMP);
        if !force && is_recent(&stamp) {
            return Ok(());
        }
        if !self.ytdlp().is_file() {
            return Err(ResolveError::ToolMissing);
        }
        let mut cmd = self.command(&self.ytdlp());
        cmd.args(["--ignore-config", "-U"]);
        if let Some(p) = crate::proxy::https_proxy()
            .as_ref()
            .and_then(crate::Proxy::argument)
        {
            cmd.arg("--proxy").arg(p);
        }
        let out = match tokio::time::timeout(UPDATE_TIMEOUT, cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(ResolveError::ToolFailed {
                    code: -1,
                    stderr_tail: format!("yt-dlp -U: {e}"),
                });
            }
            Err(_) => {
                return Err(ResolveError::ToolFailed {
                    code: -1,
                    stderr_tail: "yt-dlp -U timed out".into(),
                });
            }
        };
        if !out.status.success() {
            return Err(ResolveError::ToolFailed {
                code: out.status.code().unwrap_or(-1),
                stderr_tail: tail(&String::from_utf8_lossy(&out.stderr), 600),
            });
        }
        touch(&stamp).await;
        Ok(())
    }

    /// A sidecar command with no console window, no stdin, killed if its future is dropped,
    /// and deno's cache kept under `{cache}` instead of %LOCALAPPDATA%.
    pub(crate) fn command(&self, program: &Path) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(program);
        cmd.env("DENO_DIR", self.home().join("deno-dir"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
}

/// The version a sidecar's `--version` printed: yt-dlp prints `2026.10.26`, deno
/// `deno 2.9.8 (stable, release, x86_64-pc-windows-msvc)` and two more lines.
fn version_line(stdout: &str) -> Option<String> {
    let first = stdout.lines().map(str::trim).find(|l| !l.is_empty())?;
    let version = first.strip_prefix("deno ").unwrap_or(first);
    let version = version.split(" (").next().unwrap_or(version).trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// The last `max` bytes of `s`, cut on a char boundary.
pub(crate) fn tail(s: &str, max: usize) -> String {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].trim().to_string()
}

/// Marks the update stamp as made now. Its time is set, not left to the write: an empty file
/// written over an empty one keeps its old time on Windows.
async fn touch(stamp: &Path) {
    let stamp = stamp.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        std::fs::File::create(&stamp)?.set_modified(SystemTime::now())
    })
    .await;
}

fn is_recent(stamp: &Path) -> bool {
    std::fs::metadata(stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < UPDATE_EVERY)
}

fn sidecar_err(what: &str, e: impl std::fmt::Display) -> ResolveError {
    ResolveError::Sidecar(format!("{what}: {e}"))
}

/// HTTPS on github.com or GitHub's release download hosts. Nothing secret is sent; the
/// pinned sha256 is what vouches for the file.
fn download_host(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|h| h == "github.com" || h.ends_with(ASSET_DOMAIN))
}

async fn download(
    http: &reqwest::Client,
    from: &Downloads,
    url: &str,
    dest: &Path,
    name: &str,
    progress: &impl Fn(&str, u64, u64),
) -> Result<(), ResolveError> {
    if !reqwest::Url::parse(url).is_ok_and(|u| from.allows(&u)) {
        return Err(sidecar_err(url, "not a GitHub download"));
    }
    let result = async {
        let mut resp = http
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| sidecar_err(url, e))?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| sidecar_err("creating the download file", e))?;
        let mut done = 0u64;
        progress(name, 0, total);
        while let Some(chunk) = resp.chunk().await.map_err(|e| sidecar_err(url, e))? {
            file.write_all(&chunk)
                .await
                .map_err(|e| sidecar_err("writing the download", e))?;
            done += chunk.len() as u64;
            progress(name, done, total);
        }
        file.flush()
            .await
            .map_err(|e| sidecar_err("writing the download", e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// Checks `path`'s sha256 against `expected_hex` (any case). On a mismatch the file is deleted.
async fn verify_or_delete(path: &Path, expected_hex: &str) -> Result<(), ResolveError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let result = match sha256_file(path).await {
        Ok(actual) if actual.eq_ignore_ascii_case(expected_hex.trim()) => return Ok(()),
        Ok(actual) => sidecar_err(
            &format!("sha256 mismatch for {name}"),
            format!("expected {expected_hex}, got {actual}"),
        ),
        Err(e) => sidecar_err(&format!("reading {name} for its sha256"), e),
    };
    let _ = tokio::fs::remove_file(path).await;
    Err(result)
}

async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn extract_deno(zip_path: &Path, exe_dest: &Path) -> Result<(), ResolveError> {
    let file = std::fs::File::open(zip_path).map_err(|e| sidecar_err("opening the deno zip", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| sidecar_err("reading the deno zip", e))?;
    let mut entry = archive
        .by_name("deno.exe")
        .map_err(|e| sidecar_err("deno.exe in the zip", e))?;
    let mut out =
        std::fs::File::create(exe_dest).map_err(|e| sidecar_err("creating deno.exe", e))?;
    std::io::copy(&mut entry, &mut out).map_err(|e| sidecar_err("extracting deno.exe", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_stay_on_github_over_https() {
        let ok = |u: &str| download_host(&reqwest::Url::parse(u).unwrap());
        for pins in [&X64, &ARM64] {
            assert!(ok(pins.ytdlp_url) && ok(pins.deno_url));
        }
        assert!(ok(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1"
        ));
        assert!(!ok(
            "http://github.com/yt-dlp/yt-dlp/releases/download/x/yt-dlp.exe"
        ));
        assert!(!ok("https://github.com.example.net/"));
        assert!(!ok("https://githubusercontent.com.example.net/"));
        assert!(!ok("https://example.net/yt-dlp.exe"));
    }

    #[test]
    fn versions_are_read_from_either_tool() {
        assert_eq!(
            version_line("2026.10.26\r\n").as_deref(),
            Some("2026.10.26")
        );
        assert_eq!(
            version_line(
                "deno 2.9.8 (stable, release, x86_64-pc-windows-msvc)\nv8 14.1\ntypescript 5.9\n"
            )
            .as_deref(),
            Some("2.9.8")
        );
        assert_eq!(version_line("\n  \n"), None);
    }

    #[test]
    fn the_pins_are_sha256_hex() {
        for pins in [&X64, &ARM64] {
            for h in [pins.ytdlp_sha256, pins.deno_sha256] {
                assert_eq!(h.len(), 64);
                assert!(
                    h.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                );
            }
            assert!(!pins.ytdlp_url.contains("/latest/") && !pins.deno_url.contains("/latest/"));
            assert!(pins.deno_url.ends_with(&format!("/{}", pins.deno_zip)));
        }
    }

    /// Each architecture downloads its own build, and both pin the same releases, so the
    /// weekly pin check (sidecar-pins.yml) reads one version for each helper.
    #[test]
    fn the_architectures_pin_the_same_releases_of_their_own_builds() {
        let release = |url: &'static str| url.rsplit_once('/').map(|(dir, _)| dir);
        assert_eq!(release(X64.ytdlp_url), release(ARM64.ytdlp_url));
        assert_eq!(release(X64.deno_url), release(ARM64.deno_url));
        assert!(X64.deno_zip.contains("x86_64") && ARM64.deno_zip.contains("aarch64"));
        assert!(ARM64.ytdlp_url.ends_with("/yt-dlp_arm64.exe"));
        assert!(X64.ytdlp_url.ends_with("/yt-dlp.exe"));
        assert_ne!(X64.ytdlp_sha256, ARM64.ytdlp_sha256);
        assert_ne!(X64.deno_sha256, ARM64.deno_sha256);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn an_arm64_build_downloads_the_arm64_helpers() {
        assert_eq!(Downloads::pinned().deno.url, ARM64.deno_url);
        assert_eq!(Downloads::pinned().ytdlp.url, ARM64.ytdlp_url);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn an_x64_build_downloads_the_x64_helpers() {
        assert_eq!(Downloads::pinned().deno.url, X64.deno_url);
        assert_eq!(Downloads::pinned().ytdlp.url, X64.ytdlp_url);
    }

    /// Downloads both pinned releases from GitHub (~60 MB) into a scratch dir.
    #[tokio::test]
    #[ignore = "live: downloads yt-dlp and deno from GitHub"]
    async fn live_the_pinned_releases_install() {
        let base =
            std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME to a D: dir for tests");
        let dir = PathBuf::from(base)
            .join("test-streams")
            .join(format!("ensure-{}", std::process::id()));
        let sidecars = Sidecars { dir: dir.clone() };
        sidecars.ensure(|_, _, _| {}).await.unwrap();
        assert!(sidecars.ytdlp().is_file() && sidecars.deno().is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The update stamp of 2026-11-03 stayed that old through the daily updates after it
    /// (writing an empty file over an empty one leaves its time), so every start ran
    /// `yt-dlp -U` (~10 s, beside the first resolves).
    #[tokio::test]
    async fn an_old_update_stamp_is_made_recent() {
        let base =
            std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME to a D: dir for tests");
        let dir = PathBuf::from(base)
            .join("test-streams")
            .join(format!("stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stamp = dir.join(UPDATE_STAMP);
        std::fs::write(&stamp, b"").unwrap();
        let day_old = SystemTime::now() - Duration::from_secs(25 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&stamp)
            .unwrap()
            .set_modified(day_old)
            .unwrap();
        assert!(!is_recent(&stamp));
        touch(&stamp).await;
        assert!(is_recent(&stamp));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn sha256_mismatch_rejected() {
        let base =
            std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME to a D: dir for tests");
        let dir = PathBuf::from(base)
            .join("test-streams")
            .join(format!("sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join("yt-dlp.exe.part");
        std::fs::write(&part, b"corrupted download").unwrap();
        // sha256 of "hello", not of the file.
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let err = verify_or_delete(&part, expected).await.unwrap_err();
        assert!(
            matches!(&err, ResolveError::Sidecar(m) if m.contains("sha256")),
            "got {err:?}"
        );
        assert!(!part.exists(), "the corrupted download was not deleted");
        // The matching hash passes and keeps the file.
        std::fs::write(&part, b"hello").unwrap();
        verify_or_delete(&part, &expected.to_uppercase())
            .await
            .unwrap();
        assert!(part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
