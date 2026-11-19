//! Update checks (shared by the desktop apps and Android): find the newest GitHub release, say whether it is newer
//! than this build, pick the right installer for this platform and install type, download it and verify it against the
//! release's `.sha256` asset.
//!
//! Nothing here is UI. The apps hold one [`Updater`], poll [`Updater::status`] and call `check`, `download`, `cancel`,
//! `skip_version` and `remind_later`. The release source is one constant ([`DEFAULT_UPDATE_URL`]) that can be replaced with
//! the `SUPERCHAT_UPDATE_URL` environment variable or the hidden `update_url` preference (tests point it at a fake server).
//! The request carries only `User-Agent: SuperChat/<version>`: no account, device or server information.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use proto::release::{is_newer, parse_version, trim_notes, Asset, ReleaseInfo, Version, DEFAULT_UPDATE_URL, RELEASES_PAGE};
/// Automatic checks run on startup (at most once an hour) and then every 24 hours.
pub const AUTO_CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
const STARTUP_MIN_GAP_MS: i64 = 60 * 60 * 1000;
/// After a failed automatic check (offline, rate limit) the next automatic try waits this long. Without it the app polled
/// the server on every timer tick, which would hit GitHub's anonymous rate limit within a minute.
pub const AUTO_RETRY_MS: i64 = 60 * 60 * 1000;

/// Whether the automatic check is due: `first` is the first call since the app started.
pub fn auto_due(first: bool, last_check_ms: i64, last_fail_ms: i64, now: i64) -> bool {
    let last_try = last_check_ms.max(last_fail_ms);
    if first {
        return now - last_try >= STARTUP_MIN_GAP_MS;
    }
    if last_fail_ms > last_check_ms {
        return now - last_fail_ms >= AUTO_RETRY_MS;
    }
    now - last_check_ms >= AUTO_CHECK_INTERVAL_MS
}
/// "Remind me later" hides the banner for this long.
pub const REMIND_LATER_MS: i64 = 24 * 60 * 60 * 1000;

/// The update URL to use: environment `SUPERCHAT_UPDATE_URL` (or `ATOMIC_UPDATE_URL`) first, then the hidden preference,
/// then [`DEFAULT_UPDATE_URL`].
pub fn update_url(pref_override: &str) -> String {
    if let Ok(v) = proto::env_var("ATOMIC_UPDATE_URL") {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    let p = pref_override.trim();
    if p.is_empty() {
        DEFAULT_UPDATE_URL.to_string()
    } else {
        p.to_string()
    }
}

/// Reads the API's answer: one release object (`/releases/latest`) or a list (`/releases`). Drafts and pre-releases are
/// skipped; of a list the highest version is returned. `Ok(None)` when nothing eligible is in it.
pub fn parse_releases(json: &str) -> Result<Option<ReleaseInfo>, UpdateError> {
    proto::release::parse_releases(json).map_err(|_| UpdateError::BadAnswer)
}

// ------------------------------------------------------------------------------------------------
// Platform and assets

/// Which installer of a release suits this running app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Android,
    /// Linux, started from an AppImage (`$APPIMAGE` is set): replace that file.
    LinuxAppImage,
    /// Linux installed from the `.deb` (or built from source): download the `.deb`.
    LinuxDeb,
    /// Linux Flatpak: Flatpak updates it.
    Flatpak,
    /// Windows, installed with the setup program (an uninstaller sits next to the exe).
    WindowsInstalled,
    /// Windows, the portable exe.
    WindowsPortable,
    Other,
}

/// The platform this process runs as.
pub fn detect_platform() -> Platform {
    if cfg!(target_os = "android") {
        return Platform::Android;
    }
    if cfg!(windows) {
        let exe = std::env::current_exe().ok();
        let installed = exe.as_ref().and_then(|e| e.parent()).map(|d| d.join("uninstall.exe").exists() || d.join("Uninstall.exe").exists()).unwrap_or(false);
        return if installed { Platform::WindowsInstalled } else { Platform::WindowsPortable };
    }
    if cfg!(target_os = "linux") {
        if std::env::var("FLATPAK_ID").is_ok() || Path::new("/.flatpak-info").exists() {
            return Platform::Flatpak;
        }
        if std::env::var("APPIMAGE").map(|p| Path::new(&p).is_file()).unwrap_or(false) {
            return Platform::LinuxAppImage;
        }
        return Platform::LinuxDeb;
    }
    Platform::Other
}

