//! The Relationships tab: Contacts, Enemies and Pets & Cohorts sub-tabs
//! (Chummer's `tabPeople` with `ContactControl` and `PetControl` rows).
//!
//! Any entry can be linked to another character file (`.chumrs`, `.chum5`
//! or `.chum5lz`, "Attach Character"); the
//! linked character's name, metatype, gender, age and mugshot are shown in
//! place of the entry's own, and "Open Character" asks the main window to
//! open it in a tab (see [`take_open_request`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chummer_core::command::Command;
use chummer_core::contacts::{self, ContactType, LinkedCharacter, LinkedPath};
use chummer_core::data::DataStore;
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::doc::Doc;
use crate::pdf_ui::Status;
use crate::workspace::widgets::{self, Look};
use crate::workspace::{dialog, icons};

const OPEN_REQUEST: &str = "relationships_open_request";

/// Ask the main window to open a character file in a new tab (or switch
/// to it when it is open already).
pub fn request_open(ctx: &egui::Context, path: PathBuf) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(OPEN_REQUEST), path));
}

/// A file asked for by [`request_open`] since the last call.
pub fn take_open_request(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.data_mut(|d| d.remove_temp::<PathBuf>(egui::Id::new(OPEN_REQUEST)))
}

const SUB_TABS: [(ContactType, &str); 3] =
    [(ContactType::Contact, "Contacts"), (ContactType::Enemy, "Enemies"), (ContactType::Pet, "Pets & Cohorts")];

/// A linked file as last loaded, keyed by the `<file>`/`<relative>` it was
/// loaded for so a new link reloads it.
struct Linked {
    key: (String, String),
    state: Result<LinkedCharacter, String>,
    mugshot: Option<egui::TextureHandle>,
}

enum Confirm {
    Delete(String, ContactType),
    Unlink(String),
}

#[derive(Default)]
pub struct RelationshipsPanel {
    tab: usize,
    linked: HashMap<String, Linked>,
    /// Contacts with their stat block shown (`cmdExpand`).
    expanded: HashSet<String>,
    /// The notes dialog (`EditNotes`), while open.
    notes_edit: Option<NotesEdit>,
    confirm: Option<Confirm>,
    lists: Option<std::rc::Rc<Lists>>,
    /// Linked files loading on another thread: contact guid → the key
    /// being loaded.
    loading: HashMap<String, (String, String)>,
    /// The guid of the contact an Attach Character dialog is for.
    attaching: Option<String>,
    /// An Add from File dialog is open for this panel.
    importing: bool,
}

/// A linked file, loaded on another thread: the character and its
/// decoded mugshot.
type Loaded = (Result<LinkedCharacter, String>, Option<egui::ColorImage>);

/// The panel's file dialogs (`bg`).
const ATTACH: &str = "dialog:contact-attach";
const IMPORT: &str = "dialog:contacts-import";

/// The notes dialog: the text and colour being edited for one entry.
struct NotesEdit {
    guid: String,
    kind: ContactType,
    text: String,
    color: [u8; 3],
    /// The colour picker is open (`btnColorSelect` / `ColorDialog`).
    picking: bool,
}

/// Drag-and-drop payload: the guid of the entry being dragged.
struct DragContact(String);

/// A stored (light-mode) colour as shown in the current theme, like
/// `ColorManager.GenerateCurrentModeColor`.
fn shown_color(ui: &egui::Ui, rgb: [u8; 3]) -> egui::Color32 {
    let [r, g, b] = if ui.visuals().dark_mode { chummer_core::html_color::dark_mode(rgb) } else { rgb };
    egui::Color32::from_rgb(r, g, b)
}

/// Drop-down lists, read once.
struct Lists {
    fields: HashMap<&'static str, Vec<String>>,
    metatypes: Vec<(String, String, String)>,
    critters: Vec<(String, String, String)>,
}

impl RelationshipsPanel {
    /// Show a contact: the Contacts sub-tab, with its stat block open.
    pub fn show_contact(&mut self, guid: &str) {
        self.tab = 0;
        self.expanded.insert(guid.to_owned());
    }

