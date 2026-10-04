//! Character settings (house rules) editor: duplicate a preset, change its
//! build method, budgets, books, karma costs and options, and save it as a
//! settings file in the user settings directory.

use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::settings::{self, CharacterSettings};
use chummer_core::sources;
use chummer_core::xml::Element;
use eframe::egui::{self, RichText};

/// Labels for options Chummer5a's language files do not name directly.
pub(crate) const LABELS: &[(&str, &str)] = &[
    ("buildpoints", "Starting karma / build points"),
    ("qualitykarmalimit", "Quality karma limit"),
    ("sumtoten", "Sum-to-Ten total"),
    ("availability", "Maximum availability"),
    ("nuyenmaxbp", "Maximum karma for nuyen"),
    ("nuyenperbpwftm", "Nuyen per karma (forward)"),
    ("nuyenperbpwftp", "Nuyen per karma (back)"),
    ("metatypecostskarmamultiplier", "Metatype karma multiplier"),
    ("limbcount", "Limb count"),
    ("restrictedcostmultiplier", "Restricted cost multiplier"),
    ("forbiddencostmultiplier", "Forbidden cost multiplier"),
    ("cyberlimbattributebonuscap", "Cyberlimb attribute bonus cap"),
    ("dicepenaltysustaining", "Dice penalty per sustained spell"),
    ("dronearmorflatnumber", "Drone armor flat number"),
    ("mininitiativedice", "Minimum initiative dice"),
    ("maxinitiativedice", "Maximum initiative dice"),
    ("minastralinitiativedice", "Minimum astral initiative dice"),
    ("maxastralinitiativedice", "Maximum astral initiative dice"),
    ("mincoldsiminitiativedice", "Minimum cold-sim dice"),
    ("maxcoldsiminitiativedice", "Maximum cold-sim dice"),
    ("minhotsiminitiativedice", "Minimum hot-sim dice"),
    ("maxhotsiminitiativedice", "Maximum hot-sim dice"),
    ("morelethalgameplay", "More lethal gameplay"),
    ("spiritforcebasedontotalmag", "Spirit force based on total MAG"),
    ("unarmedimprovementsapplytoweapons", "Unarmed improvements apply to weapons"),
    ("allowinitiationincreatemode", "Allow initiation in creation"),
    ("usepointsonbrokengroups", "Use skill points on broken groups"),
    ("dontdoublequalities", "Don't double quality costs in career"),
    ("dontdoublequalityrefunds", "Don't double quality refunds"),
    ("allow2ndmaxattribute", "Allow a second attribute at maximum"),
    ("esslossreducesmaximumonly", "Essence loss reduces only the maximum"),
    ("allowskillregrouping", "Allow skill regrouping"),
    ("metatypecostskarma", "Metatypes cost karma"),
    ("armordegredation", "Armor degradation"),
    ("specialkarmacostbasedonshownvalue", "Special attribute karma uses shown value"),
    ("donotroundessenceinternally", "Don't round essence internally"),
    ("enforcecapacity", "Enforce capacity"),
    ("restrictrecoil", "Restrict recoil"),
    ("unrestrictednuyen", "Unrestricted nuyen"),
    ("allowhigherstackedfoci", "Allow higher stacked foci"),
    ("dontusecyberlimbcalculation", "Don't average cyberlimb attributes"),
    ("alternatemetatypeattributekarma", "Alternate metatype attribute karma"),
    ("freemartialartspecialization", "Free martial art specialization"),
    ("enableenemytracking", "Track enemies"),
    ("autobackstory", "Auto backstory"),
];

pub struct SettingsEditor {
    selected: usize,
    /// Working copy of a user preset being edited.
    draft: Option<Element>,
    new_name: String,
    message: Option<String>,
}

impl SettingsEditor {
    pub fn new() -> Self {
        SettingsEditor { selected: 0, draft: None, new_name: String::new(), message: None }
    }

