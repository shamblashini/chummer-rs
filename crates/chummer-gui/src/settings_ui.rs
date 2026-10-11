//! Character settings (house rules) editor: duplicate a preset, change its
//! build method, budgets, books, karma costs and options, and save it as a
//! settings file in the user settings directory. Presets can be exported
//! as standalone Chummer settings files and imported from them.
//!
//! The window lists the presets on the left (yours, those online campaigns
//! shared, then the built-ins; the last two read-only) and shows the selected one's pages on the right; Save,
//! Duplicate, Export and Import sit in a footer that stays visible. A
//! written preset is usable at once: the app reloads the presets
//! ([`crate::App::reload_settings`]).

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

/// The parts of a preset, one page each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    General,
    Options,
    KarmaCosts,
    Books,
    CustomData,
}

impl Page {
    const ALL: [Page; 5] = [Page::General, Page::Options, Page::KarmaCosts, Page::Books, Page::CustomData];

    fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Options => "Options",
            Page::KarmaCosts => "Karma Costs",
            Page::Books => "Books",
            Page::CustomData => "Custom data",
        }
    }
}

/// What waits for the answer to "save your changes first?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Then {
    /// Show another preset (its key).
    Select(String),
    /// Copy the shown preset under this name and show the copy.
    Duplicate(String),
    /// Pick a settings file to import.
    Import,
    /// Close the window.
    Close,
}

/// The answer to the unsaved-changes prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

pub struct SettingsEditor {
    /// The preset shown ([`ui_key`]).
    pub(crate) current: Option<String>,
    /// Working copy of the shown user preset.
    draft: Option<Element>,
    /// The draft differs from the saved preset.
    pub(crate) dirty: bool,
    pub(crate) page: Page,
    /// The name field of the Duplicate prompt while it is open.
    pub(crate) dup_name: Option<String>,
    /// The last result: (text, is an error).
    message: Option<(String, bool)>,
    import: crate::ruleset_ui::ImportFlow,
    /// Waits for the unsaved-changes prompt.
    pub(crate) pending: Option<Then>,
    /// The window is to close (after the prompt).
    close: bool,
    /// The prompt was shown last frame.
    asked: bool,
    pub(crate) book_filter: String,
    option_filter: String,
}

impl Default for SettingsEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsEditor {
    pub fn new() -> Self {
        SettingsEditor {
            current: None,
            draft: None,
            dirty: false,
            page: Page::General,
            dup_name: None,
            message: None,
            import: Default::default(),
            pending: None,
            close: false,
            asked: false,
            book_filter: String::new(),
            option_filter: String::new(),
        }
    }

    /// Start importing a settings file (from a character's missing-preset
    /// warning): the file dialog, once unsaved edits are dealt with.
    pub fn start_import(&mut self, ctx: &egui::Context) {
        self.run(Then::Import, ctx, None);
    }

    /// The Character Settings window. `open` is the window's visibility; closing
    /// it with unsaved edits asks first. Returns true when presets were
    /// written and must be reloaded.
    pub fn window(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language, open: &mut bool) -> bool {
        if !*open {
            return false;
        }
        let mut reload = false;
        let was_open = *open;
        crate::workspace::dialog::window(ctx, "character_settings_editor", &lang.tr("Character Settings"), open, egui::vec2(940.0, 680.0), true, |ui| {
            reload |= self.ui(ui, engine, lang);
        });
        if was_open && !*open && self.dirty {
            // Closed with unsaved edits: ask first.
            *open = true;
            self.pending = Some(Then::Close);
        }
        reload |= self.prompt(ctx, engine, lang);
        if std::mem::take(&mut self.close) {
            *open = false;
        }
        reload
    }