    /// The tab's contents; true when the character changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Doc, store: &DataStore, lang: &Language, status: &mut Status) -> bool {
        let dialogs = self.take_dialogs(ch, status);
        if self.lists.is_none() {
            self.lists = Some(std::rc::Rc::new(Lists {
                fields: contacts::CHOICE_LISTS.iter().map(|(f, _, _)| (*f, contacts::choices(store, f))).collect(),
                metatypes: contacts::metatype_choices(store, "metatypes.xml"),
                critters: contacts::metatype_choices(store, "critters.xml"),
            }));
        }
        let wsl = ws_layout(ui);
        if wsl {
            // Sub-tabs as a segmented switch with the number of entries.
            let labels: Vec<String> = SUB_TABS.iter().map(|(k, l)| format!("{}  {}", lang.tr(l), contacts::of_type(ch, *k).len())).collect();
            let items: Vec<(&str, &str)> = labels.iter().map(|l| (l.as_str(), l.as_str())).collect();
            if let Some(i) = ui.push_id("relationship_tabs", |ui| widgets::segmented(ui, &items, self.tab.min(2), 26.0)).inner {
                self.tab = i;
            }
            ui.add_space(4.0);
        } else {
            let tabs: Vec<(usize, String)> = SUB_TABS.iter().enumerate().map(|(i, (_, l))| (i, lang.tr(l))).collect();
            crate::theme::tab_strip(ui, &mut self.tab, &tabs);
        }
        let kind = SUB_TABS[self.tab.min(2)].0;
        let mut changed = false;
        if wsl {
            changed |= self.ws_toolbar(ui, ch, kind, lang, status);
        } else {
            ui.horizontal(|ui| {
            let add = match kind {
                ContactType::Contact => "Add Contact",
                ContactType::Enemy => "Add Enemy",
                ContactType::Pet => "Add Pet",
            };
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr(add))).clicked() {
                changed |= ch.set(Command::AddContact { kind });
            }
            if kind == ContactType::Contact {
                if ui.button(lang.tr("Add from File")).clicked() && add_from_file(ui.ctx()) {
                    self.importing = true;
                }
                if ui.button(lang.tr("Expand/Collapse All")).clicked() {
                    let all: Vec<String> = contacts::of_type(ch, kind).iter().map(|c| c.get("guid")).collect();
                    if all.iter().all(|g| self.expanded.contains(g)) {
                        self.expanded.clear();
                    } else {
                        self.expanded.extend(all);
                    }
                }
            }
            ui.menu_button(lang.tr("Sort"), |ui| changed |= sort_menu(ui, ch, kind, lang)).response.on_hover_text(lang.tr("Sort the list once; the order is saved"));
            if ui.small_button(crate::theme::glyph("⟳")).on_hover_text(lang.tr("Reload")).clicked() {
                self.linked.clear();
            }
            });
        }
        rule(ui);
        let entries: Vec<Element> = contacts::of_type(ch, kind).into_iter().cloned().collect();
        crate::trace::time("linked contacts", || self.refresh_linked(ui.ctx(), ch.file.as_deref(), &entries));
        egui::ScrollArea::both().id_salt(("relationships", self.tab)).auto_shrink(false).show(ui, |ui| {
            if entries.is_empty() {
                if wsl {
                    ui.label(RichText::new(lang.tr("None.")).size(12.0).color(crate::theme::ws(ui).muted));
                } else {
                    ui.weak(lang.tr("None."));
                }
            }
            let count = entries.len();
            for (i, c) in entries.iter().enumerate() {
                let guid = c.get("guid");
                let row = ui.push_id(&guid, |ui| {
                    // `ContactControl.BackColor` = `Contact.PreferredColor`.
                    let fill = contacts::preferred_color(c).map_or(egui::Color32::TRANSPARENT, |[a, r, g, b]| {
                        let shown = shown_color(ui, [r, g, b]);
                        egui::Color32::from_rgba_unmultiplied(shown.r(), shown.g(), shown.b(), a.min(96))
                    });
                    let margin = if wsl { egui::Margin::symmetric(2, 5) } else { egui::Margin::same(2) };
                    egui::Frame::new().fill(fill).corner_radius(if wsl { 5 } else { 0 }).inner_margin(margin).show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            changed |= self.order_handle(ui, ch, &guid, i, count, lang);
                            ui.vertical(|ui| {
                                changed |= match kind {
                                    ContactType::Pet => self.pet_row(ui, ch, c, lang, status),
                                    _ => self.contact_row(ui, ch, c, kind, lang, status),
                                };
                            });
                        });
                    })
                    .response
                });
                changed |= drop_target(ui, ch, &row.response, &guid);
                rule(ui);
            }
        });
        changed |= self.notes_dialog(ui.ctx(), ch, lang) | self.confirm_dialog(ui.ctx(), ch, lang);
        // A click or drop that changed the list shows on the next frame.
        if changed || ui.input(|i| i.pointer.any_released()) {
            ui.ctx().request_repaint();
        }
        changed || dialogs
    }

    /// "Attach Character": the dialog on its own thread; the contact is
    /// linked when the file is picked ([`RelationshipsPanel::take_dialogs`]).
    fn start_attach(&mut self, ctx: &egui::Context, ch: &Doc, guid: &str) {
        let dir = ch.file.as_deref().and_then(Path::parent).map(Path::to_owned);
        let open = crate::bg::dialog(ctx, ATTACH, move || {
            let mut dlg = rfd::FileDialog::new().add_filter("Character", &["chumrs", "chum5", "chum5lz"]).add_filter("All files", &["*"]);
            if let Some(dir) = dir {
                dlg = dlg.set_directory(dir);
            }
            dlg.pick_file()
        });
        if open {
            self.attaching = Some(guid.to_owned());
        }
    }

    /// The answers of this panel's file dialogs, once there. Returns true
    /// if the character changed.
    fn take_dialogs(&mut self, ch: &mut Doc, status: &mut Status) -> bool {
        let mut changed = false;
        if self.attaching.is_some() {
            if let Some(f) = crate::bg::take::<Option<PathBuf>>(ATTACH) {
                if let (Some(guid), Some(f)) = (self.attaching.take(), f) {
                    let startup = contacts::startup_dir();
                    changed |= ch.set(Command::LinkContact { contact: guid, file: f.to_string_lossy().into_owned(), startup: startup.to_string_lossy().into_owned() });
                }
                self.attaching = None;
            }
        }
        if self.importing {
            if let Some(r) = crate::bg::take::<Option<(PathBuf, Result<String, String>)>>(IMPORT) {
                self.importing = false;
                if let Some((f, xml)) = r {
                    changed |= import_contacts(ch, &f, xml, status);
                }
            }
        }
        changed
    }

    /// The Workspace toolbar: add, add from file, expand all and reload.
    fn ws_toolbar(&mut self, ui: &mut egui::Ui, ch: &mut Doc, kind: ContactType, lang: &Language, _status: &mut Status) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let add = match kind {
                ContactType::Contact => "Add Contact",
                ContactType::Enemy => "Add Enemy",
                ContactType::Pet => "Add Pet",
            };
            if widgets::button(ui, Some(icons::USER_PLUS), &lang.tr(add), Look::Primary, 26.0).clicked() {
                changed |= ch.set(Command::AddContact { kind });
            }
            if kind == ContactType::Contact {
                if widgets::button(ui, Some(icons::FILE_ARROW_UP), &lang.tr("Add from File"), Look::Secondary, 26.0).clicked() && add_from_file(ui.ctx()) {
                    self.importing = true;
                }
                let all: Vec<String> = contacts::of_type(ch, kind).iter().map(|c| c.get("guid")).collect();
                let open = !all.is_empty() && all.iter().all(|g| self.expanded.contains(g));
                let glyph = if open { icons::ARROWS_IN_SIMPLE } else { icons::ARROWS_OUT_SIMPLE };
                if widgets::button(ui, Some(glyph), &lang.tr("Expand/Collapse All"), Look::Ghost, 26.0).clicked() {
                    if open {
                        self.expanded.clear();
                    } else {
                        self.expanded.extend(all);
                    }
                }
            }
            let sort = widgets::button(ui, Some(icons::SORT_ASCENDING), &lang.tr("Sort"), Look::Ghost, 26.0).on_hover_text(lang.tr("Sort the list once; the order is saved"));
            egui::Popup::menu(&sort).show(|ui| changed |= sort_menu(ui, ch, kind, lang));
            if widgets::icon_button(ui, icons::ARROWS_CLOCKWISE, 26.0).on_hover_text(lang.tr("Reload")).clicked() {
                self.linked.clear();
            }
        });
        ui.add_space(4.0);
        changed
    }

    /// Load the linked files of `entries` whose link is new or changed
    /// (on another thread: a linked save is a whole character file).
    fn refresh_linked(&mut self, ctx: &egui::Context, owner: Option<&Path>, entries: &[Element]) {
        for (guid, key) in std::mem::take(&mut self.loading) {
            match crate::bg::take::<Loaded>(&format!("linked:{guid}")) {
                Some((state, img)) => {
                    let mugshot = img.map(|i| ctx.load_texture(format!("mugshot_{guid}"), i, egui::TextureOptions::LINEAR));
                    self.linked.insert(guid, Linked { key, state, mugshot });
                }
                None => {
                    self.loading.insert(guid, key);
                }
            }
        }
        let mut startup = None;
        for c in entries {
            let guid = c.get("guid");
            let key = (c.get("file"), c.get("relative"));
            if !contacts::is_linked(c) {
                self.linked.remove(&guid);
                continue;
            }
            if self.linked.get(&guid).is_some_and(|l| l.key == key) || self.loading.get(&guid) == Some(&key) {
                continue;
            }
            let startup = startup.get_or_insert_with(contacts::startup_dir);
            let found = match contacts::resolve(c, startup, owner) {
                Some(f) => f,
                None => continue,
            };
            let job = move || -> Loaded {
                let state = match found {
                    LinkedPath::Found(p) => LinkedCharacter::load(&p),
                    LinkedPath::Missing(f) => Err(missing(&f)),
                };
                let img = state.as_ref().ok().and_then(|l| l.mugshot.as_deref()).and_then(decode_mugshot);
                (state, img)
            };
            // A load of an older link still running: try again next frame.
            if crate::bg::spawn(ctx, format!("linked:{guid}"), "Loading linked characters…", job) {
                self.loading.insert(guid, key);
            }
        }
    }

    fn linked_of(&self, guid: &str) -> Option<&LinkedCharacter> {
        self.linked.get(guid).and_then(|l| l.state.as_ref().ok())
    }

    /// One `ContactControl`: name, location, role, connection, loyalty and
    /// the flags; the stat block below when expanded.
    fn contact_row(&mut self, ui: &mut egui::Ui, ch: &mut Doc, c: &Element, kind: ContactType, lang: &Language, status: &mut Status) -> bool {
        let guid = c.get("guid");
        let linked = self.linked_of(&guid).cloned();
        let read_only = c.child("readonly").is_some();
        let lists = self.lists.clone().expect("lists loaded");
        let mut changed = false;
        let wsl = ws_layout(ui);
        ui.horizontal(|ui| {
            let open = self.expanded.contains(&guid);
            let toggle = if wsl {
                widgets::icon_button(ui, if open { icons::CARET_DOWN } else { icons::CARET_RIGHT }, 22.0)
            } else {
                ui.small_button(if open { "⏷" } else { "⏵" })
            };
            if toggle.clicked() {
                if open {
                    self.expanded.remove(&guid);
                } else {
                    self.expanded.insert(guid.clone());
                }
            }
            self.mugshot(ui, &guid, 32.0);
            changed |= name_field(ui, ch, c, linked.as_ref(), lang, 150.0);
            caption(ui, &lang.tr("Location:"));
            changed |= text_field(ui, ch, &guid, c, "location", 110.0, true);
            caption(ui, &lang.tr("Archetype:"));
            changed |= combo_field(ui, ch, &guid, c, "role", &lists.fields["role"], "contacts.xml", lang, 120.0, true);
            caption(ui, &lang.tr("Connection:"));
            let max = chummer_core::chargen::connection_maximum(ch);
            changed |= int_field(ui, ch, &guid, c, "connection", 1..=max, !read_only, lang);
            caption(ui, &lang.tr("Loyalty:"));
            let group = c.get_bool("group").unwrap_or(false);
            changed |= int_field(ui, ch, &guid, c, "loyalty", 1..=6, !read_only && !group, lang);
        });
        ui.horizontal(|ui| {
            ui.add_space(if wsl { 30.0 } else { 24.0 });
            changed |= flag_field(ui, ch, &guid, c, "free", &lang.tr("Free"), !read_only);
            let group_enabled = c.get_bool("groupenabled").unwrap_or(true);
            changed |= flag_field(ui, ch, &guid, c, "group", &lang.tr("Group"), !read_only && group_enabled);
            changed |= flag_field(ui, ch, &guid, c, "blackmail", &lang.tr("Blackmail"), !read_only);
            changed |= flag_field(ui, ch, &guid, c, "family", &lang.tr("Family"), !read_only);
            ui.add_space(12.0);
            changed |= self.buttons(ui, ch, c, kind, lang, status, read_only);
        });
        if self.expanded.contains(&guid) {
            if wsl {
                ui.add_space(4.0);
            }
            egui::Grid::new("stat_block").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                caption(ui, &lang.tr("Type:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "contacttype", &lists.fields["contacttype"], "contacts.xml", lang, 140.0, true));
                caption(ui, &lang.tr("Metatype:"));
                ui.horizontal(|ui| match &linked {
                    Some(l) => read_only_field(ui, "metatype", &l.display_metatype(), 158.0),
                    None => changed |= metatype_field(ui, ch, &guid, c, &lists.metatypes, lang, 140.0),
                });
                ui.end_row();
                caption(ui, &lang.tr("Gender:"));
                ui.horizontal(|ui| changed |= linked_or_combo(ui, ch, &guid, c, "gender", linked.as_ref().map(|l| l.gender.clone()), &lists.fields["gender"], lang));
                caption(ui, &lang.tr("Age:"));
                ui.horizontal(|ui| changed |= linked_or_combo(ui, ch, &guid, c, "age", linked.as_ref().map(|l| l.age.clone()), &lists.fields["age"], lang));
                ui.end_row();
                caption(ui, &lang.tr("Personal Life:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "personallife", &lists.fields["personallife"], "contacts.xml", lang, 140.0, true));
                caption(ui, &lang.tr("Preferred Payment:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "preferredpayment", &lists.fields["preferredpayment"], "contacts.xml", lang, 140.0, true));
                ui.end_row();
                caption(ui, &lang.tr("Hobbies/Vice:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "hobbiesvice", &lists.fields["hobbiesvice"], "contacts.xml", lang, 140.0, true));
                ui.end_row();
            });
        }
        changed
    }

    /// One `PetControl`: name, metatype (from `critters.xml`), link,
    /// notes and delete.
    fn pet_row(&mut self, ui: &mut egui::Ui, ch: &mut Doc, c: &Element, lang: &Language, status: &mut Status) -> bool {
        let guid = c.get("guid");
        let linked = self.linked_of(&guid).cloned();
        let mut changed = false;
        ui.horizontal(|ui| {
            self.mugshot(ui, &guid, 48.0);
            caption(ui, &lang.tr("Name:"));
            changed |= name_field(ui, ch, c, linked.as_ref(), lang, 180.0);
            caption(ui, &lang.tr("Metatype:"));
            match &linked {
                Some(l) => read_only_field(ui, "metatype", &l.display_metatype(), 198.0),
                None => {
                    let lists = self.lists.clone().expect("lists loaded");
                    changed |= metatype_field(ui, ch, &guid, c, &lists.critters, lang, 180.0);
                }
            }
            changed |= self.buttons(ui, ch, c, ContactType::Pet, lang, status, false);
        });
        changed
    }

    /// The drag handle (Chummer drags the whole `ContactControl`) and Move
    /// Up / Move Down buttons.
    fn order_handle(&mut self, ui: &mut egui::Ui, ch: &mut Doc, guid: &str, index: usize, count: usize, lang: &Language) -> bool {
        let mut changed = false;
        if ws_layout(ui) {
            let ws = crate::theme::ws(ui);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let id = egui::Id::new(("contact_drag", guid));
                ui.dnd_drag_source(id, DragContact(guid.to_owned()), |ui| {
                    ui.add_sized([18.0, 16.0], egui::Label::new(icons::icon(icons::DOTS_SIX_VERTICAL, 14.0, ws.muted)));
                })
                .response
                .on_hover_text(lang.tr("Drag to reorder"));
                if ui.add_enabled_ui(index > 0, |ui| widgets::icon_button(ui, icons::CARET_UP, 18.0)).inner.on_hover_text(lang.tr("Move Up")).clicked() {
                    changed |= ch.set(Command::MoveContactStep { contact: guid.to_owned(), up: true });
                }
                if ui.add_enabled_ui(index + 1 < count, |ui| widgets::icon_button(ui, icons::CARET_DOWN, 18.0)).inner.on_hover_text(lang.tr("Move Down")).clicked() {
                    changed |= ch.set(Command::MoveContactStep { contact: guid.to_owned(), up: false });
                }
            });
            return changed;
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let id = egui::Id::new(("contact_drag", guid));
            ui.dnd_drag_source(id, DragContact(guid.to_owned()), |ui| {
                ui.label(RichText::new(crate::theme::glyph("☰")).weak());
            })
            .response
            .on_hover_text(lang.tr("Drag to reorder"));
            if ui.add_enabled(index > 0, egui::Button::new("⏶").small().frame(false)).on_hover_text(lang.tr("Move Up")).clicked() {
                changed |= ch.set(Command::MoveContactStep { contact: guid.to_owned(), up: true });
            }
            if ui.add_enabled(index + 1 < count, egui::Button::new("⏷").small().frame(false)).on_hover_text(lang.tr("Move Down")).clicked() {
                changed |= ch.set(Command::MoveContactStep { contact: guid.to_owned(), up: false });
            }
        });
        changed
    }

    fn mugshot(&self, ui: &mut egui::Ui, guid: &str, size: f32) {
        match self.linked.get(guid) {
            Some(Linked { mugshot: Some(t), .. }) => {
                let s = t.size_vec2();
                let scale = size / s.x.max(s.y).max(1.0);
                ui.image((t.id(), s * scale));
            }
            Some(Linked { state: Err(e), .. }) => {
                if ws_layout(ui) {
                    ui.label(icons::icon(icons::WARNING, 16.0, crate::theme::ws(ui).warning)).on_hover_text(e);
                } else {
                    ui.colored_label(crate::theme::warn(ui), crate::theme::glyph("⚠")).on_hover_text(e);
                }
            }
            _ => {}
        }
    }

    /// Link, notes and delete buttons (`cmdLink`, `cmdNotes`, `cmdDelete`).
    #[allow(clippy::too_many_arguments)]
    fn buttons(&mut self, ui: &mut egui::Ui, ch: &mut Doc, c: &Element, kind: ContactType, lang: &Language, status: &mut Status, read_only: bool) -> bool {
        if ws_layout(ui) {
            return self.ws_buttons(ui, ch, c, kind, lang, status, read_only);
        }
        let guid = c.get("guid");
        let changed = false;
        let tip = match (kind, contacts::is_linked(c)) {
            (ContactType::Enemy, true) => "Open the linked Enemy save file.",
            (ContactType::Enemy, false) => "Link this Enemy to a Chummer save file.",
            (_, true) => "Open the linked Contact save file.",
            (_, false) => "Link this Contact to a Chummer save file.",
        };
        let link_icon = if contacts::is_linked(c) { crate::theme::glyph("🔗") } else { crate::theme::glyph("📎") };
        ui.menu_button(link_icon, |ui| {
            if contacts::is_linked(c) {
                if ui.button(lang.tr("Open Character")).clicked() {
                    match contacts::resolve(c, &contacts::startup_dir(), ch.file.as_deref()) {
                        Some(LinkedPath::Found(p)) => request_open(ui.ctx(), p),
                        _ => *status = Some((lang.tr_fmt("The save file {0} could not be found.", &[&c.get("file")]), true)),
                    }
                    ui.close();
                }
                if ui.button(lang.tr("Remove Character")).clicked() {
                    self.confirm = Some(Confirm::Unlink(guid.clone()));
                    ui.close();
                }
            } else if ui.button(lang.tr("Attach Character")).clicked() {
                ui.close();
                self.start_attach(ui.ctx(), ch, &guid);
            }
        })
        .response
        .on_hover_text(lang.tr(tip));
        let notes = c.get("notes");
        let note_tip = if kind == ContactType::Enemy { lang.tr("Edit Enemy Notes.") } else { lang.tr("Edit Contact Notes.") };
        let note_tip = if notes.is_empty() { note_tip } else { format!("{note_tip}\n\n{notes}") };
        // With notes, the button shows the notes colour (the colour
        // Chummer's tree nodes use for items with notes).
        let label = if notes.is_empty() { RichText::new(crate::theme::glyph("📝")) } else { RichText::new(crate::theme::glyph("📝")).color(shown_color(ui, contacts::notes_color(c))) };
        if ui.small_button(label).on_hover_text(note_tip).clicked() {
            self.notes_edit = Some(NotesEdit { guid: guid.clone(), kind, text: notes.replace("\r\n", "\n"), color: contacts::notes_color(c), picking: false });
        }
        if ui.add_enabled(!read_only, egui::Button::new(crate::theme::glyph("🗑")).small()).clicked() {
            self.confirm = Some(Confirm::Delete(guid, kind));
        }
        changed
    }

    /// [`Self::buttons`] in the Workspace style: icon buttons, the link
    /// menu in a popup.
    #[allow(clippy::too_many_arguments)]
    fn ws_buttons(&mut self, ui: &mut egui::Ui, ch: &mut Doc, c: &Element, kind: ContactType, lang: &Language, status: &mut Status, read_only: bool) -> bool {
        let guid = c.get("guid");
        let changed = false;
        let linked = contacts::is_linked(c);
        let tip = match (kind, linked) {
            (ContactType::Enemy, true) => "Open the linked Enemy save file.",
            (ContactType::Enemy, false) => "Link this Enemy to a Chummer save file.",
            (_, true) => "Open the linked Contact save file.",
            (_, false) => "Link this Contact to a Chummer save file.",
        };
        ui.spacing_mut().item_spacing.x = 2.0;
        let link = widgets::icon_button(ui, if linked { icons::LINK } else { icons::PAPERCLIP }, 24.0).on_hover_text(lang.tr(tip));
        egui::Popup::menu(&link).show(|ui| {
            if linked {
                if ui.button(lang.tr("Open Character")).clicked() {
                    match contacts::resolve(c, &contacts::startup_dir(), ch.file.as_deref()) {
                        Some(LinkedPath::Found(p)) => request_open(ui.ctx(), p),
                        _ => *status = Some((lang.tr_fmt("The save file {0} could not be found.", &[&c.get("file")]), true)),
                    }
                    ui.close();
                }
                if ui.button(lang.tr("Remove Character")).clicked() {
                    self.confirm = Some(Confirm::Unlink(guid.clone()));
                    ui.close();
                }
            } else if ui.button(lang.tr("Attach Character")).clicked() {
                ui.close();
                self.start_attach(ui.ctx(), ch, &guid);
            }
        });
        let notes = c.get("notes");
        let note_tip = if kind == ContactType::Enemy { lang.tr("Edit Enemy Notes.") } else { lang.tr("Edit Contact Notes.") };
        let note_tip = if notes.is_empty() { note_tip } else { format!("{note_tip}\n\n{notes}") };
        let r = widgets::icon_button(ui, if notes.is_empty() { icons::NOTE_BLANK } else { icons::NOTE }, 24.0);
        if !notes.is_empty() {
            // A dot in the notes colour, as Chummer colours items with notes.
            ui.painter().circle_filled(r.rect.right_top() + egui::vec2(-5.0, 5.0), 2.5, shown_color(ui, contacts::notes_color(c)));
        }
        if r.on_hover_text(note_tip).clicked() {
            self.notes_edit = Some(NotesEdit { guid: guid.clone(), kind, text: notes.replace("\r\n", "\n"), color: contacts::notes_color(c), picking: false });
        }
        if ui.add_enabled_ui(!read_only, |ui| widgets::icon_button(ui, icons::TRASH, 24.0)).inner.on_hover_text(lang.tr("Delete")).clicked() {
            self.confirm = Some(Confirm::Delete(guid, kind));
        }
        changed
    }

    /// `EditNotes`: the notes text, shown in the notes colour, and "Select
    /// Colour" (only while there are notes). OK saves both.
    fn notes_dialog(&mut self, ctx: &egui::Context, ch: &mut Doc, lang: &Language) -> bool {
        let Some(edit) = &mut self.notes_edit else { return false };
        let mut choice = None;
        let title = if edit.kind == ContactType::Enemy { lang.tr("Edit Enemy Notes.") } else { lang.tr("Edit Contact Notes.") };
        dialog::modal(ctx, "contact_notes", 520.0, |ui| {
            dialog::heading(ui, title.trim_end_matches('.'));
            ui.add_space(4.0);
            let color = shown_color(ui, edit.color);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                dialog::text_area(ui, &mut edit.text, 10, color);
            });
            if edit.picking && !edit.text.is_empty() {
                // The picker edits the stored (light-mode) colour.
                let mut c = egui::Color32::from_rgb(edit.color[0], edit.color[1], edit.color[2]);
                if egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque) {
                    edit.color = [c.r(), c.g(), c.b()];
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!edit.text.is_empty(), |ui| {
                    let select = if ws_layout(ui) { widgets::button(ui, Some(icons::PALETTE), &lang.tr("Select Colour"), Look::Secondary, 26.0) } else { ui.button(lang.tr("Select Colour")) };
                    if dialog::swatch(ui, color).clicked() | select.clicked() {
                        edit.picking = !edit.picking;
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                        choice = Some(false);
                    }
                    if dialog::button(ui, &lang.tr("OK"), true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl) {
                        choice = Some(true);
                    }
                });
            });
        });
        let Some(ok) = choice else { return false };
        let edit = self.notes_edit.take().expect("open");
        if !ok {
            return false;
        }
        let stored = ch.items("contacts", "contact").into_iter().find(|c| c.get("guid") == edit.guid).map(|c| c.get("notes")).unwrap_or_default();
        // Keep the file's line endings when the text did not change.
        let notes = (stored.replace("\r\n", "\n") != edit.text).then_some(edit.text);
        ch.set(Command::SetContactNotes { contact: edit.guid, notes, color: edit.color })
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, ch: &mut Doc, lang: &Language) -> bool {
        let Some(confirm) = &self.confirm else { return false };
        let (title, text, ok) = match confirm {
            Confirm::Delete(_, ContactType::Enemy) => (lang.tr("Delete"), lang.tr("Are you sure you want to delete this Enemy?"), lang.tr("Delete")),
            Confirm::Delete(..) => (lang.tr("Delete"), lang.tr("Are you sure you want to delete this Contact?"), lang.tr("Delete")),
            Confirm::Unlink(_) => (
                lang.tr("Remove Character Association"),
                lang.tr("Are you sure you want to remove this Character association?\\nThe save file will NOT be deleted."),
                lang.tr("Remove Character"),
            ),
        };
        let mut choice = None;
        dialog::modal(ctx, "relationships_confirm", 380.0, |ui| {
            dialog::heading(ui, &title);
            ui.label(text.replace("\\n", "\n"));
            dialog::buttons(ui, |ui| {
                if dialog::button(ui, &ok, true).clicked() {
                    choice = Some(true);
                }
                if dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                    choice = Some(false);
                }
            });
        });
        let Some(yes) = choice else { return false };
        let confirm = self.confirm.take();
        if !yes {
            return false;
        }
        match confirm {
            Some(Confirm::Delete(guid, _)) => {
                self.linked.remove(&guid);
                ch.set(Command::RemoveContact { contact: guid })
            }
            Some(Confirm::Unlink(guid)) => {
                self.linked.remove(&guid);
                ch.set(Command::UnlinkContact { contact: guid })
            }
            None => false,
        }
    }
}

