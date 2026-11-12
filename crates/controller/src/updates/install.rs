//! Automatic updates for the installed copy: the newer release's setup is downloaded in the
//! background into `{cache}/updates/`, checked against the release's `SHA256SUMS.txt`, and
//! offered as "Restart to update". Clicking it starts the setup silently and quits; the
//! setup closes what's left of featherplay, installs over it and starts the new version
//! (`packaging/windows/featherplay.iss`: `Relaunch`).
//!
//! Only the per-user install does this. It is told apart from a portable exe by the
//! installer's own uninstall key: the running exe must be the `featherplay.exe` in the
//! folder that key's `InstallLocation` names. A portable copy, or a dev build, is only told
//! about a newer release, as before.
//!
//! On Linux the AppImage takes the setup's place: the release's
//! `featherplay-<arch>.AppImage` is downloaded and checked the same way, and "Restart to
//! update" puts it in place of the AppImage this copy runs from (a new file beside it,
//! flushed, renamed over it) and starts it once featherplay has quit. Only a copy that runs
//! from an AppImage it may replace does this; one from the .tar.gz, or a distribution's
//! package, is only told about a newer release.
//!
//! Downloads come only from the project's release files on GitHub (`DOWNLOADS`), over
//! HTTPS, following redirects only to GitHub's asset hosts. `SHA256SUMS.txt` must carry a
//! minisign signature (`SHA256SUMS.txt.minisig`) by one of the two keys built into
//! featherplay (`Keys::release`), made for that version (`verify_sums`). The maintainer
//! signs it on their own PC, so someone who can only upload release files can't make
//! copies install anything. A release with no signature, a bad one, or one for another
//! version is never downloaded, and never installed unsigned instead. A setup whose sha256
//! doesn't match is deleted and the release is only announced.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::{Asset, Release};

/// Where every update download starts: the project's own release files.
pub const DOWNLOADS: &str = "https://github.com/candygoogy/featherplay/releases/download/";
/// The checksum file each release carries (`.github/workflows/release.yml`): `sha256sum`
/// lines for the exes, the setups, the AppImages, the Linux archives and THIRD_PARTY.md.
pub const SUMS: &str = "SHA256SUMS.txt";
/// `SUMS`'s minisign signature, made and uploaded by hand (`packaging/sign-release.sh`).
pub const SUMS_SIG: &str = "SHA256SUMS.txt.minisig";
/// What the signature's trusted comment says, the version after it: `featherplay v0.2.0`.
/// The comment is signed too, so an old release's files can't pass for a newer one's.
const SIGNED_FOR: &str = "featherplay v";
/// The public keys that sign releases: the main one, and the backup kept apart in case the
/// main one is lost. Either one's signature is enough. They stay in the app's folder
/// (`apps/featherplay/keys/`), where the signing scripts and RELEASING.md look for them.
const RELEASE_KEYS: [&str; 2] = [
    include_str!("../../../../apps/featherplay/keys/update.pub"),
    include_str!("../../../../apps/featherplay/keys/update-backup.pub"),
];
/// How the setup is started: no wizard, no message boxes, no reboot, and `/RELAUNCH`, which
/// the setup script reads to close featherplay first and start it again afterwards.
pub const SETUP_ARGS: [&str; 4] = [
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/RELAUNCH",
];
/// The folder in the cache that holds the downloaded setup and the setup's log.
const DIR: &str = "updates";
const SETUP_PREFIX: &str = "featherplay-setup-";
/// Far more than any setup (11 MB for 0.1.0) or checksum file; a larger reply is refused.
const MAX_SETUP: u64 = 256 * 1024 * 1024;
const MAX_SUMS: u64 = 64 * 1024;
/// A minisign signature is about 300 bytes.
const MAX_SIG: u64 = 4 * 1024;
/// A download that can't connect, or stops sending, for this long fails.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// GitHub sends release downloads on to a host under this domain
/// (`release-assets.githubusercontent.com` in Nov 2026).
const ASSET_DOMAIN: &str = ".githubusercontent.com";