    /// The unsaved-changes prompt while an action waits for it.
    fn prompt(&mut self, ctx: &egui::Context, engine: &Engine, lang: &Language) -> bool {
        let Some(then) = self.pending.clone() else { return false };
        if !self.dirty {
            self.pending = None;
            return self.run(then, ctx, Some(engine));
        }
        let lib = engine.settings();
        let name = self.draft.as_ref().map(|d| d.get("name")).or_else(|| self.current.as_deref().and_then(|k| by_key(&lib, k)).map(CharacterSettings::name)).unwrap_or_default();
        let mut answer = None;
        crate::workspace::dialog::modal(ctx, "settings_unsaved", 400.0, |ui| {
            crate::workspace::dialog::heading(ui, &lang.tr("Unsaved Changes"));
            ui.add_space(4.0);
            ui.label(lang.tr_fmt("Save your changes to {0} first?", &[&name]));
            crate::workspace::dialog::buttons(ui, |ui| {
                if crate::workspace::dialog::button(ui, &lang.tr("Save"), true).clicked() {
                    answer = Some(Answer::Save);
                }
                if crate::workspace::dialog::button(ui, &lang.tr("Don't save"), false).clicked() {
                    answer = Some(Answer::Discard);
                }
                if crate::workspace::dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        // Escape cancels, unless it is the press that closed the window.
        if self.asked && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Answer::Cancel);
        }
        self.asked = true;
        let Some(answer) = answer else { return false };
        self.pending = None;
        self.asked = false;
        let mut reload = false;
        match answer {
            Answer::Cancel => return false,
            Answer::Save => {
                reload = self.save(engine);
                if self.dirty {
                    // Not saved: stay.
                    return reload;
                }
            }
            Answer::Discard => self.revert(),
        }
        reload | self.run(then, ctx, Some(engine))
    }

    /// Do `then` now, or ask first when there are unsaved edits.
    fn run(&mut self, then: Then, ctx: &egui::Context, engine: Option<&Engine>) -> bool {
        if self.dirty {
            self.pending = Some(then);
            return false;
        }
        match then {
            Then::Select(key) => {
                self.show(Some(key));
                false
            }
            Then::Import => {
                self.import.start(ctx);
                false
            }
            Then::Close => {
                self.close = true;
                false
            }
            Then::Duplicate(name) => engine.is_some_and(|e| self.duplicate(e, &name)),
        }
    }

    /// Show the preset `key`, dropping the old draft.
    fn show(&mut self, key: Option<String>) {
        self.current = key;
        self.draft = None;
        self.dirty = false;
        self.dup_name = None;
    }

    fn revert(&mut self) {
        self.draft = None;
        self.dirty = false;
    }

    /// Copy the shown preset as `name` and show the copy (once reloaded).
    fn duplicate(&mut self, engine: &Engine, name: &str) -> bool {
        let lib = engine.settings();
        let Some(preset) = self.current.as_deref().and_then(|k| by_key(&lib, k)) else { return false };
        let res = engine.settings_dir().ok_or_else(|| "no settings directory".to_owned()).and_then(|d| settings::duplicate(preset, name.trim(), &d));
        match res {
            Ok(path) => {
                let key = path.file_name().map(|f| f.to_string_lossy().into_owned());
                self.message = Some((format!("Created {}", path.display()), false));
                self.show(key);
                true
            }
            Err(e) => {
                self.message = Some((e, true));
                false
            }
        }
    }

    /// Write the draft to its file and reload the presets, so what runs
    /// next (a duplicate after "Save") sees it. Returns true when written.
    fn save(&mut self, engine: &Engine) -> bool {
        let lib = engine.settings();
        let Some(preset) = self.current.as_deref().and_then(|k| by_key(&lib, k)) else { return false };
        let (Some(path), Some(d)) = (preset.file.clone(), self.draft.clone()) else { return false };
        match std::fs::write(&path, settings::export_string(&d)) {
            Ok(()) => {
                self.message = Some((format!("Saved {}", path.display()), false));
                // The reloaded preset becomes the new draft.
                self.revert();
                if let Err(e) = engine.reload_settings() {
                    self.message = Some((format!("Saved {}, but could not read the presets again: {e}", path.display()), true));
                }
                true
            }
            Err(e) => {
                self.message = Some((format!("Could not save: {e}"), true));
                false
            }
        }
    }

    fn finish_import(&mut self, done: Option<Result<settings::Imported, String>>, lang: &Language) -> bool {
        match done {
            Some(Ok(i)) => {
                self.message = Some((lang.tr_fmt("Imported {0} as {1}", &[&i.name, &i.path.display()]), false));
                self.show(Some(i.key));
                true
            }
            Some(Err(e)) => {
                self.message = Some((e, true));
                false
            }
            None => false,
        }
    }

    /// The window's contents: the preset list on the left, the shown
    /// preset's pages on the right, the actions in a footer that stays
    /// visible. Returns true when presets were written.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &Engine, lang: &Language) -> bool {
        let mut reload = false;
        let lib = engine.settings();
        if lib.presets.is_empty() {
            ui.label(lang.tr("No settings found."));
            return false;
        }
        // The shown preset; the first when it is gone (or none yet).
        let key = match self.current.as_deref().and_then(|k| by_key(&lib, k)) {
            Some(p) => ui_key(p),
            None => {
                let k = ui_key(&lib.presets[0]);
                self.show(Some(k.clone()));
                k
            }
        };
        let preset = by_key(&lib, &key).expect("shown preset").clone();
        let editable = preset.is_user();
        if editable && self.draft.is_none() {
            self.draft = Some(preset.raw.clone());
            self.dirty = false;
        }
        let ws = crate::workspace::dialog::ws(ui.ctx());

        // Footer: always visible.
        let footer_id = egui::Id::new("settings_footer");
        egui::TopBottomPanel::bottom(footer_id).frame(egui::Frame::NONE).show_separator_line(false).show_inside(ui, |ui| {
            crate::workspace::dialog::rule(ui);
            reload |= self.footer(ui, engine, &preset, lang);
        });
        // The preset list.
        egui::SidePanel::left("settings_presets")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin { right: 8, ..Default::default() }))
            .resizable(true)
            .default_width(220.0)
            .width_range(150.0..=360.0)
            .show_inside(ui, |ui| {
                if let Some(k) = self.preset_list(ui, &lib, &key, lang) {
                    self.run(Then::Select(k), ui.ctx(), Some(engine));
                }
            });
        egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 10, ..Default::default() })).show_inside(ui, |ui| {
            self.header(ui, &preset, editable, ws, engine, lang);
            if let Some(p) = page_tabs(ui, self.page, lang) {
                self.page = p;
            }
            ui.add_space(4.0);
            let mut el = if editable { self.draft.clone().unwrap_or_else(|| preset.raw.clone()) } else { preset.raw.clone() };
            let mut changed = false;
            egui::ScrollArea::vertical().id_salt(("settings_page", self.page as u8)).auto_shrink([false, false]).show(ui, |ui| {
                changed = self.page_ui(ui, &mut el, editable, engine, lang);
            });
            if editable && changed {
                self.dirty = el.to_xml_string() != preset.raw.to_xml_string();
                self.draft = Some(el);
            }
        });
        reload
    }

    /// The presets: yours, each campaign's, the built-ins. Returns the
    /// [`ui_key`] of one clicked.
    fn preset_list(&self, ui: &mut egui::Ui, lib: &settings::SettingsLibrary, current: &str, lang: &Language) -> Option<String> {
        let mut picked = None;
        // The campaigns that shared presets, by scope, in order.
        let mut campaigns: Vec<(String, String)> = Vec::new();
        for c in lib.campaign.iter().filter_map(|p| p.campaign.as_ref()) {
            if !campaigns.iter().any(|(s, _)| *s == c.scope) {
                campaigns.push((c.scope.clone(), c.campaign.clone()));
            }
        }
        let mut groups: Vec<(String, Vec<&CharacterSettings>, Option<String>)> = vec![(lang.tr("Your presets"), lib.presets.iter().filter(|p| p.is_user()).collect(), Some(lang.tr("None yet: duplicate a preset to make your own.")))];
        for (scope, name) in &campaigns {
            groups.push((lang.tr_fmt("From campaign {0} (read-only)", &[name]), lib.campaign_presets(scope).collect(), None));
        }
        groups.push((lang.tr("Built-in (read-only)"), lib.presets.iter().filter(|p| p.is_builtin()).collect(), None));
        egui::ScrollArea::vertical().id_salt("settings_preset_list").auto_shrink([false, false]).show(ui, |ui| {
            for (title, presets, empty) in groups {
                crate::workspace::dialog::caption(ui, &title);
                for p in &presets {
                    let k = ui_key(p);
                    let selected = k == current;
                    let mut name = p.name();
                    if selected && self.dirty {
                        name = format!("{name} *");
                    }
                    let r = crate::workspace::dialog::list_row(ui, selected, &name, &build_method_label(&p.build_method(), lang), false);
                    let hover = match (&p.campaign, &p.file) {
                        (Some(c), _) => lang.tr_fmt("Shared by the GM of {0}. Read-only: duplicate it to make your own.", &[&c.campaign]),
                        (None, Some(f)) => f.display().to_string(),
                        (None, None) => lang.tr("Built-in preset"),
                    };
                    if r.on_hover_text(hover).clicked() && !selected {
                        picked = Some(k);
                    }
                }
                if let Some(e) = empty.filter(|_| presets.is_empty()) {
                    crate::workspace::dialog::note(ui, e);
                }
                ui.add_space(8.0);
            }
        });
        picked
    }

    /// Which preset is shown, whether it can be changed, and whether it has
    /// unsaved changes.
    fn header(&mut self, ui: &mut egui::Ui, preset: &CharacterSettings, editable: bool, ws: bool, engine: &Engine, lang: &Language) {
        let name = self.draft.as_ref().map_or_else(|| preset.name(), |d| d.get("name"));
        let (text, muted, warn) = if ws {
            let p = crate::theme::ws(ui);
            (p.text, p.muted, p.warning)
        } else {
            (ui.visuals().strong_text_color(), ui.visuals().weak_text_color(), ui.visuals().warn_fg_color)
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(lang.tr("Editing")).color(muted));
            let r = ui.label(RichText::new(&name).font(crate::workspace::widgets::bold(16.0)).color(text));
            if let Some(f) = &preset.file {
                r.on_hover_text(f.display().to_string());
            }
            if editable {
                ui.label(RichText::new(lang.tr("your preset")).color(muted));
            } else {
                let (what, why) = match &preset.campaign {
                    Some(c) => (
                        lang.tr_fmt("from campaign {0}, read-only", &[&c.campaign]),
                        lang.tr("The campaign's GM shared this preset; the campaign's characters use it. The GM's changes arrive with the campaign. Duplicate it to make your own house rules."),
                    ),
                    None => (lang.tr("built-in, read-only"), lang.tr("Built-in presets cannot be changed. Duplicate this one to make your own house rules.")),
                };
                ui.label(RichText::new(format!("{} {what}", crate::workspace::icons::LOCK_SIMPLE)).color(muted)).on_hover_text(why);
                if crate::workspace::dialog::button(ui, &lang.tr("Duplicate to edit…"), true).on_hover_text(lang.tr("Copy this preset as a new preset of your own and edit the copy")).clicked() {
                    self.dup_name = Some(lang.tr_fmt("{0} (house rules)", &[&preset.name()]));
                }
            }
            if self.dirty {
                ui.label(RichText::new(format!("{} {}", crate::workspace::icons::PENCIL_SIMPLE, lang.tr("Unsaved changes"))).color(warn).strong());
            }
        });
        let missing = engine.missing_custom_data(preset);
        if !missing.is_empty() {
            let hint = if preset.campaign.is_some() { lang.tr("Ask your GM for these custom data folders.") } else { String::new() };
            ui.label(RichText::new(format!("{} {} {hint}", crate::workspace::icons::WARNING, lang.tr_fmt("Custom data not installed here: {0}.", &[&missing.join(", ")]))).color(warn));
        }
        ui.add_space(4.0);
    }

    /// The action buttons and messages. Returns true when presets were
    /// written.
    fn footer(&mut self, ui: &mut egui::Ui, engine: &Engine, preset: &CharacterSettings, lang: &Language) -> bool {
        let mut reload = false;
        let done = self.import.ui(ui, engine, lang);
        reload |= self.finish_import(done, lang);
        if let Some((m, err)) = &self.message {
            if *err {
                crate::workspace::dialog::warning(ui, m);
            } else {
                crate::workspace::dialog::note(ui, m);
            }
        }
        if let Some(mut name) = self.dup_name.take() {
            let mut keep = true;
            ui.horizontal_wrapped(|ui| {
                crate::workspace::dialog::label(ui, &lang.tr("Name of the copy"));
                let r = crate::workspace::dialog::text_input(ui, &mut name, "", 240.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = !name.trim().is_empty();
                let create = ui.add_enabled_ui(ok, |ui| crate::workspace::dialog::button(ui, &lang.tr("Create copy"), true)).inner.clicked();
                if ok && (create || enter) {
                    keep = false;
                    reload |= self.run(Then::Duplicate(name.trim().to_owned()), ui.ctx(), Some(engine));
                }
                if crate::workspace::dialog::button(ui, &lang.tr("Cancel"), false).clicked() {
                    keep = false;
                }
            });
            if keep {
                self.dup_name = Some(name);
            }
        }
        let editable = preset.is_user();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let save = ui.add_enabled_ui(editable && self.dirty, |ui| crate::workspace::dialog::button(ui, &lang.tr("Save house rules"), true)).inner;
            let save = if editable {
                save.on_disabled_hover_text(lang.tr("No changes to save"))
            } else if preset.campaign.is_some() {
                save.on_disabled_hover_text(lang.tr("A campaign's presets are read-only: duplicate this one to edit it"))
            } else {
                save.on_disabled_hover_text(lang.tr("Built-in presets are read-only: duplicate this one to edit it"))
            };
            if save.clicked() {
                reload |= self.save(engine);
            }
            if ui.add_enabled_ui(self.dirty, |ui| crate::workspace::dialog::button(ui, &lang.tr("Revert"), false)).inner.on_hover_text(lang.tr("Drop the unsaved changes")).clicked() {
                self.revert();
            }
            ui.separator();
            if crate::workspace::dialog::button(ui, &lang.tr("Duplicate…"), false).on_hover_text(lang.tr("Copy this preset as a new preset of your own")).clicked() {
                self.dup_name = Some(lang.tr_fmt("{0} (copy)", &[&preset.name()]));
            }
            if crate::workspace::dialog::button(ui, &lang.tr("Export…"), false).on_hover_text(lang.tr("Save this preset as a settings file to share, e.g. with your players. Chummer5a reads it too.")).clicked() {
                let file = if !preset.is_builtin() && preset.key().to_ascii_lowercase().ends_with(".xml") { preset.key() } else { format!("{}.xml", settings::file_stem_for(&preset.name())) };
                // The dialog and the write on their own thread.
                let preset = preset.clone();
                crate::bg::spawn(ui.ctx(), format!("{}settings-export", crate::app_io::STATUS), lang.tr("Exporting…"), move || {
                    let out = rfd::FileDialog::new().add_filter("Chummer settings", &["xml"]).set_file_name(file).save_file()?;
                    Some(match settings::export(&preset, &out) {
                        Ok(()) => (format!("Saved {}", out.display()), false),
                        Err(e) => (format!("Could not save: {e}"), true),
                    })
                });
            }
            if crate::workspace::dialog::button(ui, &lang.tr("Import settings file…"), false).on_hover_text(lang.tr("Install a settings file someone shared, e.g. your GM's house rules.")).clicked() {
                reload |= self.run(Then::Import, ui.ctx(), Some(engine));
            }
        });
        reload
    }

    /// The shown page. Returns true when `el` changed.
    fn page_ui(&mut self, ui: &mut egui::Ui, el: &mut Element, editable: bool, engine: &Engine, lang: &Language) -> bool {
        match self.page {
            Page::General => ui.add_enabled_ui(editable, |ui| general_ui(ui, el, engine, lang)).inner,
            Page::Options => options_ui(ui, el, editable, &mut self.option_filter, lang),
            Page::KarmaCosts => ui.add_enabled_ui(editable, |ui| karma_costs_ui(ui, el, lang)).inner,
            Page::Books => books_ui(ui, el, editable, &mut self.book_filter, &sources::book_list(&engine.store), lang),
            Page::CustomData => ui.add_enabled_ui(editable, |ui| custom_data_ui(ui, el, engine, lang)).inner,
        }
    }
}

