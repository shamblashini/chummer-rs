//! Where chummer-rs keeps the user's own files.
//!
//! The game data, the bundled character sheets and the bundled custom
//! data are read-only resources inside the installation
//! ([`crate::data::resource_dir`]). Everything the user adds or the app
//! writes lives in three per-user roots, in the place each OS expects:
//!
//! | Root | Linux / BSD | macOS | Windows |
//! |---|---|---|---|
//! | data ([`data_root`]) | `~/.local/share/chummer-rs` | `~/Library/Application Support/chummer-rs` | `%APPDATA%\chummer-rs` |
//! | config ([`config_root`]) | `~/.config/chummer-rs` | `~/Library/Application Support/chummer-rs` | `%APPDATA%\chummer-rs` |
//! | state ([`state_root`]) | `~/.local/share/chummer-rs` | `~/Library/Application Support/chummer-rs` | `%LOCALAPPDATA%\chummer-rs` |
//!
//! `XDG_DATA_HOME` (data and state) and `XDG_CONFIG_HOME` (config) are
//! honoured on every OS when they are set to an absolute path, so tests
//! and portable setups can redirect them.
//!
//! The folders in them are the [`UserDir`]s; [`init`] creates them with a
//! `README.txt` each and moves files from the locations older versions
//! used ([`migrate`]). Files directly in the roots: `gui.ini`,
//! `guide.ini`, `sourcebooks.xml`, `online.json`, `node.key` and
//! `campaigns/` (config); `app.ron`, the window state (data).
//!
//! Nothing here is cached: tests change the variables at run time.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The folder name inside each root.
pub const APP: &str = "chummer-rs";

/// Written into a migrated old folder; its presence means "done".
pub const MOVED_NOTE: &str = "MOVED.txt";

/// The OS's base folders (before [`APP`] is added).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bases {
    pub data: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub local: Option<PathBuf>,
}

/// The three roots (with [`APP`] added).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub data: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
}

/// The base folders from the `dirs` crate: Application Support on macOS,
/// Roaming/Local AppData on Windows, XDG on Linux.
pub fn platform_bases() -> Bases {
    Bases { data: dirs::data_dir(), config: dirs::config_dir(), local: dirs::data_local_dir() }
}

/// An absolute, non-empty path from an environment variable.
fn env_path(name: &str) -> Option<PathBuf> {
    absolute(std::env::var_os(name))
}

fn absolute(v: Option<OsString>) -> Option<PathBuf> {
    v.filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute())
}

impl Roots {
    /// The roots from the XDG overrides and the OS bases.
    pub fn resolve(xdg_data: Option<PathBuf>, xdg_config: Option<PathBuf>, bases: &Bases) -> Option<Roots> {
        let data = xdg_data.clone().or_else(|| bases.data.clone())?.join(APP);
        let config = xdg_config.or_else(|| bases.config.clone()).map_or_else(|| data.clone(), |c| c.join(APP));
        let state = xdg_data.or_else(|| bases.local.clone()).map_or_else(|| data.clone(), |l| l.join(APP));
        Some(Roots { data, config, state })
    }

    /// The roots for this process now.
    pub fn current() -> Option<Roots> {
        Roots::resolve(env_path("XDG_DATA_HOME"), env_path("XDG_CONFIG_HOME"), &platform_bases())
    }

    pub fn dir(&self, d: UserDir) -> PathBuf {
        let root = match d.root() {
            Root::Data => &self.data,
            Root::Config => &self.config,
            Root::State => &self.state,
        };
        root.join(d.folder())
    }
}

/// The data root: custom data, sheets, kits, the window state.
pub fn data_root() -> Option<PathBuf> {
    Roots::current().map(|r| r.data)
}

/// The config root: rulesets and the app's settings files.
pub fn config_root() -> Option<PathBuf> {
    Roots::current().map(|r| r.config)
}

/// The state root: crash logs, recovery copies, backups.
pub fn state_root() -> Option<PathBuf> {
    Roots::current().map(|r| r.state)
}

/// A user folder's path now.
pub fn user_dir(d: UserDir) -> Option<PathBuf> {
    Roots::current().map(|r| r.dir(d))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Data,
    Config,
    State,
}

/// The folders the user may want to open, in the order they are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserDir {
    CustomData,
    Sheets,
    Kits,
    Settings,
    Backups,
    Crashes,
    Recovery,
}