/// The AppImage's file name on `arch`, the same for every release: the file a user keeps is
/// replaced in place, under its own name.
const APPIMAGE_PREFIX: &str = "featherplay-";
const APPIMAGE_SUFFIX: &str = ".AppImage";

/// The setup's file name for this `version` on `arch` (`std::env::consts::ARCH`):
/// `featherplay-setup-0.2.0.exe` on x64, `featherplay-setup-0.2.0-arm64.exe` on ARM64. No
/// setup for other architectures, or for a version that isn't digits and dots.
pub fn setup_name(version: &str, arch: &str) -> Option<String> {
    let suffix = match arch {
        "x86_64" => "",
        "aarch64" => "-arm64",
        _ => return None,
    };
    plain_version(version).then(|| format!("{SETUP_PREFIX}{version}{suffix}.exe"))
}

/// The AppImage's file name on `arch`: `featherplay-x86_64.AppImage`,
/// `featherplay-aarch64.AppImage` (packaging/linux/build-linux.sh). None for other
/// architectures, or for a version that isn't digits and dots.
pub fn appimage_name(version: &str, arch: &str) -> Option<String> {
    let known = matches!(arch, "x86_64" | "aarch64");
    (known && plain_version(version)).then(|| format!("{APPIMAGE_PREFIX}{arch}{APPIMAGE_SUFFIX}"))
}

/// The file that updates this copy, for `version` on `os` and `arch`
/// (`std::env::consts::OS` and `ARCH`): Windows' setup, or Linux's AppImage.
pub fn update_name(os: &str, version: &str, arch: &str) -> Option<String> {
    match os {
        "windows" => setup_name(version, arch),
        "linux" => appimage_name(version, arch),
        _ => None,
    }
}

fn plain_version(version: &str) -> bool {
    !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && !version.starts_with('.')
        && !version.contains("..")
}

/// Whether `file` is named like a setup or an AppImage (`update_name`), and nothing else: no
/// path, no other characters.
fn update_file(file: &str) -> bool {
    let named = (file.starts_with(SETUP_PREFIX) && file.ends_with(".exe"))
        || (file.starts_with(APPIMAGE_PREFIX) && file.ends_with(APPIMAGE_SUFFIX));
    named
        && !file.contains("..")
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

/// The sha256 (lower-case hex) `SHA256SUMS.txt` gives for `file`. Lines are `sha256sum`'s:
/// `<hex>  <name>`, or `<hex> *<name>` in binary mode. None if the file isn't listed, or is
/// listed with two different hashes.
pub fn parse_sums(text: &str, file: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in text.lines() {
        let Some((hash, rest)) = line.trim_end().split_once(' ') else {
            continue;
        };
        let name = rest.strip_prefix([' ', '*']).unwrap_or(rest);
        if name != file || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let hash = hash.to_ascii_lowercase();
        match &found {
            Some(seen) if *seen != hash => return None,
            _ => found = Some(hash),
        }
    }
    found
}

/// Whether `exe` is the installed copy: `featherplay.exe` directly in `install_location`
/// (the uninstall key's `InstallLocation`, which Inno writes with a trailing backslash).
/// Compared without case, as Windows paths are.
pub fn is_installed(exe: &Path, install_location: &Path) -> bool {
    fn plain(p: &Path) -> String {
        let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        s.trim_end_matches('\\').to_string()
    }
    let named = exe
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("featherplay.exe"));
    let location = plain(install_location);
    named && !location.is_empty() && exe.parent().is_some_and(|dir| plain(dir) == location)
}

/// Whether this process is the installed copy (`is_installed`, read from the registry).
#[cfg(windows)]
pub fn installed() -> bool {
    let Some(location) = featherplay_platform::installer::install_location() else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // The same form for both (`\\?\`, long names), when the paths exist.
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    is_installed(&canonical(&exe), &canonical(&location))
}

/// Whether this process updates in place: it runs from an AppImage it may replace
/// (`installer::appimage`). A copy from the .tar.gz or a distribution's package doesn't.
#[cfg(target_os = "linux")]
pub fn installed() -> bool {
    featherplay_platform::installer::appimage().is_some()
}

