use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::failure::HelperFault;
use featherplay_core::sync::LockExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{ResolveError, SidecarError};

// The releases a first run installs, pinned with their sha256. A checksum file fetched
// from the same "latest" release would only catch a broken download, not a replaced
// release. Each hash is the one in that release's own checksum file (yt-dlp's
// `SHA2-256SUMS`, deno's `.sha256sum`), and matched the downloaded binaries on 2026-11-03.
// yt-dlp then keeps itself current with `yt-dlp -U`, which fetches the build of its own
// variant (`yt-dlp_arm64.exe` for the ARM64 one), whatever the file is called here.
// Both architectures pin the same versions; `.github/workflows/sidecar-pins.yml` checks that,
// and that the second sources below hold the same releases.
//
// Each file comes from GitHub first, then from a second source when GitHub can't be reached
// or sent something else: yt-dlp from featherplay's site (the same file, put there by the
// site's build, `site/scripts/helpers.mjs`, which reads the pins here), Deno from Deno's own
// download server, whose zips aren't GitHub's bytes (measured 2026-11-09: same sizes, and a
// `deno.exe` inside that differs from GitHub's too), so they have their own pins, measured
// from dl.deno.land itself. Deno publishes no checksum there to check them against: moving
// the Deno pin means downloading both zips once and pinning what came.

/// The helpers one architecture's build downloads.
struct Pins {
    ytdlp_url: &'static str,
    ytdlp_sha256: &'static str,
    /// The same file on featherplay's site.
    ytdlp_mirror: &'static str,
    /// The zip's file name; its download is kept as `{DENO_ZIP}.part`.
    deno_zip: &'static str,
    deno_url: &'static str,
    deno_sha256: &'static str,
    /// The release's zip on dl.deno.land, and its own sha256.
    deno_cdn_url: &'static str,
    deno_cdn_sha256: &'static str,
}

#[cfg(any(test, not(target_arch = "aarch64")))]
const X64: Pins = Pins {
    ytdlp_url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe",
    ytdlp_sha256: "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d",
    ytdlp_mirror: "https://featherplay.candygoogy.dev/helpers/yt-dlp/2026.10.26/yt-dlp.exe",
    deno_zip: "deno-x86_64-pc-windows-msvc.zip",
    deno_url: "https://github.com/denoland/deno/releases/download/v2.9.8/deno-x86_64-pc-windows-msvc.zip",
    deno_sha256: "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf",
    deno_cdn_url: "https://dl.deno.land/release/v2.9.8/deno-x86_64-pc-windows-msvc.zip",
    deno_cdn_sha256: "b0f654158ad3edaee34cf6aa8d39695f242d3b122753f6fb965261b9c7986f24",
};

