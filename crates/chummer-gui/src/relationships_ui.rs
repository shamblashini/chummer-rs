//! The Relationships tab: Contacts, Enemies and Pets & Cohorts sub-tabs
//! (Chummer's `tabPeople` with `ContactControl` and `PetControl` rows).
//!
//! Any entry can be linked to another `.chum5` ("Attach Character"); the
//! linked character's name, metatype, gender, age and mugshot are shown in
//! place of the entry's own, and "Open Character" asks the main window to
//! open it in a tab (see [`take_open_request`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chummer_core::character::Character;
use chummer_core::contacts::{self, ContactType, LinkedCharacter, LinkedPath};
use chummer_core::data::DataStore;
use chummer_core::lang::Language;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

use crate::combo::{self, Combo};
use crate::pdf_ui::Status;

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
    notes_open: HashSet<String>,
    confirm: Option<Confirm>,
    lists: Option<std::rc::Rc<Lists>>,
}

/// Drop-down lists, read once.
struct Lists {
    fields: HashMap<&'static str, Vec<String>>,
    metatypes: Vec<(String, String, String)>,
    critters: Vec<(String, String, String)>,
}

impl RelationshipsPanel {
    /// The tab's contents; true when the character changed.
    pub fn ui(&mut self, ui: &mut egui::Ui, ch: &mut Character, store: &DataStore, lang: &Language, status: &mut Status) -> bool {
        if self.lists.is_none() {
            self.lists = Some(std::rc::Rc::new(Lists {
                fields: contacts::CHOICE_LISTS.iter().map(|(f, _, _)| (*f, contacts::choices(store, f))).collect(),
                metatypes: contacts::metatype_choices(store, "metatypes.xml"),
                critters: contacts::metatype_choices(store, "critters.xml"),
            }));
        }
        let tabs: Vec<(usize, String)> = SUB_TABS.iter().enumerate().map(|(i, (_, l))| (i, lang.tr(l))).collect();
        crate::theme::tab_strip(ui, &mut self.tab, &tabs);
        let kind = SUB_TABS[self.tab.min(2)].0;
        let mut changed = false;
        ui.horizontal(|ui| {
            let add = match kind {
                ContactType::Contact => "Add Contact",
                ContactType::Enemy => "Add Enemy",
                ContactType::Pet => "Add Pet",
            };
            if ui.button(format!("➕ {}", lang.tr(add))).clicked() {
                contacts::add(ch, kind);
                changed = true;
            }
            if kind == ContactType::Contact {
                if ui.button(lang.tr("Add from File")).clicked() {
                    changed |= add_from_file(ch, status);
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
            if ui.small_button("⟳").on_hover_text(lang.tr("Reload")).clicked() {
                self.linked.clear();
            }
        });
        ui.separator();
        let entries: Vec<Element> = contacts::of_type(ch, kind).into_iter().cloned().collect();
        self.refresh_linked(ui.ctx(), ch.file.as_deref(), &entries);
        egui::ScrollArea::both().id_salt(("relationships", self.tab)).auto_shrink(false).show(ui, |ui| {
            if entries.is_empty() {
                ui.weak(lang.tr("None."));
            }
            for c in &entries {
                let guid = c.get("guid");
                ui.push_id(&guid, |ui| {
                    changed |= match kind {
                        ContactType::Pet => self.pet_row(ui, ch, c, lang, status),
                        _ => self.contact_row(ui, ch, c, kind, lang, status),
                    };
                });
                ui.separator();
            }
        });
        changed | self.confirm_dialog(ui.ctx(), ch, lang)
    }

    /// Load the linked files of `entries` whose link is new or changed.
    fn refresh_linked(&mut self, ctx: &egui::Context, owner: Option<&Path>, entries: &[Element]) {
        let startup = contacts::startup_dir();
        for c in entries {
            let guid = c.get("guid");
            let key = (c.get("file"), c.get("relative"));
            if !contacts::is_linked(c) {
                self.linked.remove(&guid);
                continue;
            }
            if self.linked.get(&guid).is_some_and(|l| l.key == key) {
                continue;
            }
            let state = match contacts::resolve(c, &startup, owner) {
                Some(LinkedPath::Found(p)) => LinkedCharacter::load(&p),
                Some(LinkedPath::Unsupported(p)) => Err(format!("{}: compressed .chum5lz saves are not supported", p.display())),
                Some(LinkedPath::Missing(f)) => Err(missing(&f)),
                None => continue,
            };
            let mugshot = state.as_ref().ok().and_then(|l| l.mugshot.as_deref()).and_then(|b| texture(ctx, &guid, b));
            self.linked.insert(guid, Linked { key, state, mugshot });
        }
    }

    fn linked_of(&self, guid: &str) -> Option<&LinkedCharacter> {
        self.linked.get(guid).and_then(|l| l.state.as_ref().ok())
    }

    /// One `ContactControl`: name, location, role, connection, loyalty and
    /// the flags; the stat block below when expanded.
    fn contact_row(&mut self, ui: &mut egui::Ui, ch: &mut Character, c: &Element, kind: ContactType, lang: &Language, status: &mut Status) -> bool {
        let guid = c.get("guid");
        let linked = self.linked_of(&guid).cloned();
        let read_only = c.child("readonly").is_some();
        let lists = self.lists.clone().expect("lists loaded");
        let mut changed = false;
        ui.horizontal(|ui| {
            let open = self.expanded.contains(&guid);
            if ui.small_button(if open { "⏷" } else { "⏵" }).clicked() {
                if open {
                    self.expanded.remove(&guid);
                } else {
                    self.expanded.insert(guid.clone());
                }
            }
            self.mugshot(ui, &guid, 32.0);
            changed |= name_field(ui, ch, c, linked.as_ref(), lang, 150.0);
            ui.label(lang.tr("Location:"));
            changed |= text_field(ui, ch, &guid, c, "location", 110.0, true);
            ui.label(lang.tr("Archetype:"));
            changed |= combo_field(ui, ch, &guid, c, "role", &lists.fields["role"], "contacts.xml", lang, 120.0, true);
            ui.label(lang.tr("Connection:"));
            changed |= int_field(ui, ch, &guid, c, "connection", 1..=12, !read_only);
            ui.label(lang.tr("Loyalty:"));
            let group = c.get_bool("group").unwrap_or(false);
            changed |= int_field(ui, ch, &guid, c, "loyalty", 1..=6, !read_only && !group);
        });
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            changed |= flag_field(ui, ch, &guid, c, "free", &lang.tr("Free"), !read_only);
            let group_enabled = c.get_bool("groupenabled").unwrap_or(true);
            changed |= flag_field(ui, ch, &guid, c, "group", &lang.tr("Group"), !read_only && group_enabled);
            changed |= flag_field(ui, ch, &guid, c, "blackmail", &lang.tr("Blackmail"), !read_only);
            changed |= flag_field(ui, ch, &guid, c, "family", &lang.tr("Family"), !read_only);
            ui.add_space(12.0);
            changed |= self.buttons(ui, ch, c, kind, lang, status, read_only);
        });
        if self.expanded.contains(&guid) {
            egui::Grid::new("stat_block").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                ui.label(lang.tr("Type:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "contacttype", &lists.fields["contacttype"], "contacts.xml", lang, 140.0, true));
                ui.label(lang.tr("Metatype:"));
                ui.horizontal(|ui| match &linked {
                    Some(l) => drop(ui.add_enabled(false, egui::TextEdit::singleline(&mut l.display_metatype()).desired_width(140.0))),
                    None => changed |= metatype_field(ui, ch, &guid, c, &lists.metatypes, lang, 140.0),
                });
                ui.end_row();
                ui.label(lang.tr("Gender:"));
                ui.horizontal(|ui| changed |= linked_or_combo(ui, ch, &guid, c, "gender", linked.as_ref().map(|l| l.gender.clone()), &lists.fields["gender"], lang));
                ui.label(lang.tr("Age:"));
                ui.horizontal(|ui| changed |= linked_or_combo(ui, ch, &guid, c, "age", linked.as_ref().map(|l| l.age.clone()), &lists.fields["age"], lang));
                ui.end_row();
                ui.label(lang.tr("Personal Life:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "personallife", &lists.fields["personallife"], "contacts.xml", lang, 140.0, true));
                ui.label(lang.tr("Preferred Payment:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "preferredpayment", &lists.fields["preferredpayment"], "contacts.xml", lang, 140.0, true));
                ui.end_row();
                ui.label(lang.tr("Hobbies/Vice:"));
                ui.horizontal(|ui| changed |= combo_field(ui, ch, &guid, c, "hobbiesvice", &lists.fields["hobbiesvice"], "contacts.xml", lang, 140.0, true));
                ui.end_row();
            });
        }
        changed | self.notes(ui, ch, c)
    }

    /// One `PetControl`: name, metatype (from `critters.xml`), link,
    /// notes and delete.
    fn pet_row(&mut self, ui: &mut egui::Ui, ch: &mut Character, c: &Element, lang: &Language, status: &mut Status) -> bool {
        let guid = c.get("guid");
        let linked = self.linked_of(&guid).cloned();
        let mut changed = false;
        ui.horizontal(|ui| {
            self.mugshot(ui, &guid, 48.0);
            ui.label(lang.tr("Name:"));
            changed |= name_field(ui, ch, c, linked.as_ref(), lang, 180.0);
            ui.label(lang.tr("Metatype:"));
            match &linked {
                Some(l) => drop(ui.add_enabled(false, egui::TextEdit::singleline(&mut l.display_metatype()).desired_width(180.0))),
                None => {
                    let lists = self.lists.clone().expect("lists loaded");
                    changed |= metatype_field(ui, ch, &guid, c, &lists.critters, lang, 180.0);
                }
            }
            changed |= self.buttons(ui, ch, c, ContactType::Pet, lang, status, false);
        });
        changed | self.notes(ui, ch, c)
    }

    fn mugshot(&self, ui: &mut egui::Ui, guid: &str, size: f32) {
        match self.linked.get(guid) {
            Some(Linked { mugshot: Some(t), .. }) => {
                let s = t.size_vec2();
                let scale = size / s.x.max(s.y).max(1.0);
                ui.image((t.id(), s * scale));
            }
            Some(Linked { state: Err(e), .. }) => {
                ui.colored_label(crate::theme::warn(ui), "⚠").on_hover_text(e);
            }
            _ => {}
        }
    }

    /// Link, notes and delete buttons (`cmdLink`, `cmdNotes`, `cmdDelete`).
    #[allow(clippy::too_many_arguments)]
    fn buttons(&mut self, ui: &mut egui::Ui, ch: &mut Character, c: &Element, kind: ContactType, lang: &Language, status: &mut Status, read_only: bool) -> bool {
        let guid = c.get("guid");
        let mut changed = false;
        let tip = match (kind, contacts::is_linked(c)) {
            (ContactType::Enemy, true) => "Open the linked Enemy save file.",
            (ContactType::Enemy, false) => "Link this Enemy to a Chummer save file.",
            (_, true) => "Open the linked Contact save file.",
            (_, false) => "Link this Contact to a Chummer save file.",
        };
        let link_icon = if contacts::is_linked(c) { "🔗" } else { "📎" };
        ui.menu_button(link_icon, |ui| {
            if contacts::is_linked(c) {
                if ui.button(lang.tr("Open Character")).clicked() {
                    match contacts::resolve(c, &contacts::startup_dir(), ch.file.as_deref()) {
                        Some(LinkedPath::Found(p)) => request_open(ui.ctx(), p),
                        Some(LinkedPath::Unsupported(p)) => *status = Some((format!("{}: compressed .chum5lz saves are not supported", p.display()), true)),
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
                let mut dlg = rfd::FileDialog::new()
                    .add_filter("Chummer character", &["chum5", "chum5lz"])
                    .add_filter("All files", &["*"]);
                if let Some(dir) = ch.file.as_deref().and_then(Path::parent) {
                    dlg = dlg.set_directory(dir);
                }
                if let Some(f) = dlg.pick_file() {
                    changed |= contacts::link(ch, &guid, &f, &contacts::startup_dir());
                }
            }
        })
        .response
        .on_hover_text(lang.tr(tip));
        let notes = c.get("notes");
        let note_tip = if kind == ContactType::Enemy { lang.tr("Edit Enemy Notes.") } else { lang.tr("Edit Contact Notes.") };
        let note_tip = if notes.is_empty() { note_tip } else { format!("{note_tip}\n\n{notes}") };
        let label = if notes.is_empty() { RichText::new("📝") } else { RichText::new("📝").color(crate::theme::accent(ui)) };
        if ui.small_button(label).on_hover_text(note_tip).clicked() && !self.notes_open.remove(&guid) {
            self.notes_open.insert(guid.clone());
        }
        if ui.add_enabled(!read_only, egui::Button::new("🗑").small()).clicked() {
            self.confirm = Some(Confirm::Delete(guid, kind));
        }
        changed
    }

    fn notes(&mut self, ui: &mut egui::Ui, ch: &mut Character, c: &Element) -> bool {
        let guid = c.get("guid");
        if !self.notes_open.contains(&guid) {
            return false;
        }
        let mut text = c.get("notes");
        let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(3).desired_width(f32::INFINITY));
        r.changed() && contacts::set_field(ch, &guid, "notes", &text)
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, ch: &mut Character, lang: &Language) -> bool {
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
        egui::Modal::new(egui::Id::new("relationships_confirm")).show(ctx, |ui| {
            ui.heading(title);
            ui.label(text.replace("\\n", "\n"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(ok).clicked() {
                    choice = Some(true);
                }
                if ui.button(lang.tr("Cancel")).clicked() {
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
                contacts::remove(ch, &guid)
            }
            Some(Confirm::Unlink(guid)) => {
                self.linked.remove(&guid);
                contacts::unlink(ch, &guid)
            }
            None => false,
        }
    }
}

fn missing(file: &str) -> String {
    format!("The save file {file} could not be found.")
}

/// A texture from a base64 PNG/JPEG mugshot.
fn texture(ctx: &egui::Context, guid: &str, b64: &str) -> Option<egui::TextureHandle> {
    let bytes = contacts::decode_base64(b64)?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(format!("mugshot_{guid}"), color, egui::TextureOptions::LINEAR))
}

/// "Add from File": contacts from a Chummer contacts XML file.
fn add_from_file(ch: &mut Character, status: &mut Status) -> bool {
    let Some(f) = rfd::FileDialog::new().add_filter("XML", &["xml"]).add_filter("All files", &["*"]).pick_file() else { return false };
    match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|s| contacts::import(ch, &s)) {
        Ok(n) => {
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
fn name_field(ui: &mut egui::Ui, ch: &mut Character, c: &Element, linked: Option<&LinkedCharacter>, lang: &Language, width: f32) -> bool {
    match linked {
        Some(l) => {
            ui.add_enabled(false, egui::TextEdit::singleline(&mut l.name.clone()).desired_width(width)).on_disabled_hover_text(l.path.display().to_string());
            false
        }
        None => {
            let mut v = c.get("name");
            let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text(lang.tr("Name")).desired_width(width));
            r.changed() && contacts::set_field(ch, &c.get("guid"), "name", &v)
        }
    }
}

fn text_field(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, key: &str, width: f32, enabled: bool) -> bool {
    let mut v = c.get(key);
    let r = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut v).desired_width(width));
    r.changed() && contacts::set_field(ch, guid, key, &v)
}

fn int_field(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, key: &str, range: std::ops::RangeInclusive<i32>, enabled: bool) -> bool {
    let mut v = c.get_i32(key).unwrap_or(1);
    ui.add_enabled(enabled, egui::DragValue::new(&mut v).range(range)).changed() && contacts::set_field(ch, guid, key, &v.to_string())
}

fn flag_field(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, key: &str, label: &str, enabled: bool) -> bool {
    let mut v = c.get_bool(key).unwrap_or(false);
    ui.add_enabled(enabled, egui::Checkbox::new(&mut v, label)).changed() && contacts::set_field(ch, guid, key, if v { "True" } else { "False" })
}

/// An editable drop-down (Chummer's combo boxes accept free text): a text
/// box with a list button of `choices` beside it.
#[allow(clippy::too_many_arguments)]
fn combo_field(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, key: &str, choices: &[String], data_file: &str, lang: &Language, width: f32, enabled: bool) -> bool {
    let mut changed = text_field(ui, ch, guid, c, key, width, enabled);
    let cur = c.get(key);
    let mut pick = None;
    Combo::from_id_salt(("contact_combo", key)).selected_text("").width(16.0).show_ui(ui, |ui| {
        for v in choices {
            if combo::selectable_label(ui, *v == cur, lang.data_name(data_file, "", v)).clicked() {
                pick = Some(v.clone());
            }
        }
    });
    if let Some(v) = pick {
        changed |= contacts::set_field(ch, guid, key, &v);
    }
    changed
}

#[allow(clippy::too_many_arguments)]
fn linked_or_combo(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, key: &str, linked: Option<String>, choices: &[String], lang: &Language) -> bool {
    match linked {
        Some(mut v) => {
            ui.add_enabled(false, egui::TextEdit::singleline(&mut v).desired_width(140.0));
            false
        }
        None => combo_field(ui, ch, guid, c, key, choices, "contacts.xml", lang, 140.0, true),
    }
}

/// The metatype box: free text or a metatype / "Metatype (Metavariant)".
fn metatype_field(ui: &mut egui::Ui, ch: &mut Character, guid: &str, c: &Element, choices: &[(String, String, String)], lang: &Language, width: f32) -> bool {
    let mut changed = text_field(ui, ch, guid, c, "metatype", width, true);
    let cur = c.get("metatype");
    let mut pick = None;
    Combo::from_id_salt("contact_metatype").selected_text("").width(16.0).height(320.0).show_ui(ui, |ui| {
        for (value, metatype, variant) in choices {
            let mt = lang.data_name("metatypes.xml", "", metatype);
            let shown = if variant.is_empty() { mt } else { format!("{mt} ({})", lang.data_name("metatypes.xml", "", variant)) };
            if combo::selectable_label(ui, *value == cur, shown).clicked() {
                pick = Some(value.clone());
            }
        }
    });
    if let Some(v) = pick {
        changed |= contacts::set_field(ch, guid, "metatype", &v);
    }
    changed
}