/// Whether `exe` is the copy Scoop installed (`packaging/scoop/featherplay.json`): Scoop
/// keeps its apps in `…\scoop\apps\<name>\`, and updates them itself.
pub fn is_scoop(exe: &Path) -> bool {
    exe.to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
        .contains(r"\scoop\apps\featherplay\")
}

/// Whether this process is the copy Scoop installed (`is_scoop`).
pub fn scoop() -> bool {
    std::env::current_exe().is_ok_and(|exe| is_scoop(&exe))
}

/// The public keys a release's checksums must be signed with: `Keys::release` in the app,
/// test keys in tests.
pub struct Keys(Vec<minisign_verify::PublicKey>);

impl Keys {
    /// The two keys built into featherplay (`RELEASE_KEYS`).
    pub fn release() -> Keys {
        Keys::from_pub(&RELEASE_KEYS)
    }

    /// Keys from the text of minisign `.pub` files. One that doesn't read is left out, so
    /// it verifies nothing.
    pub fn from_pub(texts: &[&str]) -> Keys {
        let keys = texts
            .iter()
            .filter_map(|t| minisign_verify::PublicKey::decode(t).ok())
            .collect();
        Keys(keys)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Why a release's `SHA256SUMS.txt` isn't trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureError {
    /// The release has no `SHA256SUMS.txt.minisig`.
    Missing,
    /// The signature file isn't one.
    Unreadable,
    /// Signed with a key featherplay doesn't know.
    UnknownKey,
    /// Made with one of its keys, but not for this file (or damaged).
    Invalid,
    /// A valid signature, for another release: its trusted comment.
    WrongVersion(String),
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignatureError::Missing => write!(f, "the release has no {SUMS_SIG}"),
            SignatureError::Unreadable => write!(f, "{SUMS_SIG} isn't a minisign signature"),
            SignatureError::UnknownKey => {
                write!(f, "{SUMS} isn't signed with featherplay's keys")
            }
            SignatureError::Invalid => write!(f, "{SUMS}'s signature doesn't match it"),
            SignatureError::WrongVersion(comment) => {
                write!(f, "{SUMS} is signed for {comment:?}, not for this release")
            }
        }
    }
}

/// Whether `sums` (a release's `SHA256SUMS.txt`) is signed by `minisig` with one of `keys`,
/// for `version`: the signature's trusted comment must be exactly `featherplay v{version}`.
pub fn verify_sums(
    sums: &[u8],
    minisig: &str,
    version: &str,
    keys: &Keys,
) -> Result<(), SignatureError> {
    let sig =
        minisign_verify::Signature::decode(minisig).map_err(|_| SignatureError::Unreadable)?;
    // Made with one of the keys (its id matches), but not valid.
    let mut ours = false;
    for key in &keys.0 {
        // Prehashed signatures only: minisign 0.12 makes no others.
        match key.verify(sums, &sig, false) {
            Ok(()) => {
                // Read only now: until the signature checks out, the comment is anyone's.
                let comment = sig.trusted_comment();
                return match comment.strip_prefix(SIGNED_FOR) {
                    Some(v) if v == version => Ok(()),
                    _ => Err(SignatureError::WrongVersion(comment.to_string())),
                };
            }
            Err(minisign_verify::Error::UnexpectedKeyId) => {}
            Err(_) => ours = true,
        }
    }
    Err(if ours {
        SignatureError::Invalid
    } else {
        SignatureError::UnknownKey
    })
}

/// What to download for a release: its setup (or AppImage) for this OS and architecture,
/// its checksums, and their signature, if it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub version: String,
    pub setup: Asset,
    pub sums: Asset,
    pub sig: Option<Asset>,
}

