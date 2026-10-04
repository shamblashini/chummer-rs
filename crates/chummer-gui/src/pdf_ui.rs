//! Sourcebook PDF settings window and clickable source references.

use std::path::PathBuf;
use std::sync::mpsc;

use chummer_core::data::DataStore;
use chummer_core::lang::Language;
use chummer_core::sources::{self, BookInfo, SourceRef, Sourcebook, SourcebookLibrary};
use eframe::egui::{self, RichText};

pub type Status = Option<(String, bool)>;

/// A small "📖 SR5 p. 143" link. Opens the PDF, or reports why it cannot.
pub fn source_link(ui: &mut egui::Ui, lib: &SourcebookLibrary, lang: &Language, r: Option<SourceRef>, status: &mut Status) {
    let Some(r) = r else { return };
    let linked = lib.is_linked(&r.book);
    let text = RichText::new(format!("📖 {r}")).small();
    let text = if linked { text } else { text.weak() };
    let hover = if linked { lang.tr("Open the sourcebook at this page") } else { lang.tr("No PDF linked for this book — Tools → Sourcebooks") };
    if ui.add(egui::Button::new(text).frame(false)).on_hover_text(hover).clicked() {
        open(lib, &r, status);
    }
}

/// Compact icon-only variant for table rows.
pub fn source_icon(ui: &mut egui::Ui, lib: &SourcebookLibrary, r: Option<SourceRef>, status: &mut Status) {
    let Some(r) = r else {
        ui.label("");
        return;
    };
    let linked = lib.is_linked(&r.book);
    let icon = if linked { RichText::new("📖") } else { RichText::new("📖").weak() };
    if ui.add(egui::Button::new(icon).frame(false)).on_hover_text(format!("{r}")).clicked() {
        open(lib, &r, status);
    }
}

pub fn open(lib: &SourcebookLibrary, r: &SourceRef, status: &mut Status) {
    *status = Some(match lib.open(r) {
        Ok(()) => (format!("Opening {r}"), false),
        Err(e) => (e.to_string(), true),
    });
}

pub struct SourcesWindow {
    books: Vec<BookInfo>,
    filter: String,
    only_linked: bool,
    prefixes: Vec<PathBuf>,
    detect: Option<mpsc::Receiver<(String, Option<i32>)>>,
    detect_pending: usize,
    message: Option<String>,
}

impl SourcesWindow {
    pub fn new(store: &DataStore) -> Self {
        SourcesWindow {
            books: sources::book_list(store),
            filter: String::new(),
            only_linked: false,
            prefixes: sources::find_wine_prefixes(),
            detect: None,
            detect_pending: 0,
            message: None,
        }
    }