/// How the window names a preset: its key, or for a campaign's shared
/// preset the campaign scope and its key (a user file may have the same
/// key).
pub(crate) fn ui_key(p: &CharacterSettings) -> String {
    match &p.campaign {
        Some(c) => format!("campaign:{}:{}", c.scope, c.key),
        None => p.key(),
    }
}

/// The preset whose [`ui_key`] is `key`.
fn by_key<'a>(lib: &'a settings::SettingsLibrary, key: &str) -> Option<&'a CharacterSettings> {
    lib.presets.iter().chain(&lib.campaign).find(|p| ui_key(p) == key)
}

/// The row of page tabs. Returns the page clicked.
fn page_tabs(ui: &mut egui::Ui, page: Page, lang: &Language) -> Option<Page> {
    let labels: Vec<String> = Page::ALL.iter().map(|p| lang.tr(p.label())).collect();
    let mut picked = None;
    if crate::workspace::dialog::ws(ui.ctx()) {
        let font = egui::FontId::proportional(12.5);
        let need: f32 = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), egui::Color32::PLACEHOLDER).size().x + 15.0).sum();
        if need + 4.0 <= ui.available_width() {
            let items: Vec<(&str, &str)> = labels.iter().map(|l| (l.as_str(), "")).collect();
            let at = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
            return crate::workspace::widgets::segmented(ui, &items, at, 26.0).map(|i| Page::ALL[i]);
        }
        // Too narrow for the segmented switch: buttons that wrap.
        ui.horizontal_wrapped(|ui| {
            for (p, l) in Page::ALL.iter().zip(&labels) {
                let look = if *p == page { crate::workspace::widgets::Look::Primary } else { crate::workspace::widgets::Look::Ghost };
                if crate::workspace::widgets::button(ui, None, l, look, 24.0).clicked() {
                    picked = Some(*p);
                }
            }
        });
        return picked;
    }
    ui.horizontal_wrapped(|ui| {
        for (p, l) in Page::ALL.iter().zip(&labels) {
            if ui.selectable_label(*p == page, l).clicked() {
                picked = Some(*p);
            }
        }
    });
    ui.separator();
    picked
}