/// Drag and drop onto a row: a line shows where the dragged entry goes
/// (above the row when the pointer is in its upper half, else below);
/// releasing moves it there.
fn drop_target(ui: &mut egui::Ui, ch: &mut Doc, row: &egui::Response, guid: &str) -> bool {
    let Some(dragged) = row.dnd_hover_payload::<DragContact>() else { return false };
    if dragged.0 == guid {
        return false;
    }
    let Some(pos) = ui.ctx().pointer_interact_pos() else { return false };
    let after = pos.y > row.rect.center().y;
    let y = if after { row.rect.bottom() } else { row.rect.top() };
    let stroke = egui::Stroke::new(2.0_f32, crate::theme::accent(ui));
    ui.painter().hline(row.rect.x_range(), y, stroke);
    match row.dnd_release_payload::<DragContact>() {
        Some(p) => ch.set(Command::MoveContact { contact: p.0.clone(), target: guid.to_owned(), after }),
        None => false,
    }
}

fn missing(file: &str) -> String {
    format!("The save file {file} could not be found.")
}

/// A base64 PNG/JPEG mugshot, decoded.
fn decode_mugshot(b64: &str) -> Option<egui::ColorImage> {
    let bytes = contacts::decode_base64(b64)?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// "Add from File": the dialog and the read, on another thread; the
/// contacts are added by [`RelationshipsPanel::take_dialogs`].
fn add_from_file(ctx: &egui::Context) -> bool {
    crate::bg::dialog(ctx, IMPORT, || {
        let f = rfd::FileDialog::new().add_filter("XML", &["xml"]).add_filter("All files", &["*"]).pick_file()?;
        let xml = std::fs::read_to_string(&f).map_err(|e| e.to_string());
        Some((f, xml))
    })
}

/// Contacts from a Chummer contacts XML file read by [`add_from_file`].
fn import_contacts(ch: &mut Doc, f: &Path, xml: Result<String, String>, status: &mut Status) -> bool {
    match xml.and_then(|xml| ch.apply(Command::ImportContacts { xml }).map_err(|e| e.reason)) {
        Ok(r) => {
            let n = r.count.unwrap_or(0);
            *status = Some((format!("Added {n} contacts from {}", f.display()), false));
            n > 0
        }
        Err(e) => {
            *status = Some((format!("{}: {e}", f.display()), true));
            false
        }
    }
}

/// The name box; the linked character's name, read-only, when linked.
fn name_field(ui: &mut egui::Ui, ch: &mut Doc, c: &Element, linked: Option<&LinkedCharacter>, lang: &Language, width: f32) -> bool {
    match linked {
        Some(l) => {
            if ws_layout(ui) {
                ui.add_enabled_ui(false, |ui| widgets::preset_input(ui, "name", &mut l.name.clone(), &[], "", width + 14.0)).inner.on_disabled_hover_text(l.path.display().to_string());
            } else {
                ui.add_enabled(false, egui::TextEdit::singleline(&mut l.name.clone()).desired_width(width)).on_disabled_hover_text(l.path.display().to_string());
            }
            false
        }
        None => {
            let mut v = c.get("name");
            let r = if ws_layout(ui) {
                widgets::preset_input(ui, "name", &mut v, &[], &lang.tr("Name"), width + 14.0)
            } else {
                ui.add(egui::TextEdit::singleline(&mut v).hint_text(lang.tr("Name")).desired_width(width))
            };
            r.changed() && set_field(ch, &c.get("guid"), "name", v)
        }
    }
}

fn set_field(ch: &mut Doc, guid: &str, key: &str, value: String) -> bool {
    ch.set(Command::SetContactField { contact: guid.to_owned(), key: key.to_owned(), value })
}

fn text_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, width: f32, enabled: bool) -> bool {
    let mut v = c.get(key);
    let r = if ws_layout(ui) {
        ui.add_enabled_ui(enabled, |ui| widgets::preset_input(ui, key, &mut v, &[], "", width + 14.0)).inner
    } else {
        ui.add_enabled(enabled, egui::TextEdit::singleline(&mut v).desired_width(width))
    };
    r.changed() && set_field(ch, guid, key, v)
}