#[cfg(any(test, target_arch = "aarch64"))]
const ARM64: Pins = Pins {
    ytdlp_url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp_arm64.exe",
    ytdlp_sha256: "0066dbac3c124028e2dcae2a197fccee33cd91f456313d24f7af89fd54590e98",
    ytdlp_mirror: "https://featherplay.candygoogy.dev/helpers/yt-dlp/2026.10.26/yt-dlp_arm64.exe",
    deno_zip: "deno-aarch64-pc-windows-msvc.zip",
    deno_url: "https://github.com/denoland/deno/releases/download/v2.9.8/deno-aarch64-pc-windows-msvc.zip",
    deno_sha256: "b413b92b735b50c198e8fe3b85af1aa50e29d5f72e229aab701d132286e14cef",
    deno_cdn_url: "https://dl.deno.land/release/v2.9.8/deno-aarch64-pc-windows-msvc.zip",
    deno_cdn_sha256: "ef6c1d9d946dd6d52774ec1adbd55abdaab9585ba9d7be269cfe41c022504b5b",
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
/// The second sources' hosts. Neither redirected on 2026-11-09.
const SECOND_HOSTS: [&str; 2] = ["featherplay.candygoogy.dev", "dl.deno.land"];

/// Touched after every successful `yt-dlp -U`, and when yt-dlp is downloaded: the pinned
/// release is new enough for its first day, then `update_if_stale` keeps it current.
const UPDATE_STAMP: &str = "yt-dlp.updated";
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(120);
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that can't connect, or stops sending, for this long fails.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A download stops here: the helpers are 20 to 45 MB, so more is no helper, and the disk
/// stays out of a bad server's reach.
const MAX_DOWNLOAD_BYTES: u64 = 200_000_000;

/// Antivirus often holds a new `.exe` for a few seconds after it's written, and the rename,
/// the unzip or the hashing then fails with "access denied" (os error 5) or "in use" (32).
/// Such a step waits these and tries again: five tries over about six seconds.
const HELD_WAITS_LIVE: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(1500),
    Duration::from_millis(3000),
];
/// A download that couldn't connect, timed out or met a server error (5xx) is tried twice
/// more, after these. A 404 or a file that doesn't match its pin isn't.
const DOWNLOAD_WAITS_LIVE: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];
#[cfg(not(test))]
const HELD_WAITS: [Duration; 4] = HELD_WAITS_LIVE;
#[cfg(not(test))]
const DOWNLOAD_WAITS: [Duration; 2] = DOWNLOAD_WAITS_LIVE;
// Tests don't wait.
#[cfg(test)]
const HELD_WAITS: [Duration; 4] = [Duration::from_millis(1); 4];
#[cfg(test)]
const DOWNLOAD_WAITS: [Duration; 2] = [Duration::from_millis(1); 2];

/// One place a file can be downloaded from, and the sha256 it must have there.
struct Source {
    url: String,
    sha256: String,
}

impl Source {
    fn new(url: &str, sha256: &str) -> Source {
        Source {
            url: url.into(),
            sha256: sha256.into(),
        }
    }
}

/// One file a first run downloads: its sources, in the order they're tried.
struct Asset {
    sources: Vec<Source>,
}

/// Where the sidecars are downloaded from: the pinned GitHub releases, then the second
/// sources, over HTTPS, with redirects only to those hosts. Tests use local servers instead.
pub struct Downloads {
    ytdlp: Asset,
    /// A zip with `deno.exe` in it.
    deno: Asset,
    /// The test servers on 127.0.0.1, over plain HTTP.
    #[cfg(test)]
    local: bool,
}

impl Downloads {
    /// The releases pinned above.
    pub fn pinned() -> Downloads {
        Downloads {
            ytdlp: Asset {
                sources: vec![
                    Source::new(PINS.ytdlp_url, PINS.ytdlp_sha256),
                    Source::new(PINS.ytdlp_mirror, PINS.ytdlp_sha256),
                ],
            },
            deno: Asset {
                sources: vec![
                    Source::new(PINS.deno_url, PINS.deno_sha256),
                    Source::new(PINS.deno_cdn_url, PINS.deno_cdn_sha256),
                ],
            },
            #[cfg(test)]
            local: false,
        }
    }

    /// `{base}/yt-dlp.exe` and `{base}/deno.zip` from test servers on 127.0.0.1, tried in
    /// the order of `bases`.
    #[cfg(test)]
    pub(crate) fn local(bases: &[&str], ytdlp_sha256: &str, deno_sha256: &str) -> Downloads {
        let from = |file: &str, sha256: &str| Asset {
            sources: bases
                .iter()
                .map(|base| Source::new(&format!("{base}/{file}"), sha256))
                .collect(),
        };
        Downloads {
            ytdlp: from("yt-dlp.exe", ytdlp_sha256),
            deno: from("deno.zip", deno_sha256),
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

    /// HTTPS only, redirects only to the download hosts, and timeouts so a dead connection
    /// fails the download instead of holding the resolves waiting on it. The client goes
    /// with the download, and its timers with it.
    fn client(&self) -> Result<reqwest::Client, ResolveError> {
        let builder = featherplay_proxy::client()
            .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT);
        #[cfg(test)]
        if self.local {
            return builder
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| sidecar_err(HelperFault::Unreachable, "http client", e));
        }
        builder
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && download_host(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect away from the download hosts refused")
                }
            }))
            .build()
            .map_err(|e| sidecar_err(HelperFault::Unreachable, "http client", e))
    }
}