/// The release's file for `os` and `arch` (`update_name`) and its `SHA256SUMS.txt`, if it
/// has both. A missing signature is still a plan: `fetch` refuses it, and says so.
pub fn plan(release: &Release, os: &str, arch: &str) -> Option<Plan> {
    let name = update_name(os, &release.version, arch)?;
    let find = |name: &str| release.assets.iter().find(|a| a.name == name).cloned();
    Some(Plan {
        version: release.version.clone(),
        setup: find(&name)?,
        sums: find(SUMS)?,
        sig: find(SUMS_SIG),
    })
}

/// A downloaded setup whose sha256 matched: the version it installs, its file in
/// `{cache}/updates/`, its sha256, and whether that sha256 came from signed checksums.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    pub version: String,
    pub file: String,
    pub sha256: String,
    /// False for a setup 0.1.2 or older downloaded, checking only the sha256: it's never
    /// installed (`Stamp::pending`).
    #[serde(default)]
    pub signed: bool,
}

impl Ready {
    /// The setup's path; none if the stamp names anything but a setup in that folder.
    pub fn path(&self, cache: &Path) -> Option<PathBuf> {
        update_file(&self.file).then(|| dir(cache).join(&self.file))
    }

    /// The setup is still there, with the sha256 it was checked with.
    pub fn verified(&self, cache: &Path) -> bool {
        self.path(cache)
            .and_then(|p| sha256_file(&p).ok())
            .is_some_and(|h| h == self.sha256)
    }
}

fn dir(cache: &Path) -> PathBuf {
    cache.join(DIR)
}

/// Where downloads may come from: the project's release files (with GitHub's redirects), or
/// in tests a server on 127.0.0.1.
pub struct Origin {
    #[cfg(test)]
    local: bool,
}

impl Origin {
    pub fn github() -> Origin {
        Origin {
            #[cfg(test)]
            local: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn local() -> Origin {
        Origin { local: true }
    }

    /// The client for GitHub's API (`releases/latest`, the issue search): HTTPS only, or
    /// plain HTTP to the test server.
    pub fn api_client(&self, timeout: Duration) -> Result<reqwest::Client, String> {
        let builder = featherplay_proxy::client()
            .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
            .timeout(timeout);
        #[cfg(test)]
        let builder = builder.https_only(!self.local);
        #[cfg(not(test))]
        let builder = builder.https_only(true);
        builder.build().map_err(|e| format!("http client: {e}"))
    }

    /// A URL a download may start from.
    fn allows(&self, url: &str) -> bool {
        #[cfg(test)]
        if self.local {
            return url.starts_with("http://127.0.0.1:");
        }
        // Compared as the client will send it: dot segments (`download/../../..`) are
        // resolved by the parser, so they can't lead out of the project's releases.
        url.is_ascii()
            && !url.contains(char::is_whitespace)
            && reqwest::Url::parse(url).is_ok_and(|u| u.as_str().starts_with(DOWNLOADS))
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        let builder = featherplay_proxy::client()
            .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT);
        #[cfg(test)]
        if self.local {
            return builder
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| format!("http client: {e}"));
        }
        builder
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 5 && redirect_target(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("redirect away from GitHub refused")
                }
            }))
            .build()
            .map_err(|e| format!("http client: {e}"))
    }
}

/// Where GitHub may send a release download: its asset hosts, or the project's releases.
fn redirect_target(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && (url.as_str().starts_with(DOWNLOADS)
            || url.host_str().is_some_and(|h| h.ends_with(ASSET_DOMAIN)))
}

/// Why a download gave no setup: `Refused` is final for that release (a checksum that
/// doesn't match, a file the checksums don't list, a URL off the project's releases, a
/// version that isn't newer); `Unverified` too (checksums not signed for it with
/// featherplay's keys); `Failed` may work later (the network, the disk).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Refused(String),
    Unverified(SignatureError),
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Refused(e) | FetchError::Failed(e) => f.write_str(e),
            FetchError::Unverified(e) => write!(f, "not verified: {e}"),
        }
    }
}

