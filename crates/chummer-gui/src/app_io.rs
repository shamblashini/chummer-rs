//! Opening, saving, printing and exporting characters without blocking
//! the window.
//!
//! Loading a `.chum5lz` takes a quarter of a second, saving one several
//! seconds (LZMA), and a file dialog blocks for as long as it is open.
//! Each runs on its own thread ([`crate::bg`]); the app picks the result
//! up on a later frame ([`App::poll_io`]). A save writes a copy of the
//! character taken when it started, so editing on meanwhile is safe: the
//! character stays modified when it changed after the copy.

use std::path::{Path, PathBuf};

use chummer_core::character::Character;
use eframe::egui;

use crate::bg;
use crate::view::{CharacterView, Tab};
use crate::App;

/// What to do once a save succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Then {
    #[default]
    Nothing,
    /// Close the character's tab (the unsaved-changes dialog's Save).
    CloseTab,
    /// Quit when every save started for quitting is done.
    Quit,
}

/// A save running on another thread.
struct Saving {
    /// The view (`CharacterView::ws_id`).
    view: u64,
    path: PathBuf,
    /// `Doc::revision` when the copy was taken.
    revision: u64,
    then: Then,
}

/// The open, save and dialog jobs in flight.
#[derive(Default)]
pub struct Io {
    loading: Vec<PathBuf>,
    saving: Vec<Saving>,
    /// Save As dialogs open: (view, then).
    asking: Vec<(u64, Then)>,
    /// The tab characters opened from the command line show.
    pub start_tab: Option<Tab>,
    /// Quitting once the saves for it are done.
    quitting: bool,
    /// After the campaign's Save As dialog and save: close the campaign
    /// (`CloseTab`) or go on quitting (`Quit`).
    pub campaign_then: Then,
    /// Scan the roster again when the running scan is done.
    rescan: bool,
    /// The files from the command line are loading.
    pub startup: bool,
}

const ROSTER_SCAN: &str = "roster-scan";
const CAMPAIGN_SAVE: &str = "save:campaign";


impl Io {
    /// Whether characters are still loading.
    pub fn has_loads(&self) -> bool {
        !self.loading.is_empty()
    }
}

fn load_id(path: &Path) -> String {
    format!("load:{}", path.display())
}

/// The Open Campaign dialog's job.
pub const OPEN_CAMPAIGN: &str = "dialog:open-campaign";

/// Jobs whose result is a status line, `Option<(message, is an error)>`
/// (`None`: nothing to say, e.g. a cancelled dialog).
pub const STATUS: &str = "status:";

