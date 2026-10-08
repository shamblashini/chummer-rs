//! Update check and one-click update.
//!
//! At start (unless `check_updates=false` in gui.ini) a background thread
//! asks the GitHub releases API for the latest chummer-rs release. A newer
//! version shows a small card: What's new / Update now / Later.
//!
//! "Update now" downloads this platform's package from the release,
//! checks its SHA-256 against the release's `SHA256SUMS`, then:
//! - Windows (installed by the installer): runs the new installer silently;
//!   chummer-rs quits and the installer starts the new version.
//! - macOS: replaces the chummer-rs.app it runs from, then offers a restart.
//! - Linux, `install.sh` layout (`<prefix>/bin`, `<prefix>/share/chummer-rs`)
//!   or an unpacked release archive: replaces the programs and resources,
//!   then offers a restart.
//! - Anything else (system packages, read-only locations, a cargo build
//!   directory, the Windows zip): only the link to the release page.
//!
//! TODO: SHA256SUMS comes from the same release as the package, so it
//! proves the download is intact, not who made it. Sign SHA256SUMS
//! (minisign / ed25519) in the release workflow once a signing key is a
//! repository secret, and check the signature here with the public key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use chummer_core::lang::Language;
use eframe::egui;

/// The GitHub repository releases come from.
pub const REPO: &str = "shamblashini/chummer-rs";
/// gui.ini key of "Check for updates on start" (shared with first-run setup).
pub const SETTING_KEY: &str = "check_updates";
/// Name of the checksum file attached to every release.
pub const SUMS_NAME: &str = "SHA256SUMS";

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

/// A semantic version: `1.2.3`, `v1.2.3`, `1.2.3-beta.1` (build metadata
/// after `+` is ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.split('.').map(str::to_owned).collect()),
            None => (s, Vec::new()),
        };
        let mut nums = core.split('.').map(|n| n.parse::<u64>().ok());
        let v = Version { major: nums.next()??, minor: nums.next().unwrap_or(Some(0))?, patch: nums.next().unwrap_or(Some(0))?, pre };
        nums.next().is_none().then_some(v)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            // A pre-release comes before its release.
            (true, true) => Equal,
            (true, false) => Greater,
            (false, true) => Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Less,
                        (Err(_), Ok(_)) => Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if o != Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

/// The running version.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

// ---------------------------------------------------------------------------
// Releases
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub page: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

/// A release from the GitHub API's JSON (`/repos/{repo}/releases/latest`).
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("bad release data: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("release without tag_name")?.to_owned();
    let version = Version::parse(&tag).ok_or_else(|| format!("release tag {tag} is not a version"))?;
    let assets = v["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|a| Some(Asset { name: a["name"].as_str()?.to_owned(), url: a["browser_download_url"].as_str()?.to_owned(), size: a["size"].as_u64().unwrap_or(0) }))
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version,
        page: v["html_url"].as_str().map_or_else(|| format!("https://github.com/{REPO}/releases/tag/{tag}"), str::to_owned),
        notes: v["body"].as_str().unwrap_or_default().to_owned(),
        assets,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

/// The end of the release asset name the updater installs from, for this
/// platform (the release workflow's names: `chummer-rs-v0.5.0-<suffix>`).
pub fn asset_suffix(os: Os, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        (Os::Windows, "x86_64") => Some("-windows-x86_64-setup.exe"),
        (Os::Mac, "x86_64" | "aarch64") => Some("-macos-universal.zip"),
        (Os::Linux, "x86_64") => Some("-linux-x86_64.tar.gz"),
        _ => None,
    }
}

pub fn select_asset<'a>(assets: &'a [Asset], os: Os, arch: &str) -> Option<&'a Asset> {
    let suffix = asset_suffix(os, arch)?;
    assets.iter().find(|a| a.name.starts_with("chummer-rs-") && a.name.ends_with(suffix))
}

// ---------------------------------------------------------------------------
// Checksums
// ---------------------------------------------------------------------------