/// Name, build method, priority table and the numeric values.
fn general_ui(ui: &mut egui::Ui, el: &mut Element, engine: &Engine, lang: &Language) -> bool {
    let mut dirty = false;
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
        crate::combo::Combo::from_id_salt("set_bm").selected_text(build_method_label(&bm, lang)).show_ui(ui, |ui| {
            for m in ["Priority", "SumtoTen", "Karma", "LifeModule"] {
                if crate::combo::selectable_value(ui, &mut bm, m.to_owned(), build_method_label(m, lang)).changed() {
                    dirty = true;
                }
            }
        });
        if dirty {
            el.set_child_text("buildmethod", bm);
        }
        ui.end_row();
        ui.label(lang.tr("Priority Table"));
        let mut pt = el.get("prioritytable");
        let mut tables = priority_tables(engine);
        if !pt.is_empty() && !tables.contains(&pt) {
            tables.push(pt.clone());
        }
        crate::combo::Combo::from_id_salt("set_pt").selected_text(pt.clone()).show_ui(ui, |ui| {
            for t in &tables {
                if crate::combo::selectable_value(ui, &mut pt, t.clone(), t).changed() {
                    el.set_child_text("prioritytable", pt.clone());
                    dirty = true;
                }
            }
        });
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
    dirty
}