impl App {
    /// Open a character or campaign file (a character loads on another
    /// thread and gets its tab when it arrives).
    pub(crate) fn open(&mut self, path: &Path) {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(chummer_core::campaign::EXTENSION)) {
            self.open_campaign(path);
            return;
        }
        if let Some(i) = self.views.iter().position(|v| v.path().as_deref() == Some(path)) {
            self.active = i;
            self.home = None;
            return;
        }
        let owned = path.to_owned();
        let name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        if bg::spawn(&self.ctx, load_id(path), self.lang.tr_fmt("Opening {0}…", &[&name]), move || Character::load(&owned).map_err(|e| e.to_string())) {
            self.io.loading.push(path.to_owned());
        }
    }

    /// File → Open: the dialog runs on its own thread.
    pub(crate) fn open_dialog(&mut self) {
        bg::dialog(&self.ctx, "dialog:open", || {
            rfd::FileDialog::new()
                .add_filter("Chummer character", &["chum5", "chum5lz"])
                .add_filter("Raw Chummer5 Saves", &["chum5"])
                .add_filter("Compressed Chummer5 Saves", &["chum5lz"])
                .add_filter("All files", &["*"])
                .pick_files()
        });
    }

    /// Save (or Save As) character `idx`. Returns whether a save started.
    pub(crate) fn save(&mut self, idx: usize, save_as: bool) -> bool {
        self.save_then(idx, save_as, Then::Nothing)
    }

    /// Save character `idx`, then do `then` once it is written. Returns
    /// whether a save (or its Save As dialog) started.
    pub(crate) fn save_then(&mut self, idx: usize, save_as: bool, then: Then) -> bool {
        if self.views.get(idx).is_some_and(|v| v.campaign_member.is_some()) {
            // A campaign member is saved with its campaign.
            let ok = self.save_campaign(false);
            if ok && then == Then::CloseTab {
                self.close_tab(idx, true);
            }
            return ok;
        }
        let Some(v) = self.views.get(idx) else { return false };
        let view = v.ws_id();
        match (save_as, v.path()) {
            (false, Some(p)) => self.start_save(view, p, then),
            _ => {
                // Keep the current file's format; Chummer's Save As offers
                // both (`DialogFilter_Chum5` / `DialogFilter_Chum5lz`).
                let compressed = v.path().is_some_and(|p| chummer_core::chum5lz::is_chum5lz(&p));
                let (first, second) = if compressed { (("Compressed Chummer5 Saves", "chum5lz"), ("Raw Chummer5 Saves", "chum5")) } else { (("Raw Chummer5 Saves", "chum5"), ("Compressed Chummer5 Saves", "chum5lz")) };
                let name = format!("{}.{}", v.ch().display_name(), first.1);
                let started = bg::dialog(&self.ctx, format!("dialog:saveas:{view}"), move || {
                    rfd::FileDialog::new().add_filter(first.0, &[first.1]).add_filter(second.0, &[second.1]).set_file_name(name).save_file()
                });
                if started {
                    self.io.asking.push((view, then));
                }
                started
            }
        }
    }

    fn start_save(&mut self, view: u64, path: PathBuf, then: Then) -> bool {
        let id = format!("save:{view}");
        let Some(v) = self.views.iter().find(|v| v.ws_id() == view) else { return false };
        if bg::busy(&id) {
            self.status = Some((self.lang.tr("Still saving; try again in a moment."), true));
            return false;
        }
        let revision = v.doc().revision();
        let job = crate::backups::wrap(path.clone(), v.doc().save_job(path.clone()));
        let name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        if !bg::spawn(&self.ctx, id, self.lang.tr_fmt("Saving {0}…", &[&name]), job) {
            return false;
        }
        self.io.saving.push(Saving { view, path, revision, then });
        true
    }

    /// The unsaved-changes dialog's Save on quitting: saves every
    /// modified character (and the campaign), then closes the window.
    pub(crate) fn save_all_and_quit(&mut self) {
        if bg::busy(CAMPAIGN_SAVE) || bg::busy(crate::gm_screen::SAVE_DIALOG) {
            // Goes on once the campaign is saved.
            self.io.campaign_then = Then::Quit;
            return;
        }
        if self.gm.as_ref().is_some_and(|g| g.is_dirty(&self.views)) {
            // The campaign first; the characters once it is written.
            self.io.campaign_then = Then::Quit;
            if !self.save_campaign(false) {
                self.io.campaign_then = Then::Nothing;
            }
            return;
        }
        let dirty: Vec<usize> = (0..self.views.len()).filter(|&i| self.views[i].ch().dirty && self.views[i].campaign_member.is_none()).collect();
        let mut started = 0;
        for i in dirty {
            if self.save_then(i, false, Then::Quit) {
                started += 1;
            } else {
                return;
            }
        }
        if started == 0 {
            self.quit_now();
        } else {
            self.io.quitting = true;
        }
    }

    fn quit_now(&mut self) {
        self.allow_close = true;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Each frame: take what finished on other threads.
    pub(crate) fn poll_io(&mut self) {
        let _s = crate::trace::span("poll background jobs");
        if let Some(files) = bg::take::<Option<Vec<PathBuf>>>("dialog:open") {
            for f in files.unwrap_or_default() {
                self.open(&f);
            }
        }
        if let Some(Some(p)) = bg::take::<Option<PathBuf>>(OPEN_CAMPAIGN) {
            self.open_campaign(&p);
        }
        if let Some(p) = bg::take::<Option<PathBuf>>(crate::gm_screen::SAVE_DIALOG) {
            if !p.is_some_and(|p| self.save_campaign_to(&p)) {
                // Cancelled: nothing follows.
                self.io.campaign_then = Then::Nothing;
                self.io.quitting = false;
            }
        }
        if let Some(r) = bg::take::<Result<(PathBuf, crate::gm_screen::SavedLinked), String>>(CAMPAIGN_SAVE) {
            self.campaign_saved(r);
        }
        if let Some(Some(d)) = bg::take::<Option<PathBuf>>("dialog:roster-folder") {
            self.roster_folders.push(d);
            self.rescan_roster();
        }
        if let Some(r) = bg::take::<Vec<chummer_core::roster::Entry>>(ROSTER_SCAN) {
            self.roster = r;
            if std::mem::take(&mut self.io.rescan) {
                // The folders changed while scanning.
                self.rescan_roster();
            }
        }
        // In the order they were opened (tabs keep that order).
        while let Some(path) = self.io.loading.first().cloned() {
            match bg::take::<Result<Character, String>>(&load_id(&path)) {
                // The job failed outright.
                None if !bg::busy(&load_id(&path)) => {
                    self.io.loading.remove(0);
                    self.status = Some((format!("Could not open {}", path.display()), true));
                }
                None => break,
                Some(r) => {
                    self.io.loading.remove(0);
                    match r {
                        Ok(ch) => self.loaded(&path, ch),
                        Err(e) => self.status = Some((e, true)),
                    }
                }
            }
        }
        for (view, then) in std::mem::take(&mut self.io.asking) {
            let id = format!("dialog:saveas:{view}");
            match bg::take::<Option<PathBuf>>(&id) {
                None if bg::busy(&id) => self.io.asking.push((view, then)),
                Some(Some(path)) => {
                    if !self.start_save(view, path, then) && then == Then::Quit {
                        self.io.quitting = false;
                    }
                }
                // Cancelled (or the dialog failed): quitting waits for the
                // user again.
                Some(None) | None => {
                    if then == Then::Quit {
                        self.io.quitting = false;
                    }
                }
            }
        }
        for s in std::mem::take(&mut self.io.saving) {
            let id = format!("save:{}", s.view);
            match bg::take::<std::io::Result<()>>(&id) {
                None if bg::busy(&id) => self.io.saving.push(s),
                None => self.saved(s, Err(std::io::Error::other("the save failed"))),
                Some(r) => self.saved(s, r),
            }
        }
        for id in bg::finished_with("save:authority:") {
            if let Some(Err(e)) = bg::take::<Result<(), String>>(&id) {
                self.status = Some((format!("Could not save the campaign's online state: {e}"), true));
            }
        }
        let status: Vec<String> = bg::finished_with(STATUS);
        for id in status {
            if let Some(Some(st)) = bg::take::<Option<(String, bool)>>(&id) {
                self.status = Some(st);
            }
        }
        if self.io.quitting && self.io.saving.iter().all(|s| s.then != Then::Quit) && self.io.asking.iter().all(|(_, t)| *t != Then::Quit) {
            self.io.quitting = false;
            // Edited while saving: ask again rather than lose the edit.
            let dirty = self.views.iter().any(|v| v.ch().dirty && v.campaign_member.is_none()) || self.gm.as_ref().is_some_and(|g| g.is_dirty(&self.views));
            if dirty {
                self.pending = Some(crate::Pending::Quit);
            } else {
                self.quit_now();
            }
        }
        if self.io.campaign_then != Then::Nothing && !bg::busy(CAMPAIGN_SAVE) && !bg::busy(crate::gm_screen::SAVE_DIALOG) {
            // The save or its dialog went away without an answer.
            self.io.campaign_then = Then::Nothing;
        }
        let engine = self.engine.clone();
        if let Some(gm) = self.gm.as_mut() {
            // Taken in whichever page is in front: a member tab must not
            // edit the local copy the online one is about to replace.
            gm.take_online(&mut self.online, &engine, &mut self.views);
        }
        bg::keep_painting(&self.ctx);
    }

    fn loaded(&mut self, path: &Path, ch: Character) {
        if let Some(i) = self.views.iter().position(|v| v.path().as_deref() == Some(path)) {
            // Opened twice while loading.
            self.active = i;
            self.home = None;
            return;
        }
        let mut v = crate::trace::time("open file (first compute)", || CharacterView::new(ch, &self.engine));
        if let Some(t) = self.io.start_tab {
            v.set_tab(t);
        }
        self.views.push(v);
        self.active = self.views.len() - 1;
        // Files from the command line keep the page `--window` chose.
        if !self.io.startup {
            self.home = None;
        }
        self.remember(path);
        self.status = Some((format!("Opened {}", path.display()), false));
        if self.io.loading.is_empty() {
            self.io.start_tab = None;
            self.io.startup = false;
        }
    }

    fn saved(&mut self, s: Saving, r: std::io::Result<()>) {
        let idx = self.views.iter().position(|v| v.ws_id() == s.view);
        match r {
            Ok(()) => {
                if let Some(i) = idx {
                    self.views[i].doc_saved(&s.path, s.revision);
                }
                self.status = Some((format!("Saved {}", s.path.display()), false));
                self.remember(&s.path);
                if let (Then::CloseTab, Some(i)) = (s.then, idx) {
                    self.close_tab(i, true);
                }
            }
            Err(e) => {
                self.status = Some((format!("Could not save {}: {e}", s.path.display()), true));
                if s.then == Then::Quit {
                    self.io.quitting = false;
                }
            }
        }
    }

    /// Character Roster → Add folder: the dialog on its own thread, then
    /// a scan.
    pub(crate) fn add_roster_folder(&mut self) {
        bg::dialog(&self.ctx, "dialog:roster-folder", || rfd::FileDialog::new().pick_folder());
    }

    /// Scan the roster folders again (on another thread: a folder may
    /// hold hundreds of characters).
    pub(crate) fn rescan_roster(&mut self) {
        if bg::busy(ROSTER_SCAN) {
            self.io.rescan = true;
            return;
        }
        let folders = self.roster_folders.clone();
        bg::spawn(&self.ctx, ROSTER_SCAN, self.lang.tr("Scanning the character roster…"), move || chummer_core::roster::scan(&folders));
    }

    /// Save the campaign to `p`: the files are written on another thread.
    /// Returns whether the save started.
    pub(crate) fn save_campaign_to(&mut self, p: &Path) -> bool {
        if bg::busy(CAMPAIGN_SAVE) {
            self.status = Some((self.lang.tr("Still saving; try again in a moment."), true));
            return false;
        }
        let Some(gm) = self.gm.as_mut() else { return false };
        let job = gm.save_job(p, &mut self.views);
        let path = p.to_owned();
        let name = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        bg::spawn(&self.ctx, CAMPAIGN_SAVE, self.lang.tr_fmt("Saving {0}…", &[&name]), move || {
            let _s = crate::trace::span("save campaign");
            job().map(|saved| (path, saved))
        })
    }

    fn campaign_saved(&mut self, r: Result<(PathBuf, crate::gm_screen::SavedLinked), String>) {
        let then = std::mem::take(&mut self.io.campaign_then);
        let Some(gm) = self.gm.as_mut() else { return };
        let (path, r) = match r {
            Ok((p, saved)) => (Some(p), Ok(saved)),
            Err(e) => (None, Err(e)),
        };
        if path.as_ref().is_some_and(|p| gm.path.as_ref() != Some(p)) {
            // Another campaign is open now; that one was written.
            if let Some(p) = path {
                self.status = Some((format!("Saved {}", p.display()), false));
            }
            return;
        }
        match (gm.saved(&mut self.views, r), path) {
            (Ok(()), Some(p)) => {
                self.status = Some((format!("Saved {}", p.display()), false));
                self.remember(&p);
                match then {
                    Then::CloseTab => {
                        self.close_campaign(true);
                    }
                    Then::Quit => self.save_all_and_quit(),
                    Then::Nothing => {}
                }
            }
            (Err(e), _) => {
                self.status = Some((e, true));
                self.io.quitting = false;
            }
            (Ok(()), None) => {}
        }
    }

    /// Wait for saves still writing (at exit).
    pub(crate) fn finish_io(&mut self) {
        bg::wait("save:", std::time::Duration::from_secs(120));
        bg::wait(STATUS, std::time::Duration::from_secs(30));
    }
}