impl UserDir {
    pub const ALL: [UserDir; 7] = [UserDir::CustomData, UserDir::Sheets, UserDir::Kits, UserDir::Settings, UserDir::Backups, UserDir::Crashes, UserDir::Recovery];

    pub fn folder(self) -> &'static str {
        match self {
            UserDir::CustomData => "customdata",
            UserDir::Sheets => "sheets",
            UserDir::Kits => "kits",
            UserDir::Settings => "settings",
            UserDir::Backups => "backups",
            UserDir::Crashes => "crashes",
            UserDir::Recovery => "recovery",
        }
    }

    pub fn root(self) -> Root {
        match self {
            UserDir::CustomData | UserDir::Sheets | UserDir::Kits => Root::Data,
            UserDir::Settings => Root::Config,
            UserDir::Backups | UserDir::Crashes | UserDir::Recovery => Root::State,
        }
    }

    /// English label (translate with `lang.tr`).
    pub fn label(self) -> &'static str {
        match self {
            UserDir::CustomData => "Custom data",
            UserDir::Sheets => "Character sheets",
            UserDir::Kits => "PACKS kits",
            UserDir::Settings => "Character settings",
            UserDir::Backups => "Backups",
            UserDir::Crashes => "Crash logs",
            UserDir::Recovery => "Recovery",
        }
    }

    /// The `README.txt` written into the folder; `None` for folders the
    /// app manages alone.
    pub fn readme(self) -> Option<&'static str> {
        Some(match self {
            UserDir::CustomData => {
                "Custom data for chummer-rs\n\n\
                 Put Chummer5a-style custom data folders here: one folder per set of\n\
                 changes, holding XML files named like the game data (amend_*.xml,\n\
                 override_*.xml, custom_*.xml) and an optional manifest.xml.\n\n\
                 The folders are found at start and listed with the bundled ones under\n\
                 Character Settings > Custom data. Turn a folder on there for each\n\
                 ruleset that should use it; its files are merged into the game data\n\
                 the same way Chummer5a does.\n"
            }
            UserDir::Sheets => {
                "Character sheets for chummer-rs\n\n\
                 Put your own XSLT character sheets (*.xsl) here. They are listed with\n\
                 the bundled sheets in the Print dialog. A sheet with the same file name\n\
                 as a bundled one (for example \"Shadowrun 5 (Core).xsl\") replaces it.\n\n\
                 English sheets go directly in this folder; sheets for another language\n\
                 go in a subfolder named after it (de-de, fr-fr, ja-jp, pt-br, zh-cn),\n\
                 as in the bundled sheets folder.\n\n\
                 A sheet may import the bundled helper files (\"Shadowrun 5 set.xslt\",\n\
                 \"xs.Chummer5CSS.xslt\" and so on) without copying them: an import not\n\
                 found next to your sheet is taken from the bundled sheets.\n"
            }
            UserDir::Kits => {
                "PACKS kits for chummer-rs\n\n\
                 Kits made with Special > Create PACKS Kit... are saved here as\n\
                 custom_*_packs.xml files. Kit files from Chummer5a (a packs.xml-style\n\
                 file named custom_<name>_packs.xml) can be copied here too; they are\n\
                 listed under Special > Add PACKS Kit...\n"
            }
            UserDir::Settings => {
                "Character settings (rulesets) for chummer-rs\n\n\
                 Each *.xml file here is a character settings preset made or imported\n\
                 with Character Settings. Chummer5a settings files (from its settings\n\
                 folder) can be copied here and are listed at the next start.\n"
            }
            UserDir::Backups => {
                "Backups made by chummer-rs\n\n\
                 Saving over a file keeps its previous versions here, one folder per\n\
                 file. Restore them with File > Restore backup...; the number kept is\n\
                 set in Tools > Preferences.\n"
            }
            UserDir::Crashes => {
                "Crash logs from chummer-rs\n\n\
                 When chummer-rs closes unexpectedly it writes a crash-<time>-<pid>.log\n\
                 file here. Please attach the newest one to a bug report.\n"
            }
            UserDir::Recovery => return None,
        })
    }
}

const ROOT_README: &str = "chummer-rs user files\n\n\
    This folder holds your own files for chummer-rs. The program itself and its\n\
    game data never need editing; put your additions here instead:\n\n\
    customdata/  Chummer5a-style custom data folders\n\
    sheets/      your own XSLT character sheets\n\
    kits/        PACKS kits\n\n\
    resources/   (only after ./install.sh) the program's own data: replaced\n\
    on every install or update, do not edit it.\n\n\
    Each folder has a README.txt. Tools > Preferences > Folders lists every\n\
    folder chummer-rs uses (character settings, backups and crash logs may be\n\
    elsewhere on this system) with buttons to open them.\n";