/// Downloads the release's checksums and their signature, checks the signature with `keys`
/// (`verify_sums`), then downloads the setup into `{cache}/updates/` and checks its sha256.
/// Nothing is written before the signature is checked; on a mismatch, or any failure, the
/// partial file is deleted. Only a release newer than the running one is downloaded.
pub async fn fetch(
    cache: &Path,
    plan: &Plan,
    origin: &Origin,
    keys: &Keys,
) -> Result<Ready, FetchError> {
    use FetchError::{Failed, Refused, Unverified};
    if !super::is_newer(&plan.version, super::CURRENT) {
        return Err(Refused(format!(
            "{} isn't newer than {}",
            plan.version,
            super::CURRENT
        )));
    }
    let sig = plan
        .sig
        .as_ref()
        .ok_or(Unverified(SignatureError::Missing))?;
    for url in [&plan.sums.url, &sig.url, &plan.setup.url] {
        if !origin.allows(url) {
            return Err(Refused(format!("not a featherplay release file: {url}")));
        }
    }
    let http = origin.client().map_err(Failed)?;
    let sums = small_text(&http, origin, &plan.sums.url, MAX_SUMS)
        .await
        .map_err(Failed)?;
    let minisig = small_text(&http, origin, &sig.url, MAX_SIG)
        .await
        .map_err(Failed)?;
    verify_sums(sums.as_bytes(), &minisig, &plan.version, keys).map_err(Unverified)?;
    let expected = parse_sums(&sums, &plan.setup.name)
        .ok_or_else(|| Refused(format!("{SUMS} has no sha256 for {}", plan.setup.name)))?;
    let ready = Ready {
        version: plan.version.clone(),
        file: plan.setup.name.clone(),
        sha256: expected,
        signed: true,
    };
    let dest = ready
        .path(cache)
        .ok_or_else(|| Refused(format!("not a setup's name: {}", plan.setup.name)))?;
    let part = dest.with_file_name(format!("{}.part", ready.file));
    tokio::fs::create_dir_all(dir(cache))
        .await
        .map_err(|e| Failed(format!("creating the updates folder: {}", e.kind())))?;
    let result = async {
        let actual = download(&http, origin, &plan.setup.url, &part)
            .await
            .map_err(Failed)?;
        if actual != ready.sha256 {
            return Err(Refused(format!(
                "sha256 mismatch for {}: expected {}, got {actual}",
                ready.file, ready.sha256
            )));
        }
        let _ = tokio::fs::remove_file(&dest).await;
        tokio::fs::rename(&part, &dest)
            .await
            .map_err(|e| Failed(format!("moving the setup into place: {}", e.kind())))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result.map(|()| ready)
}

async fn get(
    http: &reqwest::Client,
    origin: &Origin,
    url: &str,
    max: u64,
) -> Result<reqwest::Response, String> {
    if !origin.allows(url) {
        return Err(format!("not a featherplay release file: {url}"));
    }
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {} for {url}", status.as_u16()));
    }
    if resp.content_length().is_some_and(|n| n > max) {
        return Err(format!("{url} is larger than expected"));
    }
    Ok(resp)
}

/// A small text file (the checksums, their signature), at most `max` bytes.
async fn small_text(
    http: &reqwest::Client,
    origin: &Origin,
    url: &str,
    max: u64,
) -> Result<String, String> {
    let mut resp = get(http, origin, url, max).await?;
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("reading {url}: {e}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max {
            return Err(format!("{url} is larger than expected"));
        }
    }
    String::from_utf8(body).map_err(|_| format!("{url} isn't text"))
}

/// Streams `url` into `dest`, at most `MAX_SETUP` bytes; gives the sha256 of what arrived.
async fn download(
    http: &reqwest::Client,
    origin: &Origin,
    url: &str,
    dest: &Path,
) -> Result<String, String> {
    let mut resp = get(http, origin, url, MAX_SETUP).await?;
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("creating the download file: {}", e.kind()))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("reading {url}: {e}"))?
    {
        done += chunk.len() as u64;
        if done > MAX_SETUP {
            return Err(format!("{url} is larger than expected"));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("writing the download: {}", e.kind()))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("writing the download: {}", e.kind()))?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Deletes the setups or AppImages (and partial downloads) in `{cache}/updates/` other than