/// Connection and loyalty: a stepper in the Workspace, a drag value in Classic.
#[allow(clippy::too_many_arguments)]
fn int_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, range: std::ops::RangeInclusive<i32>, enabled: bool, lang: &Language) -> bool {
    let mut v = c.get_i32(key).unwrap_or(1);
    let changed = if ws_layout(ui) {
        let what = if key == "loyalty" { lang.tr("Loyalty") } else { lang.tr("Connection") };
        let (lo, hi) = (*range.start(), *range.end());
        ui.add_enabled_ui(enabled, |ui| widgets::num_stepper(ui, key, &mut v, lo, hi, &lang.tr_fmt("Lower {0}", &[&what]), &lang.tr_fmt("Raise {0}", &[&what]))).inner.changed()
    } else {
        ui.add_enabled(enabled, egui::DragValue::new(&mut v).range(range)).changed()
    };
    changed && set_field(ch, guid, key, v.to_string())
}

fn flag_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, label: &str, enabled: bool) -> bool {
    let mut v = c.get_bool(key).unwrap_or(false);
    let changed = if ws_layout(ui) { ui.add_enabled_ui(enabled, |ui| widgets::check(ui, &mut v, label)).inner.changed() } else { ui.add_enabled(enabled, egui::Checkbox::new(&mut v, label)).changed() };
    changed && set_field(ch, guid, key, if v { "True" } else { "False" }.to_owned())
}