/// The priority tables `priorities.xml` names.
fn priority_tables(engine: &Engine) -> Vec<String> {
    let Ok(doc) = engine.store.doc("priorities.xml") else { return Vec::new() };
    doc.child("prioritytables").map(|t| t.children_named("prioritytable").map(Element::text).collect()).unwrap_or_default()
}

/// The yes/no options, two columns, with a filter.
fn options_ui(ui: &mut egui::Ui, el: &mut Element, editable: bool, filter: &mut String, lang: &Language) -> bool {
    let mut dirty = false;
    crate::workspace::dialog::search(ui, filter, &lang.tr("Filter options"), 260.0);
    ui.add_space(4.0);
    let needle = filter.trim().to_lowercase();
    let bools: Vec<(Element, String)> =
        el.elements().filter(|c| matches!(c.text().trim(), "True" | "False")).map(|c| (c.clone(), label(&c.name, lang))).filter(|(_, l)| crate::combo::matches(&l.to_lowercase(), &needle)).collect();
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("set_bools").num_columns(2).show(ui, |ui| {
            for (i, (c, l)) in bools.iter().enumerate() {
                let mut b = c.text().trim() == "True";
                if crate::workspace::dialog::check(ui, &mut b, l).changed() {
                    el.set_child_text(&c.name, if b { "True" } else { "False" });
                    dirty = true;
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    });
    if bools.is_empty() {
        crate::workspace::dialog::note(ui, lang.tr("No options match."));
    }
    dirty
}

fn karma_costs_ui(ui: &mut egui::Ui, el: &mut Element, lang: &Language) -> bool {
    let mut dirty = false;
    egui::Grid::new("set_karma").num_columns(4).striped(true).show(ui, |ui| {
        let costs: Vec<Element> = el.child("karmacost").map(|k| k.elements().cloned().collect()).unwrap_or_default();
        for (i, c) in costs.iter().enumerate() {
            let l = label(&c.name, lang);
            ui.label(if l == c.name { c.name.trim_start_matches("karma").to_owned() } else { l });
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
    dirty
}

/// The books whose code or name contain every word of `query` (any case).
pub(crate) fn filter_books<'a>(books: &'a [sources::BookInfo], query: &str) -> Vec<&'a sources::BookInfo> {
    let needle = query.trim().to_lowercase();
    books.iter().filter(|b| crate::combo::matches(&format!("{} {}", b.code, b.name).to_lowercase(), &needle)).collect()
}

/// The preset's enabled book codes.
fn enabled_books(el: &Element) -> Vec<String> {
    el.child("books").map(|b| b.children_named("book").map(Element::text).collect()).unwrap_or_default()
}

fn set_books(el: &mut Element, codes: Vec<String>) {
    let books = el.child_or_insert("books");
    books.children.clear();
    for b in codes {
        books.push(Element::with_text("book", b));
    }
}

/// The sourcebooks with a search and All / None / Core only.
fn books_ui(ui: &mut egui::Ui, el: &mut Element, editable: bool, filter: &mut String, books: &[sources::BookInfo], lang: &Language) -> bool {
    let enabled = enabled_books(el);
    let mut set = enabled.clone();
    let on = books.iter().filter(|b| b.permanent || set.contains(&b.code)).count();
    ui.horizontal_wrapped(|ui| {
        crate::workspace::dialog::search(ui, filter, &lang.tr("Search books (code or name)"), 260.0);
        ui.add_enabled_ui(editable, |ui| {
            if crate::workspace::dialog::button(ui, &lang.tr("All"), false).on_hover_text(lang.tr("Enable every book")).clicked() {
                set = books.iter().map(|b| b.code.clone()).collect();
            }
            if crate::workspace::dialog::button(ui, &lang.tr("None"), false).on_hover_text(lang.tr("Only the core rulebook, which is always enabled")).clicked() {
                set = books.iter().filter(|b| b.permanent).map(|b| b.code.clone()).collect();
            }
        });
        crate::workspace::dialog::note(ui, lang.tr_fmt("{0} of {1} enabled", &[&on, &books.len()]));
    });
    ui.add_space(4.0);
    let shown = filter_books(books, filter);
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new("set_books").num_columns(2).striped(true).show(ui, |ui| {
            for b in &shown {
                let mut checked = b.permanent || set.contains(&b.code);
                let r = ui.add_enabled_ui(!b.permanent, |ui| crate::workspace::dialog::check(ui, &mut checked, &b.code)).inner;
                if b.permanent {
                    r.on_disabled_hover_text(lang.tr("The core rulebook is always enabled"));
                } else if r.changed() {
                    if checked {
                        set.push(b.code.clone());
                    } else {
                        set.retain(|x| *x != b.code);
                    }
                }
                ui.label(&b.name);
                ui.end_row();
            }
        });
    });
    if shown.is_empty() {
        crate::workspace::dialog::note(ui, lang.tr("No books match."));
    }
    if set != enabled {
        set_books(el, set);
        return true;
    }
    false
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

/// Display name of a `<buildmethod>` value.
fn build_method_label(m: &str, lang: &Language) -> String {
    match m {
        "SumtoTen" => lang.tr("Sum-to-Ten"),
        "LifeModule" => lang.tr("Life Modules"),
        _ => lang.tr(m),
    }
}

/// "Open folder" for the user's custom data folder, with its path; a
/// failure to open it shows under the button.
fn user_folder_row(ui: &mut egui::Ui, lang: &Language) {
    let dir = chummer_core::paths::user_dir(chummer_core::paths::UserDir::CustomData);
    let err_id = egui::Id::new("custom_data_folder_error");
    ui.horizontal_wrapped(|ui| {
        if crate::prefs::open_button(ui, lang, "Open folder").on_hover_text(lang.tr("Put your own custom data folders here; they are listed at the next start")).clicked() {
            let res = dir.as_deref().map_or_else(|| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no home folder")), crate::prefs::open_folder);
            let msg = res.err().map(|e| format!("{e}"));
            ui.ctx().data_mut(|d| d.insert_temp(err_id, msg));
        }
        if let Some(d) = &dir {
            ui.weak(d.display().to_string());
        }
    });
    if let Some(msg) = ui.ctx().data(|d| d.get_temp::<Option<String>>(err_id)).flatten() {
        ui.colored_label(ui.visuals().warn_fg_color, msg);
    }
    ui.add_space(4.0);
}

/// Toggle custom data directories for a preset. Writes Chummer's
/// `<customdatadirectorynames>` layout (directoryname, order, enabled).
fn custom_data_ui(ui: &mut egui::Ui, el: &mut Element, engine: &Engine, lang: &Language) -> bool {
    use chummer_core::custom_data;
    let dirs = engine.custom_data_directories();
    user_folder_row(ui, lang);
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

impl crate::App {
    /// Read the settings presets again after one was added, imported or
    /// saved, and let open characters and the GM screen use them.
    pub(crate) fn reload_settings(&mut self) {
        if let Err(e) = self.engine.reload_settings() {
            self.status = Some((format!("Could not read the settings presets: {e}"), true));
            return;
        }
        self.follow_settings();
    }

    /// When the presets changed (a reload, a campaign's shared presets
    /// arriving or going): open characters and the GM screen resolve
    /// theirs again, and a hosted campaign sends the GM's changed presets.
    pub(crate) fn follow_settings(&mut self) {
        let rev = self.engine.settings_rev();
        if rev == self.settings_rev {
            return;
        }
        self.settings_rev = rev;
        for v in &mut self.views {
            v.refresh_settings(&self.engine);
        }
        if let Some(gm) = self.gm.as_mut() {
            gm.settings_changed();
        }
    }
}
