//! Backups: saving over an existing file first keeps its previous
//! version in `backups/<file>-<hash>/` in the state root, the
//! newest [`crate::prefs::backup_count`] per file. File → Restore
//! backup… lists them and opens one as a modified copy of its file.
//!
//! The copy is made by the save job on its own thread ([`wrap`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chummer_core::character::Character;
use eframe::egui;

use crate::App;

/// The file in each backup folder naming the file it backs up.
const SOURCE: &str = "source.txt";

/// The backups folder ([`chummer_core::paths::UserDir::Backups`]).
pub fn backups_dir() -> Option<PathBuf> {
    chummer_core::paths::user_dir(chummer_core::paths::UserDir::Backups)
}

/// A save job that first keeps the file it is about to replace. A backup
/// that fails does not stop the save.
pub fn wrap(path: PathBuf, job: impl FnOnce() -> std::io::Result<()> + Send + 'static) -> impl FnOnce() -> std::io::Result<()> + Send + 'static {
    move || {
        if let Some(dir) = backups_dir() {
            if let Err(e) = keep(&dir, &path, crate::prefs::backup_count(), SystemTime::now()) {
                eprintln!("chummer-rs: could not back up {}: {e}", path.display());
            }
        }
        job()
    }
}

/// A stable 64-bit FNV-1a hash (the folder name must not change between
/// Rust versions, as `DefaultHasher` may).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The folder for `file`'s backups under `root`.
pub fn folder_for(root: &Path, file: &Path) -> PathBuf {
    let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_owned());
    let stem: String = abs.file_stem().map(|s| s.to_string_lossy().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect()).unwrap_or_default();
    root.join(format!("{stem}-{:016x}", fnv(abs.to_string_lossy().as_bytes())))
}

/// Copy `file` (if it exists) into its backup folder under `root`, named
/// by `now`, then delete the oldest beyond `count`. Nothing is copied
/// when the newest backup has the same contents. Returns the new backup.
pub fn keep(root: &Path, file: &Path, count: usize, now: SystemTime) -> std::io::Result<Option<PathBuf>> {
    if count == 0 || !file.is_file() {
        return Ok(None);
    }
    let dir = folder_for(root, file);
    std::fs::create_dir_all(&dir)?;
    let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_owned());
    std::fs::write(dir.join(SOURCE), abs.to_string_lossy().as_bytes())?;
    let current = std::fs::read(file)?;
    let existing = list_in(&dir);
    if existing.first().is_some_and(|b| std::fs::read(&b.path).is_ok_and(|old| old == current)) {
        return Ok(None);
    }
    let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut name = format!("{}{ext}", crate::safety::stamp(now));
    // Two saves in the same millisecond.
    let mut n = 1;
    while dir.join(&name).exists() {
        name = format!("{}-{n}{ext}", crate::safety::stamp(now));
        n += 1;
    }
    let out = dir.join(name);
    std::fs::write(&out, &current)?;
    rotate(&dir, count)?;
    Ok(Some(out))
}

/// Delete the oldest backups in `dir` beyond `count`.
pub fn rotate(dir: &Path, count: usize) -> std::io::Result<()> {
    for b in list_in(dir).into_iter().skip(count) {
        std::fs::remove_file(&b.path)?;
    }
    Ok(())
}

/// One kept version.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
}

/// The backups in one folder, newest first (by name: the names are
/// sortable time stamps).
fn list_in(dir: &Path) -> Vec<Backup> {
    let mut v: Vec<Backup> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name() != SOURCE && e.path().is_file())
        .map(|e| {
            let meta = e.metadata().ok();
            Backup { path: e.path(), modified: meta.as_ref().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH), size: meta.map_or(0, |m| m.len()) }
        })
        .collect();
    v.sort_by(|a, b| b.path.file_name().cmp(&a.path.file_name()));
    v
}

/// The backups of one file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileBackups {
    pub original: PathBuf,
    pub backups: Vec<Backup>,
}

/// Every backed-up file under `root`, the most recently backed up first.
pub fn list(root: &Path) -> Vec<FileBackups> {
    let mut out: Vec<FileBackups> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let original = PathBuf::from(std::fs::read_to_string(e.path().join(SOURCE)).ok()?.trim());
            let backups = list_in(&e.path());
            (!backups.is_empty()).then_some(FileBackups { original, backups })
        })
        .collect();
    out.sort_by(|a, b| b.backups[0].path.file_name().cmp(&a.backups[0].path.file_name()));
    out
}

/// "3 min ago", "2 days ago".
pub fn age(t: SystemTime, now: SystemTime, lang: &chummer_core::lang::Language) -> String {
    let s = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        0..60 => lang.tr("just now"),
        60..3600 => lang.tr_fmt("{0} min ago", &[&(s / 60)]),
        3600..86400 => lang.tr_fmt("{0} h ago", &[&(s / 3600)]),
        _ => lang.tr_fmt("{0} days ago", &[&(s / 86400)]),
    }
}

// ----- File → Restore backup… -----

const LIST_JOB: &str = "ux:backups-list";
const OPEN_JOB: &str = "ux:backup-open";

/// The Restore backup window.
#[derive(Default)]
pub struct RestoreWindow {
    pub open: bool,
    files: Option<Vec<FileBackups>>,
    /// The backup loading, with the file it restores.
    loading: Option<PathBuf>,
}