impl Platform {
    /// Short name for the UI ("AppImage", "Windows installer"...).
    pub fn label(&self) -> &'static str {
        match self {
            Platform::Android => "Android APK",
            Platform::LinuxAppImage => "AppImage",
            Platform::LinuxDeb => ".deb package",
            Platform::Flatpak => "Flatpak",
            Platform::WindowsInstalled => "Windows installer",
            Platform::WindowsPortable => "Windows portable",
            Platform::Other => "this platform",
        }
    }
}

/// An installer to download plus its checksum asset (None when the release has no `.sha256` next to it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetPick {
    pub asset: Asset,
    pub sha256: Option<Asset>,
}

/// The asset for `platform` in `rel`, with its `.sha256` file. Names follow `docs/DEPLOY.md`:
/// `superchat-<v>-android.apk`, `superchat_<v>-1_amd64.deb`, `SuperChat-<v>-x86_64.AppImage`,
/// `superchat-<v>-x64-setup.exe`, `superchat-<v>-x64-portable.exe`.
pub fn select_asset(rel: &ReleaseInfo, platform: Platform) -> Option<AssetPick> {
    let x86_64 = cfg!(not(any(target_arch = "aarch64", target_arch = "arm")));
    let ok = |n: &str| -> bool {
        let l = n.to_ascii_lowercase();
        match platform {
            Platform::Android => l.ends_with(".apk"),
            Platform::LinuxAppImage => l.ends_with(".appimage") && (l.contains("x86_64") || l.contains("amd64")) == x86_64,
            Platform::LinuxDeb => l.ends_with(".deb") && (l.contains("amd64") || l.contains("x86_64")) == x86_64,
            Platform::WindowsInstalled => l.ends_with("-setup.exe") || l.ends_with("setup.exe"),
            Platform::WindowsPortable => l.ends_with("-portable.exe") || l.ends_with("portable.exe"),
            Platform::Flatpak | Platform::Other => false,
        }
    };
    let asset = rel.assets.iter().find(|a| ok(&a.name))?.clone();
    let sha = rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case(&format!("{}.sha256", asset.name))).cloned();
    Some(AssetPick { asset, sha256: sha })
}

// ------------------------------------------------------------------------------------------------
// Checksums

/// The first 64-digit hex word of a `.sha256` file (`<hash>  <name>`, or just the hash).
pub fn parse_sha256(text: &str) -> Option<String> {
    text.split_whitespace().map(|w| w.trim_start_matches('*')).find(|w| w.len() == 64 && w.bytes().all(|b| b.is_ascii_hexdigit())).map(|w| w.to_ascii_lowercase())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
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

/// Compares the file with the expected digest (hex, any case).
pub fn verify_file(path: &Path, expected: &str) -> Result<(), UpdateError> {
    let got = sha256_file(path).map_err(|e| UpdateError::Io(e.to_string()))?;
    if got.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch)
    }
}

// ------------------------------------------------------------------------------------------------
// Errors

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UpdateError {
    #[error("Can't reach GitHub. Check your internet connection and try again.")]
    Offline,
    #[error("{}", match .0 { Some(m) if *m > 0 => format!("GitHub is limiting update checks from this network. Try again in about {m} minutes."), _ => "GitHub is limiting update checks from this network. Try again later.".to_string() })]
    RateLimited(Option<u64>),
    #[error("No release has been published yet.")]
    NotFound,
    #[error("GitHub answered with an error (HTTP {0}). Try again later.")]
    Http(u16),
    #[error("The release information could not be read.")]
    BadAnswer,
    #[error("This release has no checksum file, so the download can't be verified. It was not installed.")]
    NoChecksum,
    #[error("The downloaded file does not match its checksum, so it was discarded. Try again.")]
    ChecksumMismatch,
    #[error("The download address is not secure (https is required).")]
    Insecure,
    #[error("This release has no installer for this device.")]
    NoAsset,
    #[error("Download cancelled.")]
    Cancelled,
    #[error("Could not save the update: {0}")]
    Io(String),
}

fn net(e: reqwest::Error) -> UpdateError {
    if e.is_connect() || e.is_timeout() || e.is_request() {
        UpdateError::Offline
    } else if e.is_decode() || e.is_body() {
        UpdateError::Io(e.to_string())
    } else {
        UpdateError::Offline
    }
}

fn http_client(current: &str) -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .user_agent(format!("SuperChat/{current}"))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| UpdateError::Io(e.to_string()))
}

fn download_client(current: &str) -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .user_agent(format!("SuperChat/{current}"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| UpdateError::Io(e.to_string()))
}