/// Create the user folders and their READMEs (existing files are kept).
pub fn ensure_layout(roots: &Roots) -> io::Result<()> {
    std::fs::create_dir_all(&roots.data)?;
    write_new(&roots.data.join("README.txt"), ROOT_README)?;
    for d in UserDir::ALL {
        let dir = roots.dir(d);
        std::fs::create_dir_all(&dir)?;
        if let Some(text) = d.readme() {
            write_new(&dir.join("README.txt"), text)?;
        }
    }
    Ok(())
}

fn write_new(path: &Path, text: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    // Windows users open these with Notepad.
    let text = if cfg!(windows) { text.replace('\n', "\r\n") } else { text.to_owned() };
    std::fs::write(path, text)
}

/// At start: move files from older locations, then create the folders.
/// Returns what was moved (also printed on stderr and appended to
/// `migration.log` in the data root).
pub fn init() -> Vec<String> {
    let Some(roots) = Roots::current() else { return Vec::new() };
    let legacy = legacy_roots(&roots);
    let log = migrate(&roots, &legacy);
    if let Err(e) = ensure_layout(&roots) {
        eprintln!("chummer-rs: cannot create {}: {e}", roots.data.display());
    }
    record(&roots, &log);
    log
}

/// [`migrate`] alone, for the command-line tool.
pub fn migrate_now() -> Vec<String> {
    let Some(roots) = Roots::current() else { return Vec::new() };
    let log = migrate(&roots, &legacy_roots(&roots));
    record(&roots, &log);
    log
}

fn record(roots: &Roots, log: &[String]) {
    if log.is_empty() {
        return;
    }
    for line in log {
        eprintln!("chummer-rs: {line}");
    }
    let _ = std::fs::create_dir_all(&roots.data);
    let path = roots.data.join("migration.log");
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for line in log {
        text.push_str(line);
        text.push('\n');
    }
    let _ = std::fs::write(path, text);
}

// ----- migration -----

/// Folders older versions used, outside the current roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Legacy {
    /// Old data roots (`~/.local/share/chummer-rs`).
    pub data: Vec<PathBuf>,
    /// Old config roots (`~/.config/chummer-rs`).
    pub config: Vec<PathBuf>,
}

/// The old Linux-style roots on macOS and Windows: until 0.4 every OS used
/// `$HOME/.local/share/chummer-rs` and `$HOME/.config/chummer-rs`. On
/// Linux those are the current roots, so nothing moves.
pub fn legacy_roots(roots: &Roots) -> Legacy {
    if cfg!(any(target_os = "macos", windows)) {
        let homes = [absolute(std::env::var_os("HOME")), absolute(std::env::var_os("USERPROFILE")), dirs::home_dir()];
        legacy_for(roots, &homes.into_iter().flatten().collect::<Vec<_>>())
    } else {
        Legacy::default()
    }
}

/// The old roots under each home folder, without the current roots.
pub fn legacy_for(roots: &Roots, homes: &[PathBuf]) -> Legacy {
    let mut out = Legacy::default();
    for h in homes {
        let d = h.join(".local").join("share").join(APP);
        let c = h.join(".config").join(APP);
        if !out.data.contains(&d) && d != roots.data && d != roots.state {
            out.data.push(d);
        }
        if !out.config.contains(&c) && c != roots.config {
            out.config.push(c);
        }
    }
    out
}

/// Entries of an old data root that belong in the state root.
const STATE_ENTRIES: [&str; 4] = ["crashes", "recovery", "sessions", "backups"];

/// Move files from `legacy` into `roots`, and tidy the roots themselves
/// (`packs` → `kits`; on Windows the state folders → `%LOCALAPPDATA%` and
/// eframe's `data\app.ron` → `app.ron`). An entry already present at the
/// new place is left where it was. Each old root gets a [`MOVED_NOTE`]
/// and is skipped from then on. Returns a line per move.
pub fn migrate(roots: &Roots, legacy: &Legacy) -> Vec<String> {
    let mut log = Vec::new();
    for old in &legacy.data {
        migrate_root(old, &mut log, |name| data_target(roots, name));
    }
    for old in &legacy.config {
        migrate_root(old, &mut log, |name| Some(roots.config.join(name)));
    }
    // Inside the current roots.
    move_logged(&roots.data.join("packs"), &roots.data.join("kits"), &mut log);
    if roots.state != roots.data {
        for name in STATE_ENTRIES {
            move_logged(&roots.data.join(name), &roots.state.join(name), &mut log);
        }
    }
    let eframe_dir = roots.data.join("data");
    move_logged(&eframe_dir.join("app.ron"), &roots.data.join("app.ron"), &mut log);
    let _ = std::fs::remove_dir(&eframe_dir);
    log
}