    /// Returns true when the library changed (so the caller saves it).
    pub fn ui(&mut self, ui: &mut egui::Ui, lib: &mut SourcebookLibrary, lang: &Language) -> bool {
        let mut changed = self.poll_detect(lib);

        ui.horizontal(|ui| {
            ui.label(lang.tr("PDF viewer"));
            crate::combo::Combo::from_id_salt("viewer_preset").selected_text(lang.tr("Choose…")).show_ui(ui, |ui| {
                for v in sources::installed_viewers() {
                    if crate::combo::selectable_label(ui, lib.viewer == v.template, v.name).clicked() {
                        lib.viewer = v.template.to_owned();
                        changed = true;
                    }
                }
            });
            changed |= ui.add(egui::TextEdit::singleline(&mut lib.viewer).desired_width(380.0)).changed();
        });
        ui.weak(lang.tr("{page} and {path} are replaced. Chummer5a's {localpath} also works."));
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            for pfx in self.prefixes.clone() {
                if ui.button(lang.tr_fmt("Import from Chummer5a ({0})", &[&prefix_label(&pfx)])).on_hover_text(pfx.display().to_string()).clicked() {
                    match sources::import_from_wine(&pfx) {
                        Ok(found) => {
                            let n = found.len();
                            for (code, path, offset) in found {
                                lib.books.insert(code, Sourcebook { path: Some(path), offset });
                            }
                            self.message = Some(format!("Imported {n} books from {}", pfx.display()));
                            changed = true;
                        }
                        Err(e) => self.message = Some(format!("Import failed: {e}")),
                    }
                }
            }
            if ui.button(lang.tr("Scan a Folder for PDF Files…")).clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let found = sources::scan_folder(&dir, &self.books);
                    let mut added = 0;
                    for (code, path) in found {
                        let e = lib.books.entry(code).or_default();
                        if e.path.is_none() {
                            e.path = Some(path);
                            added += 1;
                        }
                    }
                    self.message = Some(format!("Linked {added} more books from {}", dir.display()));
                    changed |= added > 0;
                }
            }
            let can_detect = sources::which("pdftotext").is_some();
            let busy = self.detect.is_some();
            let r = ui.add_enabled(can_detect && !busy, egui::Button::new(if busy { lang.tr("Detecting offsets…") } else { lang.tr("Detect page offsets") }));
            let r = if can_detect { r.on_hover_text(lang.tr("Reads each PDF with pdftotext to find where printed page numbers start")) } else { r.on_disabled_hover_text(lang.tr("Install poppler (pdftotext) to detect offsets")) };
            if r.clicked() {
                self.start_detect(lib);
            }
        });
        if let Some(m) = &self.message {
            ui.label(m);
        }
        if self.detect.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(lang.tr_fmt("{0} books left", &[&self.detect_pending]));
            });
            ui.ctx().request_repaint();
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(lang.tr("Filter books")).desired_width(200.0));
            ui.checkbox(&mut self.only_linked, lang.tr("Only linked"));
            ui.weak(lang.tr_fmt("{0} of {1} linked", &[&lib.linked_count(), &self.books.len()]));
        });
        let needle = self.filter.to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("books").striped(true).num_columns(5).spacing([12.0, 4.0]).show(ui, |ui| {
                for h in lang.tr_all(["Code", "Book", "Offset", "PDF", ""]) {
                    ui.strong(h);
                }
                ui.end_row();
                for b in &self.books {
                    let linked = lib.is_linked(&b.code);
                    if (self.only_linked && !linked)
                        || (!needle.is_empty() && !b.name.to_lowercase().contains(&needle) && !b.code.to_lowercase().contains(&needle))
                    {
                        continue;
                    }
                    ui.monospace(&b.code);
                    ui.label(&b.name);
                    let entry = lib.books.entry(b.code.clone()).or_default();
                    changed |= ui.add(egui::DragValue::new(&mut entry.offset).range(-50..=50)).on_hover_text(lang.tr("PDF page = printed page + offset")).changed();
                    match &entry.path {
                        Some(p) => {
                            let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                            if p.is_file() {
                                ui.label(name).on_hover_text(p.display().to_string());
                            } else {
                                ui.colored_label(ui.visuals().error_fg_color, lang.tr_fmt("missing: {0}", &[&name])).on_hover_text(p.display().to_string());
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button(lang.tr("Choose…")).clicked() {
                            if let Some(f) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                                entry.path = Some(f);
                                changed = true;
                            }
                        }
                        if entry.path.is_some()
                            && ui.small_button(lang.tr("Clear")).clicked() {
                                entry.path = None;
                                changed = true;
                            }
                    });
                    ui.end_row();
                }
            });
        });
        changed
    }

    fn start_detect(&mut self, lib: &SourcebookLibrary) {
        let jobs: Vec<(BookInfo, PathBuf)> = self
            .books
            .iter()
            .filter_map(|b| {
                let p = lib.books.get(&b.code)?.path.clone()?;
                (p.is_file() && b.match_text.is_some()).then(|| (b.clone(), p))
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        self.detect_pending = jobs.len();
        std::thread::spawn(move || {
            for (b, p) in jobs {
                let off = sources::detect_offset(&p, &b);
                if tx.send((b.code, off)).is_err() {
                    return;
                }
            }
        });
        self.detect = Some(rx);
        self.message = None;
    }

    fn poll_detect(&mut self, lib: &mut SourcebookLibrary) -> bool {
        let Some(rx) = &self.detect else { return false };
        let mut changed = false;
        let mut found = 0;
        loop {
            match rx.try_recv() {
                Ok((code, off)) => {
                    self.detect_pending = self.detect_pending.saturating_sub(1);
                    if let (Some(off), Some(e)) = (off, lib.books.get_mut(&code)) {
                        if e.offset != off {
                            e.offset = off;
                            changed = true;
                        }
                        found += 1;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.detect = None;
                    self.message = Some("Offset detection finished.".into());
                    break;
                }
            }
        }
        if found > 0 {
            self.message = Some(format!("Found offsets for {found} more books…"));
        }
        changed
    }
}

/// Short name for a Wine prefix: "Steam 12345" for Proton, else the folder.
fn prefix_label(p: &std::path::Path) -> String {
    if p.file_name().is_some_and(|f| f == "pfx") {
        if let Some(app) = p.parent().and_then(|a| a.file_name()) {
            return format!("Steam/Proton {}", app.to_string_lossy());
        }
    }
    p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}