fn rate_limit_error(resp: &reqwest::Response) -> UpdateError {
    let h = resp.headers();
    let reset = h.get("x-ratelimit-reset").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok());
    let retry_after = h.get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let mins = retry_after.map(|s| s.div_ceil(60)).or_else(|| reset.map(|r| ((r - now_ms() / 1000).max(0) as u64).div_ceil(60)));
    UpdateError::RateLimited(mins)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Fetches the newest eligible release from `url`. `Ok(None)` when the answer holds only drafts and pre-releases.
pub async fn fetch_latest(url: &str, current: &str) -> Result<Option<ReleaseInfo>, UpdateError> {
    let http = http_client(current)?;
    let resp = http.get(url).header("Accept", "application/vnd.github+json").send().await.map_err(net)?;
    let st = resp.status();
    if st.as_u16() == 403 || st.as_u16() == 429 {
        return Err(rate_limit_error(&resp));
    }
    if st.as_u16() == 404 {
        return Err(UpdateError::NotFound);
    }
    if !st.is_success() {
        return Err(UpdateError::Http(st.as_u16()));
    }
    let text = resp.text().await.map_err(net)?;
    parse_releases(&text)
}

/// Downloads the picked asset into `dest_dir`, checks it against its `.sha256` and returns the final path. A file that fails
/// the check is deleted. `progress(done, total)` is called as bytes arrive; setting `cancel` stops the download.
pub async fn download_verified(
    pick: &AssetPick,
    current: &str,
    dest_dir: &Path,
    allow_http: bool,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<PathBuf, UpdateError> {
    use futures_util::StreamExt;
    use std::io::Write;
    let secure = |u: &str| u.starts_with("https://") || allow_http;
    if !secure(&pick.asset.url) || pick.sha256.as_ref().map(|s| !secure(&s.url)).unwrap_or(false) {
        return Err(UpdateError::Insecure);
    }
    let sha_asset = pick.sha256.as_ref().ok_or(UpdateError::NoChecksum)?;
    let name = &pick.asset.name;
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(UpdateError::Io("unsafe file name in the release".into()));
    }
    let http = download_client(current)?;
    // The checksum first: it is tiny, and without it there is nothing to download for.
    let sr = http.get(&sha_asset.url).send().await.map_err(net)?;
    if !sr.status().is_success() {
        return Err(UpdateError::NoChecksum);
    }
    let sha_text = sr.text().await.map_err(net)?;
    let expected = parse_sha256(&sha_text).ok_or(UpdateError::NoChecksum)?;

    std::fs::create_dir_all(dest_dir).map_err(|e| UpdateError::Io(e.to_string()))?;
    let part = dest_dir.join(format!("{name}.part"));
    let final_path = dest_dir.join(name);
    let resp = http.get(&pick.asset.url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(UpdateError::Http(resp.status().as_u16()));
    }
    let total = resp.content_length().filter(|n| *n > 0).unwrap_or(pick.asset.size);
    let mut file = std::fs::File::create(&part).map_err(|e| UpdateError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    let mut last = std::time::Instant::now();
    progress(0, total);
    let result: Result<(), UpdateError> = async {
        while let Some(chunk) = stream.next().await {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err(UpdateError::Cancelled);
            }
            let chunk = chunk.map_err(net)?;
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(|e| UpdateError::Io(e.to_string()))?;
            done += chunk.len() as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                last = std::time::Instant::now();
                progress(done, total.max(done));
            }
        }
        if cancel.load(AtomicOrdering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        file.flush().map_err(|e| UpdateError::Io(e.to_string()))?;
        Ok(())
    }
    .await;
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    progress(done, done);
    if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(&expected) {
        let _ = std::fs::remove_file(&part);
        return Err(UpdateError::ChecksumMismatch);
    }
    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&part, &final_path).map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(final_path)
}

// ------------------------------------------------------------------------------------------------
// The updater (state machine the UIs poll)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    /// Downloaded and verified; `Status::file` is the installer.
    Ready,
    Failed,
}

/// What a failure was about, so "Try again" repeats the right step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Check,
    Download,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub phase: Phase,
    pub current: String,
    pub platform: Platform,
    pub release: Option<ReleaseInfo>,
    pub pick: Option<AssetPick>,
    pub done: u64,
    pub total: u64,
    pub error: String,
    pub stage: Stage,
    pub file: Option<PathBuf>,
    pub last_check_ms: i64,
    pub skipped: String,
    pub remind_until_ms: i64,
}