fn data_target(roots: &Roots, name: &str) -> Option<PathBuf> {
    Some(match name {
        "packs" => roots.data.join("kits"),
        // eframe's window state: on Windows it was in `data\app.ron`.
        "data" => return None,
        n if STATE_ENTRIES.contains(&n) => roots.state.join(n),
        n => roots.data.join(n),
    })
}

fn migrate_root(old: &Path, log: &mut Vec<String>, target: impl Fn(&str) -> Option<PathBuf>) {
    if !old.is_dir() || old.join(MOVED_NOTE).exists() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(old) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    let mut moved_to: Vec<PathBuf> = Vec::new();
    for src in entries {
        let Some(name) = src.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        let Some(dst) = target(&name) else { continue };
        if move_logged(&src, &dst, log) {
            if let Some(p) = dst.parent() {
                if !moved_to.contains(&p.to_path_buf()) {
                    moved_to.push(p.to_owned());
                }
            }
        }
    }
    let dests: Vec<String> = moved_to.iter().map(|p| p.display().to_string()).collect();
    let note = format!(
        "chummer-rs now keeps these files in:\n{}\n\nThey were moved there. Anything still here was already present at the new place and was not moved.\n",
        if dests.is_empty() { "(nothing was moved)".to_owned() } else { dests.join("\n") }
    );
    if let Err(e) = std::fs::write(old.join(MOVED_NOTE), note) {
        log.push(format!("could not mark {} as moved: {e}", old.display()));
    }
}

/// Move `src` to `dst` when `dst` does not exist yet; a folder whose new
/// place exists is merged (its missing entries move). True if anything
/// moved.
fn move_logged(src: &Path, dst: &Path, log: &mut Vec<String>) -> bool {
    if std::fs::symlink_metadata(src).is_err() {
        return false;
    }
    if !dst.exists() {
        return match move_path(src, dst) {
            Ok(()) => {
                log.push(format!("moved {} to {}", src.display(), dst.display()));
                true
            }
            Err(e) => {
                log.push(format!("could not move {} to {}: {e}", src.display(), dst.display()));
                false
            }
        };
    }
    if !(src.is_dir() && dst.is_dir()) {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(src) else { return false };
    let mut any = false;
    for e in rd.flatten() {
        any |= move_logged(&e.path(), &dst.join(e.file_name()), log);
    }
    // Gone when every entry moved.
    let _ = std::fs::remove_dir(src);
    any
}

/// Rename, or copy and remove (another volume).
fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_all(src, dst)?;
    if src.is_dir() {
        std::fs::remove_dir_all(src)
    } else {
        std::fs::remove_file(src)
    }
}