/// `keep`'s: an older one, or the one just installed. Only plain files named like one.
pub fn clean(cache: &Path, keep: Option<&Ready>) {
    let Ok(entries) = std::fs::read_dir(dir(cache)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        let ours = update_file(name.strip_suffix(".part").unwrap_or(&name));
        if !is_file || !ours || keep.is_some_and(|r| r.file == name) {
            continue;
        }
        if let Err(e) = std::fs::remove_file(entry.path()) {
            log!("removing an old update {name}: {}", e.kind());
        }
    }
}

/// The last `n` lines of the setup's log (`{cache}/updates/install.log`), for featherplay's
/// own log after an update didn't install. Inno writes it in the system's code page; the
/// lines that matter are ASCII.
pub fn install_log_tail(cache: &Path, n: usize) -> Vec<String> {
    let Ok(bytes) = std::fs::read(dir(cache).join("install.log")) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect()
}

/// The downloaded file's path, once it's checked again.
fn checked(cache: &Path, ready: &Ready) -> Result<PathBuf, String> {
    let path = ready
        .path(cache)
        .ok_or_else(|| format!("not a setup's name: {}", ready.file))?;
    if !ready.verified(cache) {
        return Err(format!(
            "{} is gone or no longer matches its sha256",
            ready.file
        ));
    }
    Ok(path)
}

/// Starts the downloaded setup (checked again first) with `SETUP_ARGS`, logging to
/// `{cache}/updates/install.log`. featherplay quits right after.
#[cfg(windows)]
pub fn launch(cache: &Path, ready: &Ready) -> Result<(), String> {
    use std::process::Command;

    let path = checked(cache, ready)?;
    let command = || {
        let mut c = Command::new(&path);
        c.args(SETUP_ARGS)
            .arg(format!("/LOG={}", dir(cache).join("install.log").display()));
        c
    };
    // Out of featherplay's job, so it outlives featherplay.
    featherplay_platform::installer::start_setup(command)
        .map_err(|e| format!("starting {}: {}", ready.file, e.kind()))
}

/// Replaces the AppImage this copy runs from with the downloaded one (`replace_appimage`),
/// then starts it once featherplay has quit, which it does right after.
#[cfg(target_os = "linux")]
pub fn launch(cache: &Path, ready: &Ready) -> Result<(), String> {
    use featherplay_platform::installer;

    let image = installer::appimage().ok_or("this copy doesn't run from an AppImage")?;
    replace_appimage(cache, ready, &image)?;
    installer::start_setup(|| installer::appimage_command(&image))
        .map_err(|e| format!("starting {}: {}", image.display(), e.kind()))
}