impl Status {
    /// A newer release is known (whatever has been done about it since).
    pub fn has_update(&self) -> bool {
        self.release.as_ref().map(|r| is_newer(&r.version, &self.current)).unwrap_or(false)
    }
    /// The unobtrusive "Update available" banner: an update exists, was not skipped, and "remind me later" has run out. A
    /// download in progress or finished keeps it visible.
    pub fn show_banner(&self, now_ms: i64) -> bool {
        if !self.has_update() {
            return false;
        }
        if matches!(self.phase, Phase::Downloading | Phase::Ready) {
            return true;
        }
        let v = self.release.as_ref().map(|r| r.version.as_str()).unwrap_or("");
        self.skipped != v && now_ms >= self.remind_until_ms
    }
    /// What to tell the user when the platform cannot install by itself.
    pub fn manual_hint(&self) -> String {
        match self.platform {
            Platform::Flatpak => "This copy was installed with Flatpak. Update it with: flatpak update au.superchat.SuperChat".into(),
            _ if self.pick.is_none() => format!("Download the new version from {RELEASES_PAGE}"),
            _ => String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    last_check_ms: i64,
    release: Option<ReleaseInfo>,
    skipped: String,
    remind_until_ms: i64,
}

struct Inner {
    st: Status,
    gen: u64,
    started: bool,
    /// When the last check failed (not saved; a restart tries again after the startup gap rule).
    last_fail_ms: i64,
}

/// One per app. Cheap to clone.
#[derive(Clone)]
pub struct Updater {
    inner: Arc<Mutex<Inner>>,
    cancel: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
    dir: PathBuf,
    url: Arc<Mutex<String>>,
    allow_http: Arc<AtomicBool>,
}

impl Updater {
    /// `dir` keeps the cached answer and the skip/remind choices (`update.json`); `url` is the already resolved update
    /// address (see [`update_url`]); `platform` normally [`detect_platform`].
    pub fn new(dir: &Path, handle: tokio::runtime::Handle, current: &str, platform: Platform, url: String) -> Updater {
        let saved: Saved = std::fs::read_to_string(dir.join("update.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut st = Status {
            phase: Phase::Idle,
            current: current.to_string(),
            platform,
            release: None,
            pick: None,
            done: 0,
            total: 0,
            error: String::new(),
            stage: Stage::Check,
            file: None,
            last_check_ms: saved.last_check_ms,
            skipped: saved.skipped,
            remind_until_ms: saved.remind_until_ms,
        };
        if let Some(r) = saved.release.filter(|r| is_newer(&r.version, current)) {
            st.pick = select_asset(&r, platform);
            st.release = Some(r);
            st.phase = Phase::Available;
        }
        let allow_http = !url.starts_with("https://") || url != DEFAULT_UPDATE_URL;
        Updater {
            inner: Arc::new(Mutex::new(Inner { st, gen: 0, started: false, last_fail_ms: 0 })),
            cancel: Arc::new(AtomicBool::new(false)),
            handle,
            dir: dir.to_path_buf(),
            url: Arc::new(Mutex::new(url)),
            allow_http: Arc::new(AtomicBool::new(allow_http)),
        }
    }

    pub fn set_url(&self, url: String) {
        let allow = url != DEFAULT_UPDATE_URL;
        *self.url.lock().unwrap() = url;
        self.allow_http.store(allow, AtomicOrdering::Relaxed);
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().st.clone()
    }

    fn save(&self, g: &Inner) {
        let s = Saved { last_check_ms: g.st.last_check_ms, release: g.st.release.clone(), skipped: g.st.skipped.clone(), remind_until_ms: g.st.remind_until_ms };
        let _ = std::fs::create_dir_all(&self.dir);
        if let Ok(j) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(self.dir.join("update.json"), j);
        }
    }

    /// Looks for a newer release now. `manual` (the button) always runs; the automatic one is skipped while a download runs.
    pub fn check(&self) {
        {
            let mut g = self.inner.lock().unwrap();
            if matches!(g.st.phase, Phase::Checking | Phase::Downloading) {
                return;
            }
            g.st.phase = Phase::Checking;
            g.st.error.clear();
            g.st.stage = Stage::Check;
            g.gen += 1;
        }
        let (me, url) = (self.clone(), self.url.lock().unwrap().clone());
        let current = self.status().current;
        self.handle.spawn(async move {
            let res = fetch_latest(&url, &current).await;
            me.finish_check(res);
        });
    }

    fn finish_check(&self, res: Result<Option<ReleaseInfo>, UpdateError>) {
        let mut g = self.inner.lock().unwrap();
        match res {
            Ok(rel) => {
                g.st.last_check_ms = now_ms();
                match rel.filter(|r| is_newer(&r.version, &g.st.current)) {
                    Some(r) => {
                        let same = g.st.release.as_ref().map(|o| o.version == r.version).unwrap_or(false);
                        g.st.pick = select_asset(&r, g.st.platform);
                        if !same || g.st.phase != Phase::Ready {
                            g.st.file = None;
                        }
                        g.st.release = Some(r);
                        g.st.phase = Phase::Available;
                    }
                    None => {
                        g.st.release = None;
                        g.st.pick = None;
                        g.st.phase = Phase::UpToDate;
                    }
                }
            }
            Err(e) => {
                g.last_fail_ms = now_ms();
                g.st.phase = Phase::Failed;
                g.st.stage = Stage::Check;
                g.st.error = e.to_string();
            }
        }
        self.save(&g);
    }

    /// Automatic check, called now and then by the app (a minute timer is fine). Does a check on the first call (unless one
    /// ran in the last hour) and then every 24 hours, only when `enabled`.
    pub fn auto_tick(&self, enabled: bool, now: i64) {
        if !enabled {
            return;
        }
        let run = {
            let mut g = self.inner.lock().unwrap();
            if matches!(g.st.phase, Phase::Checking | Phase::Downloading) {
                false
            } else {
                let first = !g.started;
                g.started = true;
                auto_due(first, g.st.last_check_ms, g.last_fail_ms, now)
            }
        };
        if run {
            self.check();
        }
    }

    /// Hides the banner for this version for good (until a newer one appears).
    pub fn skip_version(&self) {
        let mut g = self.inner.lock().unwrap();
        if let Some(r) = &g.st.release {
            g.st.skipped = r.version.clone();
        }
        self.save(&g);
    }

    /// Hides the banner for a day.
    pub fn remind_later(&self, now: i64) {
        let mut g = self.inner.lock().unwrap();
        g.st.remind_until_ms = now + REMIND_LATER_MS;
        self.save(&g);
    }

    /// Forgets "skip" and "remind later" (the button on the About page always shows the update regardless).
    pub fn clear_choices(&self) {
        let mut g = self.inner.lock().unwrap();
        g.st.skipped.clear();
        g.st.remind_until_ms = 0;
        self.save(&g);
    }

    /// Downloads and verifies the installer into `dest_dir`. Needs an available update with an asset for this platform.
    pub fn download(&self, dest_dir: PathBuf) {
        let (pick, current) = {
            let mut g = self.inner.lock().unwrap();
            if matches!(g.st.phase, Phase::Checking | Phase::Downloading) {
                return;
            }
            let Some(pick) = g.st.pick.clone() else {
                g.st.phase = Phase::Failed;
                g.st.stage = Stage::Download;
                g.st.error = UpdateError::NoAsset.to_string();
                return;
            };
            g.st.phase = Phase::Downloading;
            g.st.stage = Stage::Download;
            g.st.error.clear();
            g.st.done = 0;
            g.st.total = pick.asset.size;
            g.st.file = None;
            g.gen += 1;
            (pick, g.st.current.clone())
        };
        self.cancel.store(false, AtomicOrdering::Relaxed);
        let me = self.clone();
        let allow = self.allow_http.load(AtomicOrdering::Relaxed);
        self.handle.spawn(async move {
            let m2 = me.clone();
            let progress = move |d: u64, t: u64| {
                let mut g = m2.inner.lock().unwrap();
                g.st.done = d;
                g.st.total = t;
            };
            let res = download_verified(&pick, &current, &dest_dir, allow, &me.cancel, &progress).await;
            let mut g = me.inner.lock().unwrap();
            match res {
                Ok(p) => {
                    g.st.phase = Phase::Ready;
                    g.st.file = Some(p);
                }
                Err(UpdateError::Cancelled) => {
                    g.st.phase = Phase::Available;
                    g.st.done = 0;
                }
                Err(e) => {
                    g.st.phase = Phase::Failed;
                    g.st.stage = Stage::Download;
                    g.st.error = e.to_string();
                }
            }
        });
    }

    /// Stops a running download (the partial file is deleted).
    pub fn cancel(&self) {
        self.cancel.store(true, AtomicOrdering::Relaxed);
    }

    /// "Try again" after a failure: repeats the step that failed.
    pub fn retry(&self, dest_dir: PathBuf) {
        let stage = {
            let mut g = self.inner.lock().unwrap();
            if g.st.phase != Phase::Failed {
                return;
            }
            let s = g.st.stage;
            if s == Stage::Download {
                g.st.phase = Phase::Available;
            } else {
                g.st.phase = Phase::Idle;
            }
            s
        };
        match stage {
            Stage::Check => self.check(),
            Stage::Download => self.download(dest_dir),
        }
    }

    /// An install attempt failed or was declined (Android's installer said so): shown on the About page.
    pub fn report_install_failure(&self, message: String) {
        let mut g = self.inner.lock().unwrap();
        g.st.phase = Phase::Failed;
        g.st.stage = Stage::Download;
        g.st.error = message;
        g.st.file = None;
    }

    /// Back to "update available" (for example the user closed the installer): the downloaded file can be installed again.
    pub fn reset_to_available(&self) {
        let mut g = self.inner.lock().unwrap();
        if g.st.release.is_some() {
            g.st.phase = Phase::Available;
        }
    }

    /// Where the installer goes by default: next to the AppImage when it can be written, else under `fallback`.
    pub fn download_dir(platform: Platform, fallback: &Path) -> PathBuf {
        if platform == Platform::LinuxAppImage {
            if let Some(dir) = std::env::var("APPIMAGE").ok().and_then(|p| Path::new(&p).parent().map(|d| d.to_path_buf())) {
                let probe = dir.join(".superchat-write-test");
                if std::fs::write(&probe, b"x").is_ok() {
                    let _ = std::fs::remove_file(&probe);
                    return dir;
                }
            }
        }
        fallback.join("updates")
    }
}

// ------------------------------------------------------------------------------------------------
// Installing on the desktop

/// Replaces the AppImage at `target` with the verified file `new`: the old file is kept as `<target>.bak` and the new one is
/// moved into place with one atomic rename, so the path always holds a complete AppImage.
pub fn replace_appimage(new: &Path, target: &Path) -> std::io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    let mut bak_name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    bak_name.push(".bak");
    let bak = target.with_file_name(bak_name);
    if target.exists() {
        let _ = std::fs::remove_file(&bak);
        if std::fs::hard_link(target, &bak).is_err() {
            std::fs::copy(target, &bak)?;
        }
    }
    std::fs::rename(new, target)?;
    Ok(bak)
}

/// `sudo apt install '<path>'`: the exact command for a downloaded `.deb`.
pub fn deb_install_command(file: &Path) -> String {
    format!("sudo apt install '{}'", file.display().to_string().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(version: &str, names: &[&str]) -> ReleaseInfo {
        ReleaseInfo {
            version: version.into(),
            tag: format!("v{version}"),
            name: format!("SuperChat {version}"),
            notes: String::new(),
            html_url: "https://example.org/r".into(),
            published: String::new(),
            assets: names.iter().map(|n| Asset { name: n.to_string(), url: format!("https://example.org/dl/{n}"), size: 10 }).collect(),
        }
    }

    fn all_assets(v: &str) -> ReleaseInfo {
        rel(
            v,
            &[
                &format!("superchat-{v}-android.apk"),
                &format!("superchat-{v}-android.apk.sha256"),
                &format!("superchat_{v}-1_amd64.deb"),
                &format!("superchat_{v}-1_amd64.deb.sha256"),
                &format!("SuperChat-{v}-x86_64.AppImage"),
                &format!("SuperChat-{v}-x86_64.AppImage.sha256"),
                &format!("superchat-{v}-x64-setup.exe"),
                &format!("superchat-{v}-x64-setup.exe.sha256"),
                &format!("superchat-{v}-x64-portable.exe"),
                &format!("superchat-{v}-x64-portable.exe.sha256"),
                "superchat-server-notes.txt",
            ],
        )
    }

    #[test]
    fn auto_check_backs_off_after_a_failure() {
        let h = 60 * 60 * 1000;
        // never checked: the first call runs
        assert!(auto_due(true, 0, 0, 1_000_000_000));
        // checked 10 minutes ago: the startup call waits
        assert!(!auto_due(true, 1_000_000_000, 0, 1_000_000_000 + 10 * 60 * 1000));
        // a failure just now: no new try on the next ticks, one after an hour
        let t = 5_000_000_000;
        assert!(!auto_due(false, 1, t, t + 300));
        assert!(!auto_due(false, 1, t, t + h - 1));
        assert!(auto_due(false, 1, t, t + h));
        // a success: next one in 24 hours
        assert!(!auto_due(false, t, 0, t + 23 * h));
        assert!(auto_due(false, t, 0, t + 24 * h));
    }

    #[test]
    fn semver_compare() {
        assert!(is_newer("1.3.10", "1.3.9"));
        assert!(!is_newer("1.3.9", "1.3.10"));
        assert!(is_newer("v1.3.1", "1.3.0"));
        assert!(is_newer("V2.0.0", "v1.99.99"));
        assert!(!is_newer("1.3.1", "v1.3.1"));
        assert!(is_newer("1.4", "1.3.9"));
        assert!(is_newer("1.3.1", "1.3.1-rc.1"));
        assert!(!is_newer("1.3.1-rc.1", "1.3.1"));
        assert!(is_newer("1.3.1-rc.10", "1.3.1-rc.9"));
        assert!(is_newer("1.3.1-beta", "1.3.1-alpha"));
        assert!(!is_newer("1.3.1+build7", "1.3.1"));
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("1.0.0", "garbage"));
        assert_eq!(parse_version("v1.3.1").unwrap(), Version { major: 1, minor: 3, patch: 1, pre: String::new() });
        assert!(parse_version("1.2.3.4").is_none());
        assert!(parse_version("").is_none());
    }

    #[test]
    fn asset_selection_per_platform() {
        let r = all_assets("1.3.1");
        let pick = |p| select_asset(&r, p);
        assert_eq!(pick(Platform::Android).unwrap().asset.name, "superchat-1.3.1-android.apk");
        assert_eq!(pick(Platform::Android).unwrap().sha256.unwrap().name, "superchat-1.3.1-android.apk.sha256");
        assert_eq!(pick(Platform::LinuxAppImage).unwrap().asset.name, "SuperChat-1.3.1-x86_64.AppImage");
        assert_eq!(pick(Platform::LinuxDeb).unwrap().asset.name, "superchat_1.3.1-1_amd64.deb");
        assert_eq!(pick(Platform::WindowsInstalled).unwrap().asset.name, "superchat-1.3.1-x64-setup.exe");
        assert_eq!(pick(Platform::WindowsPortable).unwrap().asset.name, "superchat-1.3.1-x64-portable.exe");
        assert!(pick(Platform::Flatpak).is_none());
        assert!(pick(Platform::Other).is_none());
        // A release without the checksum file still selects the installer; the download then refuses it.
        let r2 = rel("1.3.1", &["superchat-1.3.1-android.apk"]);
        assert!(select_asset(&r2, Platform::Android).unwrap().sha256.is_none());
        // Nothing for this platform.
        assert!(select_asset(&rel("1.3.1", &["superchat-1.3.1-android.apk"]), Platform::LinuxDeb).is_none());
    }

    #[test]
    fn sha256_parse_and_verify() {
        let h = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(parse_sha256(&format!("{h}  superchat.apk\n")).unwrap(), h);
        assert_eq!(parse_sha256(&format!("{}\n", h.to_uppercase())).unwrap(), h);
        assert_eq!(parse_sha256(&format!("SHA256 ({h}) = x\n{h} *file")).unwrap_or_default(), h);
        assert!(parse_sha256("not a hash").is_none());
        assert!(parse_sha256(&h[..63]).is_none());

        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.bin");
        std::fs::write(&f, b"").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), h);
        assert!(verify_file(&f, h).is_ok());
        assert!(verify_file(&f, &h.to_uppercase()).is_ok());
        std::fs::write(&f, b"tampered").unwrap();
        assert_eq!(verify_file(&f, h), Err(UpdateError::ChecksumMismatch));
    }

    const FIXTURE: &str = r###"{
        "url": "https://api.github.com/repos/uri-travoski/superchat/releases/1",
        "tag_name": "v1.3.1", "name": "SuperChat 1.3.1", "draft": false, "prerelease": false,
        "html_url": "https://github.com/uri-travoski/superchat/releases/tag/v1.3.1",
        "published_at": "2026-11-20T01:02:03Z",
        "body": "## What's new\r\n\r\n\r\n* **Update checks** in every app\r\n- `Check for updates` button\r\n\r\n### Fixed\r\n- a thing",
        "assets": [
            {"name": "superchat-1.3.1-android.apk", "size": 1234, "browser_download_url": "https://github.com/x/releases/download/v1.3.1/superchat-1.3.1-android.apk"},
            {"name": "superchat-1.3.1-android.apk.sha256", "size": 90, "browser_download_url": "https://github.com/x/releases/download/v1.3.1/superchat-1.3.1-android.apk.sha256"}
        ]}"###;

    #[test]
    fn parses_release_fixture() {
        let r = parse_releases(FIXTURE).unwrap().unwrap();
        assert_eq!(r.version, "1.3.1");
        assert_eq!(r.tag, "v1.3.1");
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.assets[0].size, 1234);
        assert!(r.notes.starts_with("What's new\n\n\u{2022} Update checks in every app"), "{:?}", r.notes);
        assert!(!r.notes.contains("**") && !r.notes.contains('#') && !r.notes.contains('`'));
        assert!(r.notes.ends_with("\u{2022} a thing"));
    }

    #[test]
    fn skips_drafts_and_prereleases() {
        let one = |extra: &str| format!(r#"{{"tag_name":"v9.0.0","body":"","assets":[]{extra}}}"#);
        assert_eq!(parse_releases(&one(r#","draft":true"#)).unwrap(), None);
        assert_eq!(parse_releases(&one(r#","prerelease":true"#)).unwrap(), None);
        assert!(parse_releases(&one("")).unwrap().is_some());
        // A list: the highest eligible version wins, whatever the order.
        let list = r#"[
            {"tag_name":"v2.0.0-rc.1","prerelease":true,"assets":[]},
            {"tag_name":"v1.3.9","assets":[]},
            {"tag_name":"v1.3.10","assets":[]},
            {"tag_name":"v3.0.0","draft":true,"assets":[]}
        ]"#;
        assert_eq!(parse_releases(list).unwrap().unwrap().version, "1.3.10");
        assert_eq!(parse_releases("[]").unwrap(), None);
        assert_eq!(parse_releases("nope"), Err(UpdateError::BadAnswer));
        assert_eq!(parse_releases("42"), Err(UpdateError::BadAnswer));
        // GitHub's "Not Found" object has no tag.
        assert_eq!(parse_releases(r#"{"message":"Not Found"}"#).unwrap(), None);
    }

    #[test]
    fn notes_are_trimmed() {
        let long = "word ".repeat(1000);
        let t = trim_notes(&long);
        assert!(t.chars().count() <= 1501 && t.ends_with('\u{2026}'));
        assert_eq!(trim_notes("\n\n  hello  \n\n\n\n"), "hello");
    }

    #[test]
    fn banner_rules() {
        let mut st = Status {
            phase: Phase::Available,
            current: "1.3.0".into(),
            platform: Platform::Android,
            release: Some(rel("1.3.1", &[])),
            pick: None,
            done: 0,
            total: 0,
            error: String::new(),
            stage: Stage::Check,
            file: None,
            last_check_ms: 0,
            skipped: String::new(),
            remind_until_ms: 0,
        };
        assert!(st.show_banner(1000));
        st.remind_until_ms = 5000;
        assert!(!st.show_banner(1000) && st.show_banner(5000));
        st.skipped = "1.3.1".into();
        assert!(!st.show_banner(9000));
        st.phase = Phase::Downloading;
        assert!(st.show_banner(9000));
        st.release = Some(rel("1.3.0", &[]));
        assert!(!st.has_update() && !st.show_banner(9000));
    }

    #[test]
    fn appimage_replace_keeps_backup() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("SuperChat.AppImage");
        std::fs::write(&target, b"old").unwrap();
        let new = d.path().join("SuperChat-1.3.1-x86_64.AppImage");
        std::fs::write(&new, b"new").unwrap();
        let bak = replace_appimage(&new, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(&bak).unwrap(), b"old");
        assert_eq!(bak.file_name().unwrap(), "SuperChat.AppImage.bak");
        assert!(!new.exists());
        // A second update replaces the old backup.
        let new2 = d.path().join("n2");
        std::fs::write(&new2, b"newer").unwrap();
        let bak = replace_appimage(&new2, &target).unwrap();
        assert_eq!(std::fs::read(&bak).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[test]
    fn deb_command_quotes_the_path() {
        assert_eq!(deb_install_command(Path::new("/tmp/a b/superchat.deb")), "sudo apt install '/tmp/a b/superchat.deb'");
        assert_eq!(deb_install_command(Path::new("/tmp/it's.deb")), "sudo apt install '/tmp/it'\\''s.deb'");
    }

    #[test]
    fn url_override_order() {
        std::env::remove_var("SUPERCHAT_UPDATE_URL");
        std::env::remove_var("ATOMIC_UPDATE_URL");
        assert_eq!(update_url(""), DEFAULT_UPDATE_URL);
        assert_eq!(update_url(" http://10.0.2.2:18310/latest "), "http://10.0.2.2:18310/latest");
    }
}