/// The Sort menu: sorts the entries of `kind` once (a command, so it is
/// one undo step and is saved). Chummer's "Swap Ordering" only switched
/// the panel's flow; a sort changes the order Chummer shows too.
fn sort_menu(ui: &mut egui::Ui, ch: &mut Doc, kind: ContactType, lang: &Language) -> bool {
    let mut changed = false;
    for by in contacts::ContactSort::ALL {
        let label = lang.tr_fmt("By {0}", &[&lang.tr(by.label())]);
        if ui.button(label).clicked() {
            changed |= ch.set(Command::SortContacts { kind, by });
            ui.close();
        }
    }
    changed
}

/// Whether the Workspace layout is active (its widgets replace Classic's).
fn ws_layout(ui: &egui::Ui) -> bool {
    crate::theme::current(ui.ctx()).workspace_layout()
}

/// A field caption ("Location:"): muted small text in the Workspace.
fn caption(ui: &mut egui::Ui, text: &str) {
    if ws_layout(ui) {
        ui.label(RichText::new(text).size(12.0).color(crate::theme::ws(ui).muted));
    } else {
        ui.label(text);
    }
}

/// A line under the toolbar and between entries.
fn rule(ui: &mut egui::Ui) {
    if ws_layout(ui) {
        ui.add_space(2.0);
        widgets::rule(ui);
        ui.add_space(2.0);
    } else {
        ui.separator();
    }
}