impl App {
    /// File → Restore backup…: list the backups (on another thread).
    pub(crate) fn show_restore_backup(&mut self) {
        self.ux.restore.open = true;
        self.ux.restore.files = None;
        if let Some(root) = backups_dir() {
            crate::bg::spawn(&self.ctx, LIST_JOB, self.lang.tr("Looking for backups…"), move || list(&root));
        } else {
            self.ux.restore.files = Some(Vec::new());
        }
    }

    pub(crate) fn restore_window(&mut self, ctx: &egui::Context) {
        if let Some(files) = crate::bg::take::<Vec<FileBackups>>(LIST_JOB) {
            self.ux.restore.files = Some(files);
        }
        if let Some(r) = crate::bg::take::<Result<Character, String>>(OPEN_JOB) {
            let original = self.ux.restore.loading.take();
            match r {
                Ok(ch) => {
                    let name = ch.display_name();
                    self.open_recovered(ch, original);
                    self.status = Some((self.lang.tr_fmt("Opened a backup of {0}; save to keep it", &[&name]), false));
                    self.ux.restore.open = false;
                }
                Err(e) => self.status = Some((e, true)),
            }
        }
        if !self.ux.restore.open {
            return;
        }
        let mut open = true;
        let mut pick: Option<(PathBuf, PathBuf)> = None;
        let current = self.current().and_then(|i| self.views[i].path());
        let now = SystemTime::now();
        egui::Window::new(self.lang.tr("Restore backup")).id(egui::Id::new("restore_backup")).open(&mut open).default_size([560.0, 420.0]).show(ctx, |ui| {
            ui.weak(self.lang.tr("Earlier versions kept when you saved over a file. Opening one gives a modified copy; save it to replace the file."));
            ui.add_space(6.0);
            let Some(files) = &self.ux.restore.files else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.lang.tr("Looking for backups…"));
                });
                return;
            };
            if files.is_empty() {
                ui.label(self.lang.tr("No backups yet."));
                return;
            }
            // The character in front first.
            let mut order: Vec<&FileBackups> = files.iter().collect();
            order.sort_by_key(|f| Some(&f.original) != current.as_ref());
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                for f in order {
                    let name = f.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    egui::CollapsingHeader::new(crate::theme::strong(ui, name)).id_salt(&f.original).default_open(Some(&f.original) == current.as_ref() || files.len() == 1).show(ui, |ui| {
                        ui.weak(f.original.display().to_string());
                        egui::Grid::new(("backups", &f.original)).num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                            for b in &f.backups {
                                ui.label(age(b.modified, now, &self.lang));
                                ui.weak(format!("{} KB", b.size.div_ceil(1024)));
                                let busy = self.ux.restore.loading.is_some();
                                if ui.add_enabled(!busy, egui::Button::new(self.lang.tr("Open"))).clicked() {
                                    pick = Some((b.path.clone(), f.original.clone()));
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
            });
        });
        if !open {
            self.ux.restore.open = false;
        }
        if let Some((backup, original)) = pick {
            if crate::bg::spawn(&self.ctx, OPEN_JOB, self.lang.tr("Opening the backup…"), move || Character::load(&backup).map_err(|e| e.to_string())) {
                self.ux.restore.loading = Some(original);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chummer-backups-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rotation_keeps_the_newest() {
        let d = scratch("rotate");
        let root = d.join("backups");
        let file = d.join("Runner.chum5");
        // No file yet: nothing to keep.
        assert_eq!(keep(&root, &file, 5, SystemTime::now()).unwrap(), None);
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        for i in 0..8 {
            std::fs::write(&file, format!("<character>{i}</character>")).unwrap();
            let b = keep(&root, &file, 5, t0 + Duration::from_secs(i * 60)).unwrap().expect("a backup");
            assert_eq!(std::fs::read_to_string(&b).unwrap(), format!("<character>{i}</character>"));
            assert_eq!(b.extension().unwrap(), "chum5");
        }
        // The same contents again: no new backup.
        assert_eq!(keep(&root, &file, 5, t0 + Duration::from_secs(9999)).unwrap(), None);
        let all = list(&root);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].original, std::path::absolute(&file).unwrap());
        let kept: Vec<String> = all[0].backups.iter().map(|b| std::fs::read_to_string(&b.path).unwrap()).collect();
        assert_eq!(kept, (3..8).rev().map(|i| format!("<character>{i}</character>")).collect::<Vec<_>>(), "newest five, newest first");
        // A smaller count prunes on the next save; 0 keeps nothing new.
        std::fs::write(&file, "<character>x</character>").unwrap();
        keep(&root, &file, 2, t0 + Duration::from_secs(100_000)).unwrap();
        assert_eq!(list(&root)[0].backups.len(), 2);
        assert_eq!(keep(&root, &file, 0, SystemTime::now()).unwrap(), None);
        // Another file gets its own folder.
        let other = d.join("sub").join("Runner.chum5");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "y").unwrap();
        keep(&root, &other, 5, SystemTime::now()).unwrap();
        assert_eq!(list(&root).len(), 2);
        assert_ne!(folder_for(&root, &file), folder_for(&root, &other));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_backup_still_saves() {
        let d = scratch("wrap");
        let file = d.join("a.chum5");
        let f2 = file.clone();
        let job = wrap(file.clone(), move || std::fs::write(&f2, "new"));
        job().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&d);
    }
}