/// Downloads the missing sidecars one download at a time: a caller that comes while one
/// runs waits for it and shares its outcome instead of starting another. Within a download,
/// each file is tried a few times (`DOWNLOAD_WAITS`, `HELD_WAITS`); after a failed one, the
/// next caller that comes tries again, so there's no retry loop.
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
            return Err(self.failed.lock_or_recover().clone().unwrap_or_else(|| {
                sidecar_err(
                    HelperFault::Unreachable,
                    "downloading yt-dlp and deno",
                    "failed",
                )
            }));
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
            Err(ResolveError::Sidecar(e)) => log::warn!(
                "downloading yt-dlp and deno failed ({}): {}",
                e.fault.name(),
                e.message
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
            .map_err(|e| io_err(HelperFault::NoSpace, "creating the sidecar dir", e))?;
        let http = from.client()?;

        if !self.ytdlp().is_file() {
            let part = self.dir.join("yt-dlp.exe.part");
            fetch(&http, from, &from.ytdlp, &part, "yt-dlp", &progress).await?;
            put_in_place(&part, &self.ytdlp()).await?;
            // The pinned release counts as today's update: no `yt-dlp -U` right after it.
            touch(&self.dir.join(UPDATE_STAMP)).await;
        }

        if !self.deno().is_file() {
            let zip = self.dir.join(format!("{DENO_ZIP}.part"));
            fetch(&http, from, &from.deno, &zip, "deno", &progress).await?;
            let exe_part = self.dir.join("deno.exe.part");
            let extracted = patiently(|| {
                let (z, e) = (zip.clone(), exe_part.clone());
                async move {
                    tokio::task::spawn_blocking(move || extract_deno(&z, &e))
                        .await
                        .map_err(std::io::Error::other)?
                }
            })
            .await
            .map_err(|e| io_err(HelperFault::Blocked, "extracting deno.exe", e));
            let _ = tokio::fs::remove_file(&zip).await;
            if let Err(e) = extracted {
                let _ = tokio::fs::remove_file(&exe_part).await;
                return Err(e);
            }
            put_in_place(&exe_part, &self.deno()).await?;
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
        // The update comes from GitHub.
        if let Some(proxy) = featherplay_proxy::for_url("https://github.com/")
            && let Some(p) = proxy.argument()
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

    /// A sidecar command (`hidden_command`), with deno's cache kept under `{cache}` instead
    /// of %LOCALAPPDATA%.
    pub(crate) fn command(&self, program: &Path) -> tokio::process::Command {
        let mut cmd = hidden_command(program);
        cmd.env("DENO_DIR", self.home().join("deno-dir"));
        cmd
    }
}

/// A command with no console window and no stdin, its output piped, killed if its future
/// is dropped.
pub(crate) fn hidden_command(program: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    featherplay_platform::process::no_console_window(cmd.as_std_mut());
    cmd
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

fn failure(fault: HelperFault, what: &str, e: impl std::fmt::Display) -> SidecarError {
    SidecarError {
        fault,
        message: format!("{what}: {e}"),
        host: None,
        status: None,
        os_error: None,
    }
}

fn sidecar_err(fault: HelperFault, what: &str, e: impl std::fmt::Display) -> ResolveError {
    ResolveError::Sidecar(failure(fault, what, e))
}

/// A file step that failed: `fault` unless the error says better. A full disk is `NoSpace`,
/// and a zip that can't be read (`InvalidData`) `Damaged`.
fn io_err(fault: HelperFault, what: &str, e: std::io::Error) -> ResolveError {
    // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL.
    let full = matches!(e.raw_os_error(), Some(39 | 112))
        || e.kind() == std::io::ErrorKind::StorageFull
        || e.kind() == std::io::ErrorKind::QuotaExceeded;
    let fault = if full {
        HelperFault::NoSpace
    } else if e.kind() == std::io::ErrorKind::InvalidData {
        HelperFault::Damaged
    } else {
        fault
    };
    ResolveError::Sidecar(SidecarError {
        os_error: e.raw_os_error(),
        ..failure(fault, what, e)
    })
}

/// A failed request or body read from `url`: `Unreachable`, with its host and HTTP status.
fn http_err(url: &str, e: reqwest::Error) -> ResolveError {
    let host = Some(host_of(url)).filter(|h| !h.is_empty());
    ResolveError::Sidecar(SidecarError {
        host,
        status: e.status().map(|s| s.as_u16()),
        ..failure(HelperFault::Unreachable, url, e)
    })
}

/// HTTPS on github.com, GitHub's release download hosts or a second source's host. Nothing
/// secret is sent; the pinned sha256 is what vouches for the file.
fn download_host(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|h| {
            h == "github.com" || h.ends_with(ASSET_DOMAIN) || SECOND_HOSTS.contains(&h)
        })
}

/// A download that failed, and whether it's worth another try.
struct Failed {
    error: ResolveError,
    /// It couldn't connect, timed out, was cut off mid-file or met a server error (5xx).
    transient: bool,
}

/// A failed request or response body: transient when it couldn't connect, timed out, the
/// connection dropped mid-file, or the server answered 5xx. A dropped connection comes as a
/// body error, or as a decode error since the body goes through the gzip decoder.
fn http_failed(url: &str, e: reqwest::Error) -> Failed {
    let transient = e.is_connect()
        || e.is_timeout()
        || e.is_body()
        || e.is_decode()
        || e.status().is_some_and(|s| s.is_server_error());
    Failed {
        error: http_err(url, e),
        transient,
    }
}

/// A failed write: not one another try would mend.
fn write_failed(what: &str, e: std::io::Error) -> Failed {
    Failed {
        error: io_err(HelperFault::NoSpace, what, e),
        transient: false,
    }
}

/// Downloads `asset` to `dest` and checks it against the pin of the source it came from:
/// from each source in turn while the last couldn't be reached or sent a damaged file. A
/// failure on our side (a full disk, a held file) stops there. When every source fails, the
/// first one's error is the one returned: GitHub's, what the user's network did to it.
async fn fetch(
    http: &reqwest::Client,
    from: &Downloads,
    asset: &Asset,
    dest: &Path,
    name: &str,
    progress: &impl Fn(&str, u64, u64),
) -> Result<(), ResolveError> {
    let mut first = None;
    for (i, source) in asset.sources.iter().enumerate() {
        if i > 0 {
            log::info!("downloading {name} from {} instead", host_of(&source.url));
        }
        let fetched = async {
            download_patiently(http, from, &source.url, dest, name, progress).await?;
            verify_or_delete(dest, &source.sha256).await
        }
        .await;
        match fetched {
            Ok(()) => return Ok(()),
            Err(ResolveError::Sidecar(e))
                if matches!(e.fault, HelperFault::Unreachable | HelperFault::Damaged) =>
            {
                log::warn!(
                    "downloading {name} from {} failed ({}): {}",
                    host_of(&source.url),
                    e.fault.name(),
                    e.message
                );
                first.get_or_insert(ResolveError::Sidecar(e));
            }
            Err(e) => return Err(e),
        }
    }
    Err(first.unwrap_or_else(|| sidecar_err(HelperFault::Unreachable, name, "no source")))
}

/// `url`'s host, for the log.
fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// `download`, tried again after `DOWNLOAD_WAITS` while it fails transiently.
async fn download_patiently(
    http: &reqwest::Client,
    from: &Downloads,
    url: &str,
    dest: &Path,
    name: &str,
    progress: &impl Fn(&str, u64, u64),
) -> Result<(), ResolveError> {
    let mut waits = DOWNLOAD_WAITS.iter();
    loop {
        let failed = match download(http, from, url, dest, name, progress).await {
            Ok(()) => return Ok(()),
            Err(failed) => failed,
        };
        match waits.next() {
            Some(wait) if failed.transient => {
                log::info!("downloading {name} failed ({}): trying again", failed.error);
                tokio::time::sleep(*wait).await;
            }
            _ => return Err(failed.error),
        }
    }
}

async fn download(
    http: &reqwest::Client,
    from: &Downloads,
    url: &str,
    dest: &Path,
    name: &str,
    progress: &impl Fn(&str, u64, u64),
) -> Result<(), Failed> {
    if !reqwest::Url::parse(url).is_ok_and(|u| from.allows(&u)) {
        return Err(Failed {
            error: sidecar_err(HelperFault::Unreachable, url, "not an allowed download"),
            transient: false,
        });
    }
    let result = async {
        let mut resp = http
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| http_failed(url, e))?;
        let total = resp.content_length().unwrap_or(0);
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| write_failed("creating the download file", e))?;
        let mut done = 0u64;
        progress(name, 0, total);
        while let Some(chunk) = resp.chunk().await.map_err(|e| http_failed(url, e))? {
            file.write_all(&chunk)
                .await
                .map_err(|e| write_failed("writing the download", e))?;
            done += chunk.len() as u64;
            if done > MAX_DOWNLOAD_BYTES {
                return Err(Failed {
                    error: sidecar_err(HelperFault::Damaged, url, "far bigger than the helper"),
                    transient: false,
                });
            }
            progress(name, done, total);
        }
        file.flush()
            .await
            .map_err(|e| write_failed("writing the download", e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// Runs a file step on a new file, again after each of `HELD_WAITS` while antivirus holds
/// the file (os error 5 or 32). Any other error, or the last one, is the step's.
async fn patiently<T, F, Fut>(mut step: F) -> std::io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let mut waits = HELD_WAITS.iter();
    loop {
        match step().await {
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) => match waits.next() {
                Some(wait) => tokio::time::sleep(*wait).await,
                None => return Err(e),
            },
            done => return done,
        }
    }
}

/// Renames the checked `part` to `exe`, waiting out antivirus. An exe gone right after
/// (quarantined) fails too.
async fn put_in_place(part: &Path, exe: &Path) -> Result<(), ResolveError> {
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    patiently(|| tokio::fs::rename(part, exe))
        .await
        .map_err(|e| io_err(HelperFault::Blocked, &format!("installing {name}"), e))?;
    if !exe.is_file() {
        return Err(sidecar_err(
            HelperFault::Blocked,
            &format!("installing {name}"),
            "it was removed right after it was saved",
        ));
    }
    Ok(())
}

/// Checks `path`'s sha256 against `expected_hex` (any case). On a mismatch the file is deleted.
async fn verify_or_delete(path: &Path, expected_hex: &str) -> Result<(), ResolveError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let result = match patiently(|| sha256_file(path)).await {
        Ok(actual) if actual.eq_ignore_ascii_case(expected_hex.trim()) => return Ok(()),
        Ok(actual) => sidecar_err(
            HelperFault::Damaged,
            &format!("sha256 mismatch for {name}"),
            format!("expected {expected_hex}, got {actual}"),
        ),
        Err(e) => io_err(
            HelperFault::Blocked,
            &format!("reading {name} for its sha256"),
            e,
        ),
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

/// Writes the zip's `deno.exe` to `exe_dest`. A zip that can't be read is `InvalidData`; the
/// file errors are the system's, so `patiently` can tell a held file.
fn extract_deno(zip_path: &Path, exe_dest: &Path) -> std::io::Result<()> {
    let zip_err = |e: zip::result::ZipError| match e {
        zip::result::ZipError::Io(e) => e,
        e => std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the deno zip: {e}"),
        ),
    };
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;
    let mut entry = archive.by_name("deno.exe").map_err(zip_err)?;
    let mut out = std::fs::File::create(exe_dest)?;
    std::io::copy(&mut entry, &mut out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_stay_on_their_hosts_over_https() {
        let ok = |u: &str| download_host(&reqwest::Url::parse(u).unwrap());
        for pins in [&X64, &ARM64] {
            assert!(ok(pins.ytdlp_url) && ok(pins.deno_url));
            assert!(ok(pins.ytdlp_mirror) && ok(pins.deno_cdn_url));
        }
        assert!(!ok(
            "http://featherplay.candygoogy.dev/helpers/yt-dlp/x/yt-dlp.exe"
        ));
        assert!(!ok("http://dl.deno.land/release/v2.9.8/deno.zip"));
        assert!(!ok("https://dl.deno.land.example.net/"));
        assert!(!ok("https://evil.featherplay.candygoogy.dev/"));
        assert!(!ok("https://candygoogy.dev/"));
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
            for h in [pins.ytdlp_sha256, pins.deno_sha256, pins.deno_cdn_sha256] {
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
        assert_ne!(X64.deno_cdn_sha256, ARM64.deno_cdn_sha256);
    }

    /// The second sources hold the same releases, under the same file names: the site's
    /// `helpers/yt-dlp/<version>/<file>`, and Deno's `release/v<version>/<zip>`.
    #[test]
    fn the_second_sources_hold_the_pinned_releases() {
        for pins in [&X64, &ARM64] {
            let (_, ytdlp) = pins.ytdlp_url.split_once("/releases/download/").unwrap();
            assert_eq!(
                pins.ytdlp_mirror,
                format!("https://featherplay.candygoogy.dev/helpers/yt-dlp/{ytdlp}")
            );
            let (_, deno) = pins.deno_url.split_once("/releases/download/").unwrap();
            assert_eq!(
                pins.deno_cdn_url,
                format!("https://dl.deno.land/release/{deno}")
            );
            assert_ne!(pins.deno_cdn_sha256, pins.deno_sha256, "not GitHub's bytes");
        }
    }

    /// GitHub first, then the second source, each with its own pin.
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn an_arm64_build_downloads_the_arm64_helpers() {
        let urls = |a: &Asset| a.sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
        let d = Downloads::pinned();
        assert_eq!(urls(&d.ytdlp), [ARM64.ytdlp_url, ARM64.ytdlp_mirror]);
        assert_eq!(urls(&d.deno), [ARM64.deno_url, ARM64.deno_cdn_url]);
        assert_eq!(d.deno.sources[1].sha256, ARM64.deno_cdn_sha256);
    }

    /// GitHub first, then the second source, each with its own pin.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn an_x64_build_downloads_the_x64_helpers() {
        let urls = |a: &Asset| a.sources.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
        let d = Downloads::pinned();
        assert_eq!(urls(&d.ytdlp), [X64.ytdlp_url, X64.ytdlp_mirror]);
        assert_eq!(urls(&d.deno), [X64.deno_url, X64.deno_cdn_url]);
        assert_eq!(d.deno.sources[1].sha256, X64.deno_cdn_sha256);
        assert_eq!(d.ytdlp.sources[1].sha256, X64.ytdlp_sha256, "the same file");
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

    /// What a file step does while antivirus scans the new file: "in use" (os error 32)
    /// `held` times, then it goes through. Counts its calls.
    fn held_step(
        held: usize,
        code: i32,
    ) -> (
        impl FnMut() -> std::future::Ready<std::io::Result<u8>>,
        std::sync::Arc<AtomicU64>,
    ) {
        let calls = std::sync::Arc::new(AtomicU64::new(0));
        let counted = calls.clone();
        let step = move || {
            let n = counted.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if (n as usize) < held {
                Err(std::io::Error::from_raw_os_error(code))
            } else {
                Ok(7)
            })
        };
        (step, calls)
    }

    #[tokio::test]
    async fn a_file_step_waits_while_antivirus_holds_the_file() {
        let (step, calls) = held_step(2, 32);
        assert_eq!(patiently(step).await.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_file_held_too_long_fails_after_five_tries() {
        let (step, calls) = held_step(usize::MAX, 5);
        let err = patiently(step).await.unwrap_err();
        assert_eq!(err.raw_os_error(), Some(5));
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn other_file_errors_fail_at_once() {
        // ERROR_DISK_FULL
        let (step, calls) = held_step(usize::MAX, 112);
        assert_eq!(patiently(step).await.unwrap_err().raw_os_error(), Some(112));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A connection that drops mid-file is tried again, like one that couldn't connect.
    #[tokio::test]
    async fn a_download_cut_off_mid_file_is_tried_again() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().enumerate() {
                let mut s = stream.unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && s.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                // The first answer promises 100 bytes and stops after 7.
                let answer: &[u8] = if n == 0 {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial"
                } else {
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello"
                };
                let _ = s.write_all(answer);
            }
        });
        let dir = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("FEATHERPLAY_HOME"))
            .join("test-streams")
            .join(format!("cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("yt-dlp.exe.part");
        let from = Downloads::local(&[&base], "", "");
        let http = from.client().unwrap();

        let url = format!("{base}/yt-dlp.exe");
        let got = download_patiently(&http, &from, &url, &dest, "yt-dlp", &|_, _, _| {}).await;

        assert_eq!(got, Ok(()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What a failed file step is, from the system's error.
    #[test]
    fn a_file_error_says_what_kept_the_helpers_out() {
        let fault = |os: i32| {
            let e = io_err(
                HelperFault::Blocked,
                "installing yt-dlp.exe",
                std::io::Error::from_raw_os_error(os),
            );
            match e {
                ResolveError::Sidecar(e) => (e.fault, e.os_error),
                e => panic!("{e:?}"),
            }
        };
        // Still held after the retries.
        assert_eq!(fault(32), (HelperFault::Blocked, Some(32)));
        assert_eq!(fault(5), (HelperFault::Blocked, Some(5)));
        // ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL.
        assert_eq!(fault(112), (HelperFault::NoSpace, Some(112)));
        assert_eq!(fault(39), (HelperFault::NoSpace, Some(39)));
        let damaged = io_err(
            HelperFault::Blocked,
            "extracting deno.exe",
            std::io::Error::new(std::io::ErrorKind::InvalidData, "the deno zip: bad"),
        );
        assert!(matches!(damaged, ResolveError::Sidecar(e) if e.fault == HelperFault::Damaged));
    }

    /// Copy diagnostics' line: what and where, never a path.
    #[test]
    fn a_helper_error_is_summed_up_without_paths() {
        let e = SidecarError {
            fault: HelperFault::Unreachable,
            message: r"C:\Users\ada\x: boom".into(),
            host: Some("github.com".into()),
            status: Some(503),
            os_error: None,
        };
        assert_eq!(e.summary(), "unreachable, github.com, HTTP 503");
        let e = SidecarError {
            fault: HelperFault::Blocked,
            host: None,
            status: None,
            os_error: Some(32),
            ..e
        };
        assert_eq!(e.summary(), "blocked, os error 32");
    }

    /// The real waits: about six seconds for a held file, as long as antivirus usually
    /// holds one, and two more tries for a download.
    #[test]
    fn the_waits_are_few_and_short() {
        let total: Duration = HELD_WAITS_LIVE.iter().sum();
        assert!(
            total >= Duration::from_secs(5) && total <= Duration::from_secs(7),
            "{total:?}"
        );
        assert_eq!(HELD_WAITS_LIVE.len(), 4, "five tries");
        assert_eq!(DOWNLOAD_WAITS_LIVE.len(), 2, "three tries");
        assert!(DOWNLOAD_WAITS_LIVE.iter().sum::<Duration>() <= Duration::from_secs(10));
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
            matches!(&err, ResolveError::Sidecar(e)
                if e.message.contains("sha256") && e.fault == HelperFault::Damaged),
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