    /// Returns true when a preset was saved (the library must be reloaded).
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut saved = false;
        let presets = &engine.settings.presets;
        if presets.is_empty() {
            ui.label(lang.tr("No settings found."));
            return false;
        }
        self.selected = self.selected.min(presets.len() - 1);
        ui.horizontal(|ui| {
            ui.label(lang.tr("Preset"));
            egui::ComboBox::from_id_salt("preset_pick").selected_text(label_of(&presets[self.selected], lang)).width(320.0).show_ui(ui, |ui| {
                for (i, p) in presets.iter().enumerate() {
                    if ui.selectable_label(self.selected == i, label_of(p, lang)).clicked() {
                        self.selected = i;
                        self.draft = None;
                    }
                }
            });
        });
        let preset = &presets[self.selected];
        let editable = preset.file.is_some();
        if self.draft.is_none() && editable {
            self.draft = Some(preset.raw.clone());
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text(lang.tr("Name for a copy")).desired_width(220.0));
            if ui.add_enabled(!self.new_name.trim().is_empty(), egui::Button::new(lang.tr("Duplicate"))).clicked() {
                match duplicate(preset, self.new_name.trim()) {
                    Ok(path) => {
                        self.message = Some(format!("Saved {}", path.display()));
                        self.new_name.clear();
                        saved = true;
                    }
                    Err(e) => self.message = Some(e),
                }
            }
        });
        if !editable {
            ui.weak(lang.tr("Built-in presets cannot be changed. Duplicate one to make your own house rules."));
        }
        if let Some(m) = &self.message {
            ui.label(m);
        }
        ui.separator();

        let mut el = if editable { self.draft.clone().unwrap_or_else(|| preset.raw.clone()) } else { preset.raw.clone() };
        let mut dirty = false;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                ui.heading(lang.tr("General"));
                egui::Grid::new("set_general").num_columns(2).striped(true).show(ui, |ui| {
                    ui.label(lang.tr("Name"));
                    let mut name = el.get("name");
                    if ui.text_edit_singleline(&mut name).changed() {
                        el.set_child_text("name", name);
                        dirty = true;
                    }
                    ui.end_row();
                    ui.label(lang.tr("Build Method"));
                    let mut bm = el.get("buildmethod");
                    egui::ComboBox::from_id_salt("set_bm").selected_text(build_method_label(&bm, lang)).show_ui(ui, |ui| {
                        for m in ["Priority", "SumtoTen", "Karma", "LifeModule"] {
                            if ui.selectable_value(&mut bm, m.to_owned(), build_method_label(m, lang)).changed() {
                                dirty = true;
                            }
                        }
                    });
                    el.set_child_text("buildmethod", bm);
                    ui.end_row();
                    ui.label(lang.tr("Priority Table"));
                    let mut pt = el.get("prioritytable");
                    if ui.text_edit_singleline(&mut pt).changed() {
                        el.set_child_text("prioritytable", pt);
                        dirty = true;
                    }
                    ui.end_row();
                    for c in el.clone().elements() {
                        let t = c.text();
                        if c.elements().next().is_none() && t.trim().parse::<i64>().is_ok() {
                            ui.label(label(&c.name, lang));
                            let mut v: i64 = t.trim().parse().unwrap_or(0);
                            if ui.add(egui::DragValue::new(&mut v)).changed() {
                                el.set_child_text(&c.name, v.to_string());
                                dirty = true;
                            }
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new(RichText::new(lang.tr("Options")).strong()).id_salt("set_options").default_open(true).show(ui, |ui| {
                    egui::Grid::new("set_bools").num_columns(2).show(ui, |ui| {
                        let bools: Vec<Element> = el.elements().filter(|c| matches!(c.text().trim(), "True" | "False")).cloned().collect();
                        for (i, c) in bools.iter().enumerate() {
                            let mut b = c.text().trim() == "True";
                            if ui.checkbox(&mut b, label(&c.name, lang)).changed() {
                                el.set_child_text(&c.name, if b { "True" } else { "False" });
                                dirty = true;
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                });
                egui::CollapsingHeader::new(RichText::new(lang.tr("Karma Costs")).strong()).id_salt("set_karma_costs").show(ui, |ui| {
                    egui::Grid::new("set_karma").num_columns(4).show(ui, |ui| {
                        let costs: Vec<Element> = el.child("karmacost").map(|k| k.elements().cloned().collect()).unwrap_or_default();
                        for (i, c) in costs.iter().enumerate() {
                            ui.label(c.name.trim_start_matches("karma"));
                            let mut v: i64 = c.text().trim().parse().unwrap_or(0);
                            if ui.add(egui::DragValue::new(&mut v)).changed() {
                                el.child_or_insert("karmacost").set_child_text(&c.name, v.to_string());
                                dirty = true;
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                });
                egui::CollapsingHeader::new(RichText::new(lang.tr("Custom data (optional rules)")).strong()).id_salt("set_custom_data").show(ui, |ui| {
                    dirty |= custom_data_ui(ui, &mut el, engine, lang);
                });
                egui::CollapsingHeader::new(RichText::new(lang.tr("Books")).strong()).id_salt("set_books").show(ui, |ui| {
                    let enabled: Vec<String> = el.child("books").map(|b| b.children_named("book").map(Element::text).collect()).unwrap_or_default();
                    let mut set = enabled.clone();
                    egui::Grid::new("set_books").num_columns(3).show(ui, |ui| {
                        for (i, b) in sources::book_list(&engine.store).iter().enumerate() {
                            let mut on = set.contains(&b.code);
                            if ui.checkbox(&mut on, format!("{} ({})", b.name, b.code)).changed() {
                                if on {
                                    set.push(b.code.clone());
                                } else {
                                    set.retain(|x| *x != b.code);
                                }
                            }
                            if i % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                    if set != enabled {
                        let books = el.child_or_insert("books");
                        books.children.clear();
                        for b in set {
                            books.push(Element::with_text("book", b));
                        }
                        dirty = true;
                    }
                });
            });
        });
        if editable {
            if dirty {
                self.draft = Some(el);
            }
            ui.separator();
            if ui.button(RichText::new(lang.tr("Save house rules")).strong()).clicked() {
                if let (Some(path), Some(d)) = (preset.file.clone(), self.draft.clone()) {
                    let mut root = d;
                    root.name = "settings".into();
                    match std::fs::write(&path, root.to_xml_string()) {
                        Ok(()) => {
                            self.message = Some(format!("Saved {}", path.display()));
                            saved = true;
                        }
                        Err(e) => self.message = Some(format!("Could not save: {e}")),
                    }
                }
            }
        }
        saved
    }
}

fn label_of(p: &CharacterSettings, lang: &Language) -> String {
    if p.file.is_some() {
        lang.tr_fmt("{0} (yours)", &[&p.name()])
    } else {
        p.name()
    }
}

fn label(tag: &str, lang: &Language) -> String {
    if let Some((_, l)) = LABELS.iter().find(|(t, _)| *t == tag) {
        return lang.tr(l);
    }
    for prefix in ["Checkbox_Options_", "Label_Options_"] {
        for key in [format!("{prefix}{tag}"), format!("{prefix}{}", capitalize(tag))] {
            if lang.has(&key) {
                return lang.s(&key).trim_end_matches(':').to_owned();
            }
        }
    }
    tag.to_owned()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Copy a preset into the user settings directory under a new name.
fn duplicate(preset: &CharacterSettings, name: &str) -> Result<std::path::PathBuf, String> {
    let dir = settings::user_settings_dir().ok_or("no settings directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let path = dir.join(format!("{file}.xml"));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut root = preset.raw.clone();
    root.name = "settings".into();
    root.set_child_text("name", name);
    root.set_child_text("id", chummer_core::items::new_guid());
    std::fs::write(&path, root.to_xml_string()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Toggle custom data directories for a preset. Writes Chummer's
/// `<customdatadirectorynames>` layout (directoryname, order, enabled).
/// Display name of a `<buildmethod>` value.
fn build_method_label(m: &str, lang: &Language) -> String {
    match m {
        "SumtoTen" => lang.tr("Sum-to-Ten"),
        "LifeModule" => lang.tr("Life Modules"),
        _ => lang.tr(m),
    }
}

fn custom_data_ui(ui: &mut egui::Ui, el: &mut Element, engine: &Engine, lang: &Language) -> bool {
    use chummer_core::custom_data;
    let dirs = engine.custom_data_directories();
    if dirs.is_empty() {
        ui.weak(lang.tr("No custom data directories found."));
        return false;
    }
    let enabled: Vec<String> = custom_data::enabled_directories(el, dirs).iter().map(|d| d.name.clone()).collect();
    let mut set = enabled.clone();
    for d in dirs {
        let mut on = set.contains(&d.name);
        let r = ui.checkbox(&mut on, &d.name);
        let r = match d.manifest.as_ref().and_then(|m| m.description("en-us")) {
            Some(desc) => r.on_hover_text(desc),
            None => r,
        };
        if r.changed() {
            if on {
                set.push(d.name.clone());
            } else {
                set.retain(|n| *n != d.name);
            }
        }
    }
    let chosen: Vec<&custom_data::CustomDataDirectory> = dirs.iter().filter(|d| set.contains(&d.name)).collect();
    for (dir, msg) in custom_data::check_dependencies(&chosen) {
        ui.colored_label(ui.visuals().warn_fg_color, format!("{dir}: {msg}"));
    }
    if set == enabled {
        return false;
    }
    let list = el.child_or_insert("customdatadirectorynames");
    list.children.clear();
    for (i, d) in chosen.iter().enumerate() {
        let mut e = Element::new("customdatadirectoryname");
        e.push(Element::with_text("directoryname", d.save_key()));
        e.push(Element::with_text("order", i.to_string()));
        e.push(Element::with_text("enabled", "True"));
        list.push(e);
    }
    true
}