/// A disabled text box showing a linked character's value.
fn read_only_field(ui: &mut egui::Ui, key: &str, text: &str, width: f32) {
    if ws_layout(ui) {
        ui.add_enabled_ui(false, |ui| widgets::preset_input(ui, key, &mut text.to_owned(), &[], "", width));
    } else {
        ui.add_enabled(false, egui::TextEdit::singleline(&mut text.to_owned()).desired_width(width - 18.0));
    }
}

/// An editable drop-down (Chummer's combo boxes accept free text): a text
/// box with the `choices` behind a chevron inside it.
#[allow(clippy::too_many_arguments)]
fn combo_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, choices: &[String], data_file: &str, lang: &Language, width: f32, enabled: bool) -> bool {
    let presets: Vec<(String, String)> = choices.iter().map(|v| (v.clone(), lang.data_name(data_file, "", v))).collect();
    preset_field(ui, ch, guid, c, key, &presets, width, enabled)
}

/// A contact's text field with presets (value, label).
#[allow(clippy::too_many_arguments)]
fn preset_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, presets: &[(String, String)], width: f32, enabled: bool) -> bool {
    let mut v = c.get(key);
    let r = ui.add_enabled_ui(enabled, |ui| widgets::preset_input(ui, key, &mut v, presets, "", width + 18.0)).inner;
    r.changed() && set_field(ch, guid, key, v)
}

#[allow(clippy::too_many_arguments)]
fn linked_or_combo(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, key: &str, linked: Option<String>, choices: &[String], lang: &Language) -> bool {
    match linked {
        Some(v) => {
            read_only_field(ui, key, &v, 158.0);
            false
        }
        None => combo_field(ui, ch, guid, c, key, choices, "contacts.xml", lang, 140.0, true),
    }
}

/// The metatype box: free text or a metatype / "Metatype (Metavariant)".
fn metatype_field(ui: &mut egui::Ui, ch: &mut Doc, guid: &str, c: &Element, choices: &[(String, String, String)], lang: &Language, width: f32) -> bool {
    let presets: Vec<(String, String)> = choices
        .iter()
        .map(|(value, metatype, variant)| {
            let mt = lang.data_name("metatypes.xml", "", metatype);
            let shown = if variant.is_empty() { mt } else { format!("{mt} ({})", lang.data_name("metatypes.xml", "", variant)) };
            (value.clone(), shown)
        })
        .collect();
    preset_field(ui, ch, guid, c, "metatype", &presets, width, true)
}