/// Puts the downloaded AppImage (checked again first) in place of `image`, all at once
/// (`installer::replace_file`): the copy's sha256 must be the one the signed checksums
/// gave, or `image` stays as it was.
#[cfg(target_os = "linux")]
pub fn replace_appimage(cache: &Path, ready: &Ready, image: &Path) -> Result<(), String> {
    let path = checked(cache, ready)?;
    featherplay_platform::installer::replace_file(image, |out| {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            io::Write::write_all(out, &buf[..n])?;
        }
        if hex(&hasher.finalize()) == ready.sha256 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the copy doesn't match its sha256",
            ))
        }
    })
    .map_err(|e| format!("replacing {}: {e}", image.display()))
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    pub(crate) const SETUP: &[u8] = b"not really a setup, but bytes to check";

    pub(crate) fn test_cache(name: &str) -> PathBuf {
        let base = std::env::var("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
        let cache = base
            .join("test-updates")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        std::fs::create_dir_all(&cache).unwrap();
        cache
    }

    /// Answers each GET with `route(path)`, one connection at a time.
    pub(crate) fn serve(route: impl Fn(&str) -> (u16, Vec<u8>) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                    }
                    let (status, body) = route(&path);
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    );
                    if stream.write_all(head.as_bytes()).is_err()
                        || stream.write_all(&body).is_err()
                    {
                        break;
                    }
                }
            }
        });
        base
    }

    /// 9.0.0's `SHA256SUMS.txt` (`SETUP`'s sha256 for its x64 and ARM64 setups), signed with the test
    /// main key for `featherplay v9.0.0` (`tests/fixtures/signing/`).
    pub(crate) const SIGNED_SUMS: &[u8] =
        include_bytes!("../../tests/fixtures/signing/SHA256SUMS.txt");
    pub(crate) const BY_MAIN: &[u8] = include_bytes!("../../tests/fixtures/signing/main.minisig");
    const BY_UNKNOWN: &[u8] = include_bytes!("../../tests/fixtures/signing/unknown.minisig");

    /// The test main and backup keys, in place of featherplay's.
    pub(crate) fn test_keys() -> Keys {
        Keys::from_pub(&[
            include_str!("../../tests/fixtures/signing/test-main.pub"),
            include_str!("../../tests/fixtures/signing/test-backup.pub"),
        ])
    }

    fn plan(base: &str) -> Plan {
        let asset = |name: &str| Asset {
            name: name.into(),
            url: format!("{base}/{name}"),
        };
        Plan {
            version: "9.0.0".into(),
            setup: asset("featherplay-setup-9.0.0.exe"),
            sums: asset(SUMS),
            sig: Some(asset(SUMS_SIG)),
        }
    }

    /// The release's files, `sig` as its signature, and `setup` as its setup.
    fn release_files(sig: &'static [u8], setup: &'static [u8]) -> String {
        serve(move |path| match path {
            "/SHA256SUMS.txt" => (200, SIGNED_SUMS.to_vec()),
            "/SHA256SUMS.txt.minisig" => (200, sig.to_vec()),
            "/featherplay-setup-9.0.0.exe" => (200, setup.to_vec()),
            _ => (404, Vec::new()),
        })
    }

    #[tokio::test]
    async fn a_setup_that_matches_its_checksum_is_kept() {
        let cache = test_cache("match");
        let base = release_files(BY_MAIN, SETUP);

        let ready = fetch(&cache, &plan(&base), &Origin::local(), &test_keys())
            .await
            .unwrap();

        assert_eq!(ready.version, "9.0.0");
        assert_eq!(ready.file, "featherplay-setup-9.0.0.exe");
        let path = ready.path(&cache).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), SETUP);
        assert!(ready.verified(&cache));
        assert!(
            !path
                .with_file_name("featherplay-setup-9.0.0.exe.part")
                .exists()
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn a_release_without_a_signature_is_not_downloaded() {
        let cache = test_cache("unsigned");
        let base = release_files(BY_MAIN, SETUP);
        let mut unsigned = plan(&base);
        unsigned.sig = None;

        let err = fetch(&cache, &unsigned, &Origin::local(), &test_keys())
            .await
            .unwrap_err();

        assert_eq!(err, FetchError::Unverified(SignatureError::Missing));
        assert!(!cache.join("updates").exists(), "nothing downloaded");
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn checksums_signed_with_another_key_download_no_setup() {
        let cache = test_cache("unknown-key");
        let base = release_files(BY_UNKNOWN, SETUP);

        let err = fetch(&cache, &plan(&base), &Origin::local(), &test_keys())
            .await
            .unwrap_err();

        assert_eq!(err, FetchError::Unverified(SignatureError::UnknownKey));
        assert!(!cache.join("updates").exists(), "nothing downloaded");
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn a_version_not_newer_than_this_one_is_refused() {
        let cache = test_cache("not-newer");
        let mut old = plan("http://127.0.0.1:9");
        old.version = super::super::CURRENT.into();

        let err = fetch(&cache, &old, &Origin::local(), &test_keys())
            .await
            .unwrap_err();

        assert!(
            matches!(&err, FetchError::Refused(e) if e.contains("isn't newer")),
            "{err}"
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn a_setup_that_does_not_match_is_deleted() {
        let cache = test_cache("mismatch");
        let base = release_files(BY_MAIN, b"something else");

        let err = fetch(&cache, &plan(&base), &Origin::local(), &test_keys())
            .await
            .unwrap_err();

        assert!(
            matches!(&err, FetchError::Refused(e) if e.contains("sha256 mismatch")),
            "{err}"
        );
        let left: Vec<_> = std::fs::read_dir(cache.join("updates"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(left.is_empty(), "left behind: {left:?}");
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn a_setup_missing_from_the_checksums_is_not_downloaded() {
        let cache = test_cache("unlisted");
        let base = release_files(BY_MAIN, SETUP);
        // The signed checksums list the x64 and ARM64 setups, not this one.
        let mut unlisted = plan(&base);
        unlisted.setup.name = "featherplay-setup-9.0.1.exe".into();

        let err = fetch(&cache, &unlisted, &Origin::local(), &test_keys())
            .await
            .unwrap_err();

        assert!(
            matches!(&err, FetchError::Refused(e) if e.contains("no sha256")),
            "{err}"
        );
        assert!(!cache.join("updates").exists());
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[tokio::test]
    async fn a_url_off_the_projects_releases_is_refused() {
        let cache = test_cache("elsewhere");
        let mut p = plan("https://example.com/candygoogy/featherplay/releases/download/v9.0.0");
        p.sums.url = "https://example.com/SHA256SUMS.txt".into();

        let err = fetch(&cache, &p, &Origin::github(), &test_keys())
            .await
            .unwrap_err();

        assert!(
            matches!(&err, FetchError::Refused(e) if e.contains("not a featherplay release file")),
            "{err}"
        );
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn a_release_url_that_climbs_out_of_the_projects_releases_is_refused() {
        let gh = Origin::github();
        assert!(gh.allows(&format!("{DOWNLOADS}v0.2.0/SHA256SUMS.txt")));
        assert!(!gh.allows(&format!("{DOWNLOADS}../../../../someone/else/x.exe")));
        assert!(!gh.allows(&format!("{DOWNLOADS}%2e%2e/%2e%2e/%2e%2e/%2e%2e/x/y/z.exe")));
        assert!(!gh.allows(&DOWNLOADS.replace("https", "http")));
    }

    #[test]
    fn github_downloads_and_asset_hosts_are_the_only_redirects() {
        let ok = |u: &str| redirect_target(&reqwest::Url::parse(u).unwrap());
        assert!(ok(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2"
        ));
        assert!(ok(
            "https://github.com/candygoogy/featherplay/releases/download/v0.2.0/SHA256SUMS.txt"
        ));
        assert!(!ok("http://release-assets.githubusercontent.com/x"));
        assert!(!ok(
            "https://github.com/someone/else/releases/download/v1/x.exe"
        ));
        assert!(!ok("https://githubusercontent.com.example.net/x"));
    }

    #[test]
    fn old_setups_are_cleaned_and_the_ready_one_kept() {
        let cache = test_cache("clean");
        let dir = cache.join("updates");
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "featherplay-setup-0.1.5.exe",
            "featherplay-setup-0.2.0.exe",
            "featherplay-setup-0.2.0.exe.part",
            "featherplay-x86_64.AppImage.part",
            "install.log",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let ready = Ready {
            version: "0.2.0".into(),
            file: "featherplay-setup-0.2.0.exe".into(),
            sha256: String::new(),
            signed: true,
        };

        clean(&cache, Some(&ready));

        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["featherplay-setup-0.2.0.exe", "install.log"]);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn a_stamp_naming_another_file_has_no_path() {
        let cache = Path::new("D:/cache");
        for file in [
            "..\\featherplay.exe",
            "featherplay-setup-0.2.0.exe\\..\\..\\x.exe",
            "featherplay-setup-0.2.0.bat",
            "other.exe",
            "featherplay-x/../../x.AppImage",
            "featherplay-x86_64 .AppImage",
            "../featherplay-x86_64.AppImage",
            "other.AppImage",
        ] {
            let r = Ready {
                version: "0.2.0".into(),
                file: file.into(),
                sha256: String::new(),
                signed: true,
            };
            assert_eq!(r.path(cache), None, "{file}");
        }
    }
}