/// `sha256sum` output: file name -> lower-case hex digest.
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, name) = l.trim_end().split_once(char::is_whitespace)?;
            // `hash  name` (text mode) or `hash *name` (binary mode).
            let name = name.trim_start().trim_start_matches('*');
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !name.is_empty()).then(|| (name.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Check `bytes` (the download of `name`) against SHA256SUMS.
pub fn verify(name: &str, bytes: &[u8], sums: &HashMap<String, String>) -> Result<(), String> {
    let expected = sums.get(name).ok_or_else(|| format!("{SUMS_NAME} has no entry for {name}"))?;
    let actual = sha256_hex(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(format!("the download of {name} is damaged (SHA-256 {actual}, expected {expected})"))
    }
}

// ---------------------------------------------------------------------------
// Install kinds
// ---------------------------------------------------------------------------

/// How this copy was installed, and so how it can update itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// The Windows installer's directory (has `unins000.exe`).
    WindowsInstaller { dir: PathBuf },
    /// A writable `chummer-rs.app`.
    MacApp { app: PathBuf },
    /// `install.sh`: `<prefix>/bin/chummer-rs` + `<prefix>/share/chummer-rs`.
    LinuxPrefix { prefix: PathBuf },
    /// An unpacked Linux release archive (programs + `resources/`).
    LinuxPortable { dir: PathBuf },
    /// Cannot update itself; show the release page.
    Manual(ManualReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualReason {
    /// Running from a cargo `target` directory.
    Development,
    /// A system or package-manager install (`/usr`, `/opt`, Flatpak...).
    SystemPackage,
    /// The install location is not writable.
    ReadOnly,
    /// The Windows zip, or another layout the updater does not know.
    Unknown,
    /// No package for this platform.
    NoPackage,
}

impl ManualReason {
    pub fn explain(self) -> &'static str {
        match self {
            ManualReason::Development => "This chummer-rs runs from a build directory; update the source and rebuild.",
            ManualReason::SystemPackage => "This chummer-rs was installed by a package manager; update it there.",
            ManualReason::ReadOnly => "chummer-rs cannot write to the folder it is installed in; download the new version from the release page.",
            ManualReason::Unknown => "This copy of chummer-rs was not set up by its installer; download the new version from the release page.",
            ManualReason::NoPackage => "The release has no package for this computer; see the release page.",
        }
    }
}

/// Decide the install kind of the executable at `exe`. `exists` and
/// `writable` probe the file system (replaced in tests).
pub fn detect_install(exe: &Path, os: Os, exists: &dyn Fn(&Path) -> bool, writable: &dyn Fn(&Path) -> bool) -> InstallKind {
    let Some(dir) = exe.parent() else { return InstallKind::Manual(ManualReason::Unknown) };
    let names: Vec<String> = exe.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if let Some(t) = names.iter().position(|n| n == "target") {
        if names[t + 1..].iter().any(|n| n == "debug" || n == "release") {
            return InstallKind::Manual(ManualReason::Development);
        }
    }
    match os {
        Os::Windows => {
            if exists(&dir.join("unins000.exe")) {
                InstallKind::WindowsInstaller { dir: dir.to_owned() }
            } else {
                InstallKind::Manual(ManualReason::Unknown)
            }
        }
        Os::Mac => {
            let app = dir.parent().filter(|c| dir.ends_with("MacOS") && c.ends_with("Contents")).and_then(Path::parent);
            match app {
                Some(app) if app.extension().is_some_and(|e| e == "app") => match app.parent() {
                    Some(parent) if writable(parent) && writable(app) => InstallKind::MacApp { app: app.to_owned() },
                    _ => InstallKind::Manual(ManualReason::ReadOnly),
                },
                _ => InstallKind::Manual(ManualReason::Unknown),
            }
        }
        Os::Linux | Os::Other => {
            const SYSTEM: [&str; 6] = ["/usr", "/opt", "/nix", "/snap", "/app", "/gnu"];
            let system = SYSTEM.iter().any(|s| exe.starts_with(s));
            if let Some(prefix) = dir.parent().filter(|_| dir.ends_with("bin")) {
                let share = prefix.join("share/chummer-rs");
                if exists(&share.join("data")) {
                    return if system {
                        InstallKind::Manual(ManualReason::SystemPackage)
                    } else if writable(dir) && writable(&share) && writable(&prefix.join("share")) {
                        InstallKind::LinuxPrefix { prefix: prefix.to_owned() }
                    } else {
                        InstallKind::Manual(ManualReason::ReadOnly)
                    };
                }
            }
            if exists(&dir.join("resources/data")) {
                return if system {
                    InstallKind::Manual(ManualReason::SystemPackage)
                } else if writable(dir) {
                    InstallKind::LinuxPortable { dir: dir.to_owned() }
                } else {
                    InstallKind::Manual(ManualReason::ReadOnly)
                };
            }
            InstallKind::Manual(if system { ManualReason::SystemPackage } else { ManualReason::Unknown })
        }
    }
}

/// Whether a file can be created in `dir`.
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".chummer-rs-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    std::fs::remove_file(&probe).ok();
    ok
}

/// The install kind of the running program.
pub fn this_install() -> InstallKind {
    match std::env::current_exe() {
        Ok(exe) => detect_install(&exe, Os::current(), &|p| p.exists(), &dir_writable),
        Err(_) => InstallKind::Manual(ManualReason::Unknown),
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

fn api_url() -> String {
    std::env::var("CHUMMER_UPDATE_URL").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

fn client() -> Result<reqwest::Client, String> {
    // reqwest is built without a crypto provider of its own (iroh's choice);
    // use ring, which rustls already has.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("chummer-rs/", env!("CARGO_PKG_VERSION"), " (+https://github.com/shamblashini/chummer-rs)"))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch_latest(client: &reqwest::Client) -> Result<Release, String> {
    let resp = client.get(api_url()).header("Accept", "application/vnd.github+json").send().await.map_err(|e| e.to_string())?;
    let resp = resp.error_for_status().map_err(|e| e.to_string())?;
    parse_release(&resp.text().await.map_err(|e| e.to_string())?)
}

async fn download(client: &reqwest::Client, url: &str, progress: &AtomicU64) -> Result<Vec<u8>, String> {
    let mut resp = client.get(url).send().await.and_then(reqwest::Response::error_for_status).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(resp.content_length().unwrap_or(0) as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        out.extend_from_slice(&chunk);
        progress.store(out.len() as u64, Ordering::Relaxed);
    }
    Ok(out)
}

fn block_on<T>(f: impl std::future::Future<Output = T>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    Ok(rt.block_on(f))
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

/// What to do once the new version is in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// The installer runs; quit now so it can replace the files.
    QuitForInstaller,
    /// Installed; start this program to run the new version.
    Restart(PathBuf),
}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> String {
    format!("{what} {}: {e}", p.display())
}

/// Download, verify and install `asset` from `release`.
fn download_and_install(release: &Release, asset: &Asset, kind: &InstallKind, progress: &AtomicU64) -> Result<Done, String> {
    let sums_asset = release.assets.iter().find(|a| a.name == SUMS_NAME).ok_or_else(|| format!("the release has no {SUMS_NAME}, so the download cannot be checked; download it from the release page"))?;
    let (sums, bytes) = block_on(async {
        let client = client()?;
        let sums = download(&client, &sums_asset.url, &AtomicU64::new(0)).await?;
        let bytes = download(&client, &asset.url, progress).await?;
        Ok::<_, String>((sums, bytes))
    })??;
    verify(&asset.name, &bytes, &parse_sums(&String::from_utf8_lossy(&sums)))?;
    install(kind, &asset.name, &bytes)
}

fn install(kind: &InstallKind, name: &str, bytes: &[u8]) -> Result<Done, String> {
    match kind {
        InstallKind::WindowsInstaller { dir } => run_installer(dir, name, bytes),
        InstallKind::MacApp { app } => replace_app(app, name, bytes),
        InstallKind::LinuxPrefix { prefix } => replace_unix(&prefix.join("bin"), &prefix.join("share/chummer-rs"), name, bytes),
        InstallKind::LinuxPortable { dir } => replace_unix(dir, &dir.join("resources"), name, bytes),
        InstallKind::Manual(r) => Err(r.explain().to_owned()),
    }
}

/// A fresh directory in `parent` for unpacking, removed when dropped.
struct Stage(PathBuf);

impl Stage {
    fn new(parent: &Path) -> Result<Stage, String> {
        let p = parent.join(format!(".chummer-rs-update-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).map_err(|e| io_err("cannot create", &p, e))?;
        Ok(Stage(p))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// The one directory an archive unpacked into `stage`.
fn top_dir(stage: &Path, skip: &str) -> Result<PathBuf, String> {
    let dirs: Vec<PathBuf> = std::fs::read_dir(stage)
        .map_err(|e| io_err("cannot read", stage, e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n == skip))
        .collect();
    match dirs.as_slice() {
        [d] => Ok(d.clone()),
        _ => Err("the downloaded package has an unexpected layout".into()),
    }
}

fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Windows: start the new installer silently on the existing install.
fn run_installer(dir: &Path, name: &str, bytes: &[u8]) -> Result<Done, String> {
    let tmp = std::env::temp_dir().join("chummer-rs-update");
    std::fs::create_dir_all(&tmp).map_err(|e| io_err("cannot create", &tmp, e))?;
    let setup = tmp.join(name);
    std::fs::write(&setup, bytes).map_err(|e| io_err("cannot write", &setup, e))?;
    // A per-user install lives under %LOCALAPPDATA%; anything else was
    // installed for all users (the installer then asks for administrator
    // rights).
    let per_user = std::env::var_os("LOCALAPPDATA").is_some_and(|l| dir.starts_with(l));
    let mut cmd = std::process::Command::new(&setup);
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/RELAUNCH", if per_user { "/CURRENTUSER" } else { "/ALLUSERS" }])
        .arg(format!("/DIR={}", dir.display()))
        .arg(format!("/LOG={}", tmp.join("install.log").display()));
    cmd.spawn().map_err(|e| io_err("cannot start", &setup, e))?;
    Ok(Done::QuitForInstaller)
}

/// macOS: unpack the release zip next to the running app and swap the
/// bundles.
fn replace_app(app: &Path, name: &str, bytes: &[u8]) -> Result<Done, String> {
    let parent = app.parent().ok_or("no folder above the app")?;
    let stage = Stage::new(parent)?;
    let zip = stage.0.join(name);
    std::fs::write(&zip, bytes).map_err(|e| io_err("cannot write", &zip, e))?;
    let unpacked = stage.0.join("unpacked");
    run(std::process::Command::new("ditto").arg("-x").arg("-k").arg(&zip).arg(&unpacked))?;
    let new_app = top_dir(&unpacked, "")?.join("chummer-rs.app");
    if !new_app.join("Contents/MacOS/chummer-rs").is_file() {
        return Err("the downloaded package has no chummer-rs.app".into());
    }
    // Unsigned: a quarantined app would not start after the swap.
    let _ = run(std::process::Command::new("xattr").arg("-dr").arg("com.apple.quarantine").arg(&new_app));
    let old = stage.0.join("old.app");
    std::fs::rename(app, &old).map_err(|e| io_err("cannot move", app, e))?;
    if let Err(e) = std::fs::rename(&new_app, app) {
        std::fs::rename(&old, app).ok();
        return Err(io_err("cannot replace", app, e));
    }
    Ok(Done::Restart(app.to_owned()))
}

/// Programs the Linux package carries; each replaces one already there
/// (chummer-rs always).
const PROGRAMS: [&str; 4] = ["chummer-rs", "chummer-cli", "chummer-authority", "chummer-relay"];

/// Linux: unpack the release archive, then replace the programs in
/// `bin_dir` and the resource directory `res_dir`. Each replacement is a
/// rename, so a running program keeps its old file and nothing is left
/// half-written.
fn replace_unix(bin_dir: &Path, res_dir: &Path, name: &str, bytes: &[u8]) -> Result<Done, String> {
    let parent = res_dir.parent().ok_or("no folder above the resources")?;
    let stage = Stage::new(parent)?;
    let archive = stage.0.join(name);
    std::fs::write(&archive, bytes).map_err(|e| io_err("cannot write", &archive, e))?;
    run(std::process::Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&stage.0))?;
    let top = top_dir(&stage.0, "")?;
    let new_res = top.join("resources");
    if !new_res.join("data").is_dir() || !top.join("chummer-rs").is_file() {
        return Err("the downloaded package has an unexpected layout".into());
    }
    // Copy the programs next to the old ones first (same file system, so
    // the final step is only renames).
    let mut programs = Vec::new();
    for p in PROGRAMS {
        let (src, dst) = (top.join(p), bin_dir.join(p));
        if src.is_file() && (p == "chummer-rs" || dst.exists()) {
            let tmp = bin_dir.join(format!(".{p}.update"));
            std::fs::copy(&src, &tmp).map_err(|e| io_err("cannot write", &tmp, e))?;
            set_executable(&tmp)?;
            programs.push((tmp, dst));
        }
    }
    // Resources: same-file-system rename of the unpacked copy.
    let old_res = stage.0.join("old-resources");
    std::fs::rename(res_dir, &old_res).map_err(|e| io_err("cannot move", res_dir, e))?;
    if let Err(e) = std::fs::rename(&new_res, res_dir) {
        std::fs::rename(&old_res, res_dir).ok();
        for (tmp, _) in &programs {
            std::fs::remove_file(tmp).ok();
        }
        return Err(io_err("cannot replace", res_dir, e));
    }
    for (tmp, dst) in &programs {
        std::fs::rename(tmp, dst).map_err(|e| io_err("cannot replace", dst, e))?;
    }
    Ok(Done::Restart(bin_dir.join("chummer-rs")))
}

#[cfg(unix)]
fn set_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).map_err(|e| io_err("cannot set permissions on", p, e))
}

#[cfg(not(unix))]
fn set_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

/// Start the newly installed version.
fn restart(target: &Path) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-n").arg(target);
        c
    } else {
        std::process::Command::new(target)
    };
    cmd.stdin(std::process::Stdio::null()).spawn().map(|_| ())
}

// ---------------------------------------------------------------------------
// GUI
// ---------------------------------------------------------------------------

/// "Check for updates on start" (gui.ini `check_updates`, default on).
pub fn check_on_start() -> bool {
    crate::theme::load_value(SETTING_KEY).is_none_or(|v| !v.eq_ignore_ascii_case("false"))
}

enum Msg {
    Checked { manual: bool, result: Result<Release, String> },
    Installed(Result<Done, String>),
}

enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { release: Release, size: u64, progress: Arc<AtomicU64> },
    Installed { release: Release, done: Done },
    Failed { error: String, page: Option<String> },
}

/// The update check and the card it shows.
pub struct Updater {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    state: State,
    /// The card is hidden ("Later").
    dismissed: bool,
    show_notes: bool,
    /// "Update now" with unsaved changes: asking first.
    confirm_unsaved: bool,
}

impl Updater {
    /// A new updater; checks for an update in the background unless the
    /// user turned it off (or `CHUMMER_NO_UPDATE_CHECK` is set).
    pub fn start(ctx: &egui::Context) -> Updater {
        let (tx, rx) = mpsc::channel();
        let mut u = Updater { tx, rx, state: State::Idle, dismissed: false, show_notes: false, confirm_unsaved: false };
        if !cfg!(test) && std::env::var_os("CHUMMER_NO_UPDATE_CHECK").is_none() && check_on_start() {
            u.check(ctx, false);
        }
        u
    }

    fn check(&mut self, ctx: &egui::Context, manual: bool) {
        if matches!(self.state, State::Checking | State::Downloading { .. }) {
            return;
        }
        self.state = State::Checking;
        self.dismissed = !manual;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = block_on(async { fetch_latest(&client()?).await }).and_then(|r| r);
            tx.send(Msg::Checked { manual, result }).ok();
            ctx.request_repaint();
        });
    }

    fn start_update(&mut self, ctx: &egui::Context, release: Release) {
        let kind = this_install();
        let Some(asset) = select_asset(&release.assets, Os::current(), std::env::consts::ARCH).cloned() else {
            self.state = State::Failed { error: ManualReason::NoPackage.explain().into(), page: Some(release.page) };
            return;
        };
        if let InstallKind::Manual(r) = kind {
            self.state = State::Failed { error: r.explain().into(), page: Some(release.page) };
            return;
        }
        let progress = Arc::new(AtomicU64::new(0));
        self.state = State::Downloading { release: release.clone(), size: asset.size, progress: progress.clone() };
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = download_and_install(&release, &asset, &kind, &progress);
            tx.send(Msg::Installed(result)).ok();
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Checked { manual, result } => {
                    self.state = match result {
                        Ok(r) if r.version > current_version() => {
                            self.dismissed = false;
                            State::Available(r)
                        }
                        Ok(_) => State::UpToDate,
                        Err(e) if manual => State::Failed { error: e, page: None },
                        // A failed check at start stays quiet.
                        Err(_) => State::Idle,
                    };
                }
                Msg::Installed(result) => {
                    let release = match std::mem::replace(&mut self.state, State::Idle) {
                        State::Downloading { release, .. } => release,
                        _ => continue,
                    };
                    self.dismissed = false;
                    self.state = match result {
                        Ok(done) => State::Installed { release, done },
                        Err(error) => State::Failed { error, page: Some(release.page) },
                    };
                }
            }
        }
    }

    /// Help menu entries: "Check for Updates" and the start-up toggle.
    pub fn menu(&mut self, ui: &mut egui::Ui, lang: &Language) {
        if ui.button(lang.tr("Check for Updates")).clicked() {
            ui.close();
            self.check(ui.ctx(), true);
        }
        let mut on = check_on_start();
        if ui.checkbox(&mut on, lang.tr("Check for updates on start")).clicked() {
            crate::theme::save_value(SETTING_KEY, if on { "true" } else { "false" }).ok();
        }
    }

    /// Draw the update card and its dialogs. `dirty`: there are unsaved
    /// changes. Returns true when the app should quit now (the Windows
    /// installer is running).
    pub fn ui(&mut self, ctx: &egui::Context, lang: &Language, dirty: bool) -> bool {
        self.poll();
        let mut quit = false;
        if let State::Downloading { .. } = self.state {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if let State::Installed { done: Done::QuitForInstaller, .. } = self.state {
            return true;
        }
        let shown = !self.dismissed && !matches!(self.state, State::Idle);
        if shown {
            let mut action = None;
            egui::Window::new("chummer-rs update")
                .id(egui::Id::new("update_card"))
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -36.0])
                .default_width(320.0)
                .show(ctx, |ui| action = self.card(ui, lang));
            match action {
                Some(Action::Later) => self.dismissed = true,
                Some(Action::Notes) => self.show_notes = true,
                Some(Action::Update) if dirty => self.confirm_unsaved = true,
                Some(Action::Update) => self.update_now(ctx),
                Some(Action::Restart(p)) => {
                    match restart(&p) {
                        Ok(()) => quit = true,
                        Err(e) => self.state = State::Failed { error: io_err("cannot start", &p, e), page: None },
                    }
                }
                Some(Action::Page(url)) => {
                    crate::open::open(url).ok();
                }
                None => {}
            }
        }
        self.notes_window(ctx, lang);
        if self.confirm_unsaved {
            egui::Modal::new(egui::Id::new("update_unsaved")).show(ctx, |ui| {
                ui.heading(lang.tr("Unsaved Changes"));
                ui.label("Some characters have unsaved changes. Updating closes chummer-rs. Save them first, or update anyway and lose the changes?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Update anyway").clicked() {
                        self.confirm_unsaved = false;
                        self.update_now(ctx);
                    }
                    if ui.button(lang.tr("Cancel")).clicked() {
                        self.confirm_unsaved = false;
                    }
                });
            });
        }
        quit
    }

    fn update_now(&mut self, ctx: &egui::Context) {
        if let State::Available(r) = &self.state {
            let r = r.clone();
            self.start_update(ctx, r);
        }
    }

    fn card(&self, ui: &mut egui::Ui, lang: &Language) -> Option<Action> {
        let mut action = None;
        match &self.state {
            State::Idle => {}
            State::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(lang.tr("Checking for Updates..."));
                });
            }
            State::UpToDate => {
                ui.label(lang.tr("No new updates are available at this time."));
                ui.weak(format!("chummer-rs {}", current_version()));
                if ui.button(lang.tr("OK")).clicked() {
                    action = Some(Action::Later);
                }
            }
            State::Available(r) => {
                ui.strong(format!("chummer-rs {} is available", r.version));
                ui.weak(format!("You have {}.", current_version()));
                let kind = this_install();
                if let InstallKind::Manual(reason) = kind {
                    ui.label(reason.explain());
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("What's new").clicked() {
                        action = Some(Action::Notes);
                    }
                    if matches!(kind, InstallKind::Manual(_)) || select_asset(&r.assets, Os::current(), std::env::consts::ARCH).is_none() {
                        if ui.button(strong("Open release page")).clicked() {
                            action = Some(Action::Page(r.page.clone()));
                        }
                    } else if ui.button(strong("Update now")).clicked() {
                        action = Some(Action::Update);
                    }
                    if ui.button("Later").clicked() {
                        action = Some(Action::Later);
                    }
                });
            }
            State::Downloading { release, size, progress } => {
                let got = progress.load(Ordering::Relaxed);
                if *size > 0 && got >= *size {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Installing chummer-rs {}…", release.version));
                    });
                } else {
                    ui.label(format!("Downloading chummer-rs {}…", release.version));
                    let frac = if *size > 0 { got as f32 / *size as f32 } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac.min(1.0)).show_percentage());
                }
            }
            State::Installed { release, done } => {
                ui.strong(format!("chummer-rs {} is installed", release.version));
                if let Done::Restart(p) = done {
                    ui.label("Restart chummer-rs to use it.");
                    ui.horizontal(|ui| {
                        if ui.button(strong("Restart now")).clicked() {
                            action = Some(Action::Restart(p.clone()));
                        }
                        if ui.button("Later").clicked() {
                            action = Some(Action::Later);
                        }
                    });
                }
            }
            State::Failed { error, page } => {
                ui.colored_label(ui.visuals().error_fg_color, "The update failed");
                ui.label(error);
                ui.horizontal(|ui| {
                    if let Some(p) = page {
                        if ui.button("Open release page").clicked() {
                            action = Some(Action::Page(p.clone()));
                        }
                    }
                    if ui.button(lang.tr("OK")).clicked() {
                        action = Some(Action::Later);
                    }
                });
            }
        }
        action
    }

    fn notes_window(&mut self, ctx: &egui::Context, lang: &Language) {
        if !self.show_notes {
            return;
        }
        let release = match &self.state {
            State::Available(r) | State::Downloading { release: r, .. } | State::Installed { release: r, .. } => r.clone(),
            _ => {
                self.show_notes = false;
                return;
            }
        };
        let mut open = true;
        egui::Window::new(format!("What's new in chummer-rs {}", release.version)).id(egui::Id::new("update_notes")).open(&mut open).default_size([560.0, 420.0]).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                if release.notes.trim().is_empty() {
                    ui.label("No release notes.");
                }
                // GitHub's Markdown, lightly: headings bold, list bullets.
                for line in release.notes.lines() {
                    let t = line.trim_end();
                    if let Some(h) = t.strip_prefix('#') {
                        ui.add_space(4.0);
                        ui.strong(h.trim_start_matches('#').trim());
                    } else if let Some(item) = t.trim_start().strip_prefix("- ").or_else(|| t.trim_start().strip_prefix("* ")) {
                        ui.label(format!("{}• {}", " ".repeat(t.len() - t.trim_start().len()), item.replace("**", "")));
                    } else if !t.is_empty() {
                        ui.label(t.replace("**", ""));
                    }
                }
            });
            ui.separator();
            if ui.link(lang.tr("Open release page")).clicked() {
                crate::open::open(&release.page).ok();
            }
        });
        self.show_notes = open;
    }
}