fn copy_all(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_all(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chummer-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bases(data: &str, config: &str, local: &str) -> Bases {
        Bases { data: Some(data.into()), config: Some(config.into()), local: Some(local.into()) }
    }

    #[test]
    fn roots_per_os_shape() {
        // Linux: XDG data and config.
        let r = Roots::resolve(None, None, &bases("/home/u/.local/share", "/home/u/.config", "/home/u/.local/share")).unwrap();
        assert_eq!(r.data, PathBuf::from("/home/u/.local/share/chummer-rs"));
        assert_eq!(r.config, PathBuf::from("/home/u/.config/chummer-rs"));
        assert_eq!(r.state, r.data);
        assert_eq!(r.dir(UserDir::Settings), PathBuf::from("/home/u/.config/chummer-rs/settings"));
        assert_eq!(r.dir(UserDir::Kits), PathBuf::from("/home/u/.local/share/chummer-rs/kits"));
        // macOS: one folder in Application Support.
        let s = "/Users/u/Library/Application Support";
        let r = Roots::resolve(None, None, &bases(s, s, s)).unwrap();
        assert_eq!(r.data, PathBuf::from(s).join(APP));
        assert_eq!((&r.config, &r.state), (&r.data, &r.data));
        // Windows: roaming for data and config, local for crashes.
        let r = Roots::resolve(None, None, &bases("C:/U/AppData/Roaming", "C:/U/AppData/Roaming", "C:/U/AppData/Local")).unwrap();
        assert_eq!(r.config, r.data);
        assert_eq!(r.dir(UserDir::Crashes), PathBuf::from("C:/U/AppData/Local/chummer-rs/crashes"));
        assert_eq!(r.dir(UserDir::CustomData), PathBuf::from("C:/U/AppData/Roaming/chummer-rs/customdata"));
    }

    #[test]
    fn xdg_overrides_every_os() {
        let s = "/Users/u/Library/Application Support";
        let r = Roots::resolve(Some("/t/data".into()), Some("/t/config".into()), &bases(s, s, s)).unwrap();
        assert_eq!(r.data, PathBuf::from("/t/data/chummer-rs"));
        assert_eq!(r.state, r.data);
        assert_eq!(r.config, PathBuf::from("/t/config/chummer-rs"));
        // Relative or empty values are ignored.
        assert_eq!(absolute(Some("rel/dir".into())), None);
        assert_eq!(absolute(Some("".into())), None);
        // No bases at all: nothing.
        assert_eq!(Roots::resolve(None, None, &Bases::default()), None);
    }

    #[test]
    fn current_roots_match_the_platform() {
        let r = Roots::current().expect("a home folder");
        let xdg_data = env_path("XDG_DATA_HOME");
        let xdg_config = env_path("XDG_CONFIG_HOME");
        if xdg_data.is_none() {
            let data = r.data.to_string_lossy().replace('\\', "/");
            let state = r.state.to_string_lossy().replace('\\', "/");
            if cfg!(target_os = "macos") {
                assert!(data.ends_with("Library/Application Support/chummer-rs"), "{data}");
                assert_eq!(r.state, r.data);
            } else if cfg!(windows) {
                assert!(data.ends_with("AppData/Roaming/chummer-rs"), "{data}");
                assert!(state.ends_with("AppData/Local/chummer-rs"), "{state}");
            } else {
                assert!(data.ends_with(".local/share/chummer-rs"), "{data}");
            }
        }
        if xdg_config.is_none() {
            let config = r.config.to_string_lossy().replace('\\', "/");
            if cfg!(target_os = "macos") {
                assert!(config.ends_with("Library/Application Support/chummer-rs"), "{config}");
            } else if cfg!(windows) {
                assert!(config.ends_with("AppData/Roaming/chummer-rs"), "{config}");
            } else {
                assert!(config.ends_with(".config/chummer-rs"), "{config}");
            }
        }
    }

    #[test]
    fn legacy_roots_skip_the_current_ones() {
        let r = Roots::resolve(None, None, &bases("/h/.local/share", "/h/.config", "/h/.local/share")).unwrap();
        // Linux-style roots are the current ones: nothing to migrate.
        assert_eq!(legacy_for(&r, &["/h".into()]), Legacy::default());
        let s = "/h/Library/Application Support";
        let r = Roots::resolve(None, None, &bases(s, s, s)).unwrap();
        let l = legacy_for(&r, &["/h".into(), "/h".into()]);
        assert_eq!(l.data, vec![PathBuf::from("/h/.local/share/chummer-rs")]);
        assert_eq!(l.config, vec![PathBuf::from("/h/.config/chummer-rs")]);
        if cfg!(not(any(target_os = "macos", windows))) {
            assert_eq!(legacy_roots(&r), Legacy::default());
        }
    }

    #[test]
    fn migration_moves_once_and_keeps_newer_files() {
        let t = tmp("migrate");
        let home = t.join("home");
        let old_data = home.join(".local/share/chummer-rs");
        let old_config = home.join(".config/chummer-rs");
        for (p, text) in [
            (old_data.join("customdata/My Rules/amend_qualities.xml"), "q"),
            (old_data.join("packs/custom_x_packs.xml"), "k"),
            (old_data.join("crashes/crash-1-2.log"), "boom"),
            (old_data.join("backups/a-1/source"), "/a.chum5"),
            (old_config.join("settings/house.xml"), "<settings/>"),
            (old_config.join("gui.ini"), "theme=classic\n"),
            (old_config.join("sourcebooks.xml"), "<books/>"),
            (old_config.join("node.key"), "old key"),
        ] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        // The new root is not empty: eframe and chummer-net wrote there.
        let support = t.join("support");
        let local = t.join("local");
        let roots = Roots { data: support.join(APP), config: support.join(APP), state: local.join(APP) };
        std::fs::create_dir_all(&roots.data).unwrap();
        std::fs::write(roots.data.join("app.ron"), "ron").unwrap();
        std::fs::write(roots.config.join("node.key"), "new key").unwrap();
        std::fs::create_dir_all(roots.data.join("customdata/Other")).unwrap();

        let legacy = legacy_for(&roots, std::slice::from_ref(&home));
        let log = migrate(&roots, &legacy);
        assert!(!log.is_empty());
        let read = |p: PathBuf| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(roots.data.join("customdata/My Rules/amend_qualities.xml")), "q");
        assert!(roots.data.join("customdata/Other").is_dir());
        assert_eq!(read(roots.data.join("kits/custom_x_packs.xml")), "k");
        assert_eq!(read(roots.state.join("crashes/crash-1-2.log")), "boom");
        assert_eq!(read(roots.state.join("backups/a-1/source")), "/a.chum5");
        assert_eq!(read(roots.config.join("settings/house.xml")), "<settings/>");
        assert_eq!(read(roots.config.join("gui.ini")), "theme=classic\n");
        assert_eq!(read(roots.config.join("sourcebooks.xml")), "<books/>");
        assert_eq!(read(roots.data.join("app.ron")), "ron");
        // The identity at the new place wins; the old one stays.
        assert_eq!(read(roots.config.join("node.key")), "new key");
        assert_eq!(read(old_config.join("node.key")), "old key");
        assert!(old_data.join(MOVED_NOTE).is_file() && old_config.join(MOVED_NOTE).is_file());
        assert!(!old_data.join("customdata").exists());

        // Once: a file put back in the old place is not moved again.
        std::fs::write(old_config.join("guide.ini"), "x").unwrap();
        assert!(migrate(&roots, &legacy).is_empty());
        assert!(!roots.config.join("guide.ini").exists());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn migration_inside_the_roots() {
        let t = tmp("inside");
        // Linux: only packs -> kits.
        let roots = Roots { data: t.join("share/chummer-rs"), config: t.join("config/chummer-rs"), state: t.join("share/chummer-rs") };
        std::fs::create_dir_all(roots.data.join("packs")).unwrap();
        std::fs::write(roots.data.join("packs/custom_a_packs.xml"), "a").unwrap();
        std::fs::create_dir_all(roots.data.join("crashes")).unwrap();
        let log = migrate(&roots, &Legacy::default());
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(roots.data.join("kits/custom_a_packs.xml").is_file());
        assert!(roots.data.join("crashes").is_dir());
        assert!(migrate(&roots, &Legacy::default()).is_empty());
        // Windows: crash folders to LOCALAPPDATA, eframe's data\app.ron up.
        let roots = Roots { data: t.join("Roaming/chummer-rs"), config: t.join("Roaming/chummer-rs"), state: t.join("Local/chummer-rs") };
        std::fs::create_dir_all(roots.data.join("recovery/12")).unwrap();
        std::fs::write(roots.data.join("recovery/12/a.chum5"), "c").unwrap();
        std::fs::create_dir_all(roots.data.join("data")).unwrap();
        std::fs::write(roots.data.join("data/app.ron"), "ron").unwrap();
        migrate(&roots, &Legacy::default());
        assert!(roots.state.join("recovery/12/a.chum5").is_file());
        assert!(!roots.data.join("recovery").exists());
        assert!(roots.data.join("app.ron").is_file() && !roots.data.join("data").exists());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn layout_has_readmes() {
        let t = tmp("layout");
        let roots = Roots { data: t.join("d"), config: t.join("c"), state: t.join("s") };
        ensure_layout(&roots).unwrap();
        for d in UserDir::ALL {
            assert!(roots.dir(d).is_dir(), "{d:?}");
            assert_eq!(roots.dir(d).join("README.txt").is_file(), d.readme().is_some(), "{d:?}");
        }
        assert!(roots.data.join("README.txt").is_file());
        // An edited README is kept.
        std::fs::write(roots.dir(UserDir::Sheets).join("README.txt"), "mine").unwrap();
        ensure_layout(&roots).unwrap();
        assert_eq!(std::fs::read_to_string(roots.dir(UserDir::Sheets).join("README.txt")).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(&t);
    }
}