enum Action {
    Later,
    Notes,
    Update,
    Restart(PathBuf),
    Page(String),
}

/// Bold button text.
fn strong(s: &str) -> egui::RichText {
    egui::RichText::new(s).strong()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_compare() {
        assert!(v("0.5.0") > v("0.4.0"));
        assert!(v("v0.4.1") > v("0.4.0"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.10.0") > v("0.9.0"));
        assert_eq!(v("v0.4.0"), v("0.4.0"));
        assert_eq!(v("0.4"), v("0.4.0"));
        assert_eq!(v("0.4.0+build.7"), v("0.4.0"));
        assert!(v("0.5.0-beta.1") < v("0.5.0"));
        assert!(v("0.5.0-beta.1") > v("0.4.9"));
        assert!(v("0.5.0-beta.2") < v("0.5.0-beta.10"));
        assert!(v("0.5.0-alpha") < v("0.5.0-beta"));
        assert!(v("0.5.0-alpha") < v("0.5.0-alpha.1"));
        assert!(Version::parse("latest").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("").is_none());
        assert_eq!(v("v1.2.3-rc.1").to_string(), "1.2.3-rc.1");
        assert!(current_version() >= v("0.4.0"));
    }

    const RELEASE: &str = r#"{
        "tag_name": "v0.5.0",
        "html_url": "https://github.com/shamblashini/chummer-rs/releases/tag/v0.5.0",
        "body": "Installers!",
        "assets": [
            {"name": "SHA256SUMS", "size": 400, "browser_download_url": "https://x/SHA256SUMS"},
            {"name": "chummer-rs-v0.5.0-linux-x86_64.tar.gz", "size": 1, "browser_download_url": "https://x/l"},
            {"name": "chummer-rs-v0.5.0-macos-universal.dmg", "size": 2, "browser_download_url": "https://x/md"},
            {"name": "chummer-rs-v0.5.0-macos-universal.zip", "size": 3, "browser_download_url": "https://x/mz"},
            {"name": "chummer-rs-v0.5.0-windows-x86_64.zip", "size": 4, "browser_download_url": "https://x/wz"},
            {"name": "chummer-rs-v0.5.0-windows-x86_64-setup.exe", "size": 5, "browser_download_url": "https://x/ws"},
            {"name": "docker-compose.yml", "size": 6, "browser_download_url": "https://x/dc"}
        ]
    }"#;

    #[test]
    fn release_json_and_asset_choice() {
        let r = parse_release(RELEASE).unwrap();
        assert_eq!(r.version, v("0.5.0"));
        assert_eq!(r.notes, "Installers!");
        assert_eq!(r.assets.len(), 7);
        let pick = |os, arch| select_asset(&r.assets, os, arch).map(|a| a.url.as_str());
        assert_eq!(pick(Os::Windows, "x86_64"), Some("https://x/ws"));
        assert_eq!(pick(Os::Mac, "aarch64"), Some("https://x/mz"));
        assert_eq!(pick(Os::Mac, "x86_64"), Some("https://x/mz"));
        assert_eq!(pick(Os::Linux, "x86_64"), Some("https://x/l"));
        assert_eq!(pick(Os::Linux, "aarch64"), None);
        assert_eq!(pick(Os::Windows, "aarch64"), None);
        assert_eq!(pick(Os::Other, "x86_64"), None);
        assert!(parse_release("{}").is_err());
        assert!(parse_release(r#"{"tag_name": "nightly"}"#).is_err());
    }

    #[test]
    fn checksums() {
        let data = b"chummer";
        let hash = sha256_hex(data);
        assert_eq!(hash.len(), 64);
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let sums = parse_sums(&format!("{hash}  chummer-rs-v0.5.0-linux-x86_64.tar.gz\n{}  *other.zip\nnot a line\n", "A".repeat(64)));
        assert_eq!(sums.len(), 2);
        assert_eq!(sums["other.zip"], "a".repeat(64));
        assert!(verify("chummer-rs-v0.5.0-linux-x86_64.tar.gz", data, &sums).is_ok());
        assert!(verify("chummer-rs-v0.5.0-linux-x86_64.tar.gz", b"chummex", &sums).unwrap_err().contains("damaged"));
        assert!(verify("missing.zip", data, &sums).unwrap_err().contains("no entry"));
    }

    #[test]
    fn install_kinds() {
        let all = |_: &Path| true;
        let none = |_: &Path| false;
        let det = |exe: &str, os, exists: &dyn Fn(&Path) -> bool, writable: &dyn Fn(&Path) -> bool| detect_install(Path::new(exe), os, exists, writable);
        // cargo build directories
        assert_eq!(det("/home/u/src/chummer-rs/target/release/chummer-rs", Os::Linux, &all, &all), InstallKind::Manual(ManualReason::Development));
        assert_eq!(det("/w/target/x86_64-pc-windows-msvc/debug/chummer-rs.exe", Os::Windows, &all, &all), InstallKind::Manual(ManualReason::Development));
        // install.sh in ~/.local
        let local = |p: &Path| p.ends_with("share/chummer-rs/data");
        assert_eq!(det("/home/u/.local/bin/chummer-rs", Os::Linux, &local, &all), InstallKind::LinuxPrefix { prefix: "/home/u/.local".into() });
        assert_eq!(det("/home/u/.local/bin/chummer-rs", Os::Linux, &local, &none), InstallKind::Manual(ManualReason::ReadOnly));
        // system installs
        assert_eq!(det("/usr/bin/chummer-rs", Os::Linux, &local, &all), InstallKind::Manual(ManualReason::SystemPackage));
        assert_eq!(det("/usr/local/bin/chummer-rs", Os::Linux, &local, &all), InstallKind::Manual(ManualReason::SystemPackage));
        assert_eq!(det("/opt/chummer-rs/chummer-rs", Os::Linux, &|p: &Path| p.ends_with("resources/data"), &all), InstallKind::Manual(ManualReason::SystemPackage));
        // an unpacked release archive
        let portable = |p: &Path| p.ends_with("resources/data");
        assert_eq!(det("/home/u/apps/chummer-rs-v0.4.0-linux-x86_64/chummer-rs", Os::Linux, &portable, &all), InstallKind::LinuxPortable { dir: "/home/u/apps/chummer-rs-v0.4.0-linux-x86_64".into() });
        assert_eq!(det("/home/u/apps/x/chummer-rs", Os::Linux, &portable, &none), InstallKind::Manual(ManualReason::ReadOnly));
        assert_eq!(det("/home/u/bin/chummer-rs", Os::Linux, &none, &all), InstallKind::Manual(ManualReason::Unknown));
        // Windows: installer vs zip
        let inno = |p: &Path| p.ends_with("unins000.exe");
        assert_eq!(det("C:/Users/u/AppData/Local/Programs/chummer-rs/chummer-rs.exe", Os::Windows, &inno, &all), InstallKind::WindowsInstaller { dir: "C:/Users/u/AppData/Local/Programs/chummer-rs".into() });
        assert_eq!(det("C:/Users/u/Downloads/chummer-rs/chummer-rs.exe", Os::Windows, &none, &all), InstallKind::Manual(ManualReason::Unknown));
        // macOS bundles
        assert_eq!(det("/Applications/chummer-rs.app/Contents/MacOS/chummer-rs", Os::Mac, &all, &all), InstallKind::MacApp { app: "/Applications/chummer-rs.app".into() });
        assert_eq!(det("/Applications/chummer-rs.app/Contents/MacOS/chummer-rs", Os::Mac, &all, &|p: &Path| p != Path::new("/Applications")), InstallKind::Manual(ManualReason::ReadOnly));
        assert_eq!(det("/Users/u/bin/chummer-rs", Os::Mac, &all, &all), InstallKind::Manual(ManualReason::Unknown));
    }

    /// The Linux replacement on a fake install.sh prefix, from a package
    /// laid out like the release workflow's.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_replace_swaps_programs_and_resources() {
        let root = std::env::temp_dir().join(format!("chummer-rs-update-test-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let prefix = root.join("prefix");
        let (bin, share) = (prefix.join("bin"), prefix.join("share/chummer-rs"));
        std::fs::create_dir_all(share.join("data")).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("chummer-rs"), "old").unwrap();
        std::fs::write(bin.join("chummer-cli"), "old").unwrap();
        std::fs::write(share.join("data/old.xml"), "old").unwrap();
        // The package.
        let pkg = root.join("pkg/chummer-rs-v9.0.0-linux-x86_64");
        std::fs::create_dir_all(pkg.join("resources/data")).unwrap();
        for p in ["chummer-rs", "chummer-cli", "chummer-relay"] {
            std::fs::write(pkg.join(p), "new").unwrap();
        }
        std::fs::write(pkg.join("resources/data/new.xml"), "new").unwrap();
        let tgz = root.join("p.tar.gz");
        let ok = std::process::Command::new("tar").arg("-czf").arg(&tgz).arg("-C").arg(root.join("pkg")).arg("chummer-rs-v9.0.0-linux-x86_64").status().unwrap();
        assert!(ok.success());
        let bytes = std::fs::read(&tgz).unwrap();
        let kind = detect_install(&bin.join("chummer-rs"), Os::Linux, &|p| p.exists(), &dir_writable);
        assert_eq!(kind, InstallKind::LinuxPrefix { prefix: prefix.clone() });
        let done = install(&kind, "chummer-rs-v9.0.0-linux-x86_64.tar.gz", &bytes).unwrap();
        assert_eq!(done, Done::Restart(bin.join("chummer-rs")));
        assert_eq!(std::fs::read_to_string(bin.join("chummer-rs")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(bin.join("chummer-cli")).unwrap(), "new");
        // Not installed before, so not added.
        assert!(!bin.join("chummer-relay").exists());
        assert!(share.join("data/new.xml").is_file());
        assert!(!share.join("data/old.xml").exists());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(bin.join("chummer-rs")).unwrap().permissions().mode() & 0o777, 0o755);
        // No leftovers.
        let left: Vec<_> = std::fs::read_dir(prefix.join("share")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("chummer-rs")]);
        let hidden: Vec<_> = std::fs::read_dir(&bin).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
        assert!(hidden.is_empty());
        // A damaged package changes nothing.
        assert!(install(&kind, "x.tar.gz", b"not a tarball").is_err());
        assert!(share.join("data/new.xml").is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    /// The real GitHub API (network): `cargo test -p chummer-gui live_latest -- --ignored`.
    #[test]
    #[ignore]
    fn live_latest_release() {
        let r = block_on(async { fetch_latest(&client()?).await }).unwrap().unwrap();
        assert!(r.version >= v("0.4.0"), "{r:?}");
        assert!(select_asset(&r.assets, Os::Linux, "x86_64").is_some(), "{r:?}");
    }
}
