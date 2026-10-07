//! The GM screen: a campaign as its own MDI tab, next to the character
//! tabs.
//!
//! Left: the roster, members grouped by kind, with quick-add, copies and
//! the selected member's campaign fields. Middle: the encounter board
//! (initiative order and passes) and the selected combatant's card
//! (condition monitors, Edge, armor, dice pools, quick damage, GM awards
//! and overrides). Right: the activity feed and the GM's notes.
//!
//! Every member's character is a [`Doc`], so every change to it is a
//! command with the GM as author, and the feed is filled from the
//! sessions' logs (`Campaign::absorb`). Opening a member as a full tab
//! lends its `Doc` to a [`CharacterView`] (`campaign_member` set) and gets
//! it back when the tab closes, so edits there keep their history and are
//! saved with the campaign.
//!
//! An online campaign (hosted at least once, `online`) backs every
//! member's `Doc` by the campaign authority instead, and the feed is the
//! authority's, with Revert.

mod online;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chummer_core::calc::Sheet;
use chummer_core::campaign::damage::{Defender, Tracks};
use chummer_core::campaign::{self, Campaign, Combatant, CombatantId, Encounter, FeedCursor, InitStats, Member, MemberId, MemberKind};
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::dice::Rng;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_core::play::{matrix, vehicle};
use chummer_core::tree::Node;
use eframe::egui::{self, RichText};

use crate::campaign_ui::{self, AwardForm, DamageForm, KitForm};
use crate::doc::Doc;
use crate::pdf_ui::Status;
use crate::view::{cm_track, CharacterView};

/// The author of the GM's commands.
pub const AUTHOR: &str = "GM";

/// A member's character while the campaign is open.
struct Live {
    /// `None` while a character tab has it.
    doc: Option<Doc>,
    sheet: Sheet,
    /// The session revision `sheet` and the feed were taken at.
    seen: Option<u64>,
    cursor: FeedCursor,
}

/// What the app should do for the GM screen.
pub enum Action {
    /// Open the member as a character tab.
    Open(MemberId),
}

pub struct GmScreen {
    pub campaign: Campaign,
    pub path: Option<PathBuf>,
    /// Campaign-level changes (roster, encounters, notes) not yet saved.
    dirty: bool,
    live: BTreeMap<MemberId, Live>,
    /// Members whose character could not be loaded.
    errors: BTreeMap<MemberId, String>,
    selected: Option<MemberId>,
    encounter: usize,
    combatant: Option<CombatantId>,
    rng: Rng,
    copies: u32,
    confirm_remove: bool,
    adhoc: (String, i32, u32),
    award: AwardForm,
    damage: DamageForm,
    kit: Option<KitForm>,
    critter: Option<crate::gm_ui::CritterWizard>,
    improvements: crate::improvement_ui::ImprovementsPanel,
    /// Member the improvement dialog is for.
    improvement_for: Option<MemberId>,
    /// Recent dice rolls, newest first.
    rolls: Vec<String>,
    /// The online side, once the campaign was hosted.
    online: Option<online::GmOnline>,
    /// Why an online campaign opened without its online state.
    online_error: Option<String>,
    check_mail_later: bool,
}

/// The member's document, wherever it is.
fn doc_mut<'a>(live: &'a mut BTreeMap<MemberId, Live>, views: &'a mut [CharacterView], id: MemberId) -> Option<&'a mut Doc> {
    if let Some(d) = live.get_mut(&id).and_then(|l| l.doc.as_mut()) {
        return Some(d);
    }
    views.iter_mut().find(|v| v.campaign_member == Some(id)).map(CharacterView::doc_mut)
}

fn doc_ref<'a>(live: &'a BTreeMap<MemberId, Live>, views: &'a [CharacterView], id: MemberId) -> Option<&'a Doc> {
    live.get(&id).and_then(|l| l.doc.as_ref()).or_else(|| views.iter().find(|v| v.campaign_member == Some(id)).map(CharacterView::doc))
}

impl GmScreen {
    fn with(campaign: Campaign, path: Option<PathBuf>) -> GmScreen {
        GmScreen {
            campaign,
            path,
            dirty: false,
            live: BTreeMap::new(),
            errors: BTreeMap::new(),
            selected: None,
            encounter: 0,
            combatant: None,
            rng: Rng::from_time(),
            copies: 4,
            confirm_remove: false,
            adhoc: (String::new(), 8, 1),
            award: Default::default(),
            damage: Default::default(),
            kit: None,
            critter: None,
            improvements: Default::default(),
            improvement_for: None,
            rolls: Vec::new(),
            online: None,
            online_error: None,
            check_mail_later: false,
        }
    }

    /// File → New Campaign.
    pub fn new_campaign(name: &str) -> GmScreen {
        let mut c = Campaign::new(name);
        c.encounters.push(Encounter::new("Encounter 1"));
        let mut s = GmScreen::with(c, None);
        s.dirty = true;
        s
    }

    /// File → Open Campaign. Members whose character does not load are
    /// listed with the reason. A campaign that was hosted before comes back
    /// online-backed (not served until the GM hosts it again).
    pub fn open(path: &Path, engine: &Arc<Engine>, net: &mut crate::online::Online) -> Result<GmScreen, String> {
        let mut s = GmScreen::open_local(path, engine)?;
        if chummer_sync::hosted::is_online(path) {
            // Still open the file when its online state cannot be used
            // (another machine's key): the GM can look at it.
            if let Err(e) = s.go_online(net, engine, &mut []) {
                s.online_error = Some(e);
            }
        }
        Ok(s)
    }

    fn open_local(path: &Path, engine: &Arc<Engine>) -> Result<GmScreen, String> {
        let c = Campaign::load(path).map_err(|e| e.to_string())?;
        let mut s = GmScreen::with(c, Some(path.to_owned()));
        let base = path.parent().map(Path::to_owned);
        for m in s.campaign.members.clone() {
            match m.load_character(base.as_deref()) {
                Ok(ch) => s.insert(m.id, ch, engine),
                Err(e) => {
                    s.errors.insert(m.id, e.to_string());
                }
            }
        }
        Ok(s)
    }

    fn insert(&mut self, id: MemberId, ch: Character, engine: &Arc<Engine>) {
        let sheet = engine.sheet(&ch);
        self.live.insert(id, Live { doc: Some(Doc::with_author(ch, engine.clone(), AUTHOR)), sheet, seen: None, cursor: FeedCursor::default() });
    }

    /// Add a member with its character; selects it.
    fn add_member(&mut self, m: Member, ch: Character, engine: &Arc<Engine>) -> MemberId {
        let id = self.campaign.add(m);
        self.insert(id, ch, engine);
        self.select_member(id);
        self.dirty = true;
        id
    }

    pub fn title(&self, views: &[CharacterView]) -> String {
        let name = if self.campaign.name.trim().is_empty() { "GM Screen".to_owned() } else { self.campaign.name.clone() };
        if self.is_dirty(views) {
            format!("{name} •")
        } else {
            name
        }
    }

    pub fn is_dirty(&self, views: &[CharacterView]) -> bool {
        self.dirty || self.campaign.members.iter().any(|m| doc_ref(&self.live, views, m.id).is_some_and(|d| d.dirty))
    }

    /// Hand a member's document to a character tab.
    pub fn lend(&mut self, id: MemberId) -> Option<Doc> {
        self.live.get_mut(&id)?.doc.take()
    }

    /// Take a member's document back from a closed tab.
    pub fn give_back(&mut self, id: MemberId, doc: Doc) {
        if let Some(l) = self.live.get_mut(&id) {
            l.doc = Some(doc);
        }
    }

    /// Save to `path`: embedded members store their character, linked
    /// members are saved to their own files.
    pub fn save_to(&mut self, path: &Path, views: &mut [CharacterView]) -> Result<(), String> {
        if self.path.as_deref() == Some(path) && self.write_back()? {
            for m in &self.campaign.members {
                if let Some(d) = doc_mut(&mut self.live, views, m.id) {
                    d.mark_saved();
                }
            }
            self.campaign.save(path).map_err(|e| e.to_string())?;
            self.dirty = false;
            return Ok(());
        }
        let base = path.parent().map(Path::to_owned);
        for m in &mut self.campaign.members {
            let Some(doc) = doc_mut(&mut self.live, views, m.id) else { continue };
            match m.linked_path(base.as_deref()) {
                Some(p) => {
                    if doc.dirty {
                        doc.save(&p).map_err(|e| format!("Could not save {}: {e}", p.display()))?;
                    }
                    m.name = doc.display_name();
                }
                None => {
                    m.store_character(doc.ch());
                    doc.mark_saved();
                }
            }
        }
        self.campaign.save(path).map_err(|e| e.to_string())?;
        self.path = Some(path.to_owned());
        self.dirty = false;
        Ok(())
    }

    /// Save (asking for a file the first time or with `save_as`). Returns
    /// the file, or `None` when cancelled.
    pub fn save(&mut self, views: &mut [CharacterView], save_as: bool) -> Result<Option<PathBuf>, String> {
        if save_as && self.is_online() {
            // The authority file belongs to this file name; a copy under
            // another name would come back as a new campaign.
            return Err("An online campaign keeps its file name. To move it, copy the .chummercampaign and .authority files together.".into());
        }
        let path = match (&self.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("chummer-rs campaign", &[campaign::EXTENSION]).set_file_name(format!("{}.{}", self.campaign.name, campaign::EXTENSION)).save_file(),
        };
        let Some(path) = path else { return Ok(None) };
        self.save_to(&path, views)?;
        Ok(Some(path))
    }

    /// Recompute sheets and take new log entries into the feed for
    /// members whose character changed.
    fn sync(&mut self, engine: &Engine, views: &[CharacterView]) {
        let GmScreen { live, campaign, .. } = self;
        for (id, l) in live.iter_mut() {
            let doc = match l.doc.as_ref() {
                Some(d) => d,
                None => match views.iter().find(|v| v.campaign_member == Some(*id)) {
                    Some(v) => v.doc(),
                    None => continue,
                },
            };
            let rev = doc.revision();
            if l.seen == Some(rev) {
                continue;
            }
            l.sheet = engine.sheet(doc);
            if let Some(m) = campaign.member_mut(*id) {
                m.name = doc.display_name();
            }
            // An online campaign's feed is the authority's.
            if let Some(s) = doc.session() {
                campaign.absorb(*id, s.log(), &mut l.cursor);
            }
            l.seen = Some(rev);
        }
    }

    fn initiative_stats(&self, views: &[CharacterView], member: MemberId) -> Option<InitStats> {
        let l = self.live.get(&member)?;
        doc_ref(&self.live, views, member)?;
        let s = &l.sheet;
        Some(InitStats { base: s.initiative, dice: s.initiative_dice.max(1) as u32, edge: s.attr("EDG"), reaction: s.attr("REA"), intuition: s.attr("INT") })
    }

    /// The GM screen. `feed` false leaves out the activity panel (the
    /// Workspace shows it in its own window, see [`GmScreen::activity`]).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, net: &mut crate::online::Online, feed: bool) -> Option<Action> {
        self.online_tick(engine, views);
        self.take_mail_request(net);
        self.sync(engine, views);
        let mut action = None;
        egui::SidePanel::left("gm_roster").resizable(true).default_width(340.0).min_width(260.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("gm_roster_scroll").auto_shrink(false).show(ui, |ui| self.roster(ui, engine, lang, views, status, &mut action));
        });
        if feed {
            egui::SidePanel::right("gm_feed").resizable(true).default_width(320.0).min_width(220.0).max_width(520.0).show(ctx, |ui| self.activity(ui, engine, lang, views, status, net));
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("gm_board_scroll").auto_shrink(false).show(ui, |ui| self.board(ui, engine, lang, views, status, &mut action));
        });
        self.windows(ctx, engine, lang, views, status);
        action
    }

    /// The online panel and the activity feed (the right-hand panel).
    pub fn activity(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, net: &mut crate::online::Online) {
        self.online_panel(ui, net, engine, lang, views, status);
        self.feed(ui, lang, views, status);
    }

    // ----- roster -----

    #[allow(clippy::too_many_arguments)]
    fn roster(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, action: &mut Option<Action>) {
        ui.horizontal(|ui| {
            ui.label(crate::theme::strong(ui, lang.tr("Campaign")));
            if ui.add(egui::TextEdit::singleline(&mut self.campaign.name).desired_width(f32::INFINITY)).changed() {
                self.dirty = true;
            }
        });
        if let Some(p) = &self.path {
            let file = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(file).weak()).truncate()).on_hover_text(p.display().to_string());
        }
        ui.horizontal_wrapped(|ui| {
            ui.menu_button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add")), |ui| {
                if ui.button(lang.tr("Character file (copy into the campaign)…")).clicked() {
                    ui.close();
                    for p in campaign_ui::pick_characters() {
                        self.add_file(&p, false, engine, status);
                    }
                }
                if ui.button(lang.tr("Character file (link to the file)…")).clicked() {
                    ui.close();
                    for p in campaign_ui::pick_characters() {
                        self.add_file(&p, true, engine, status);
                    }
                }
                if ui.button(lang.tr("New Critter…")).clicked() {
                    ui.close();
                    self.critter = Some(crate::gm_ui::CritterWizard::new());
                }
                if ui.button(lang.tr("NPC from PACKS Kit…")).clicked() {
                    ui.close();
                    self.kit = Some(KitForm::new(engine));
                }
                let open: Vec<usize> = (0..views.len()).filter(|&i| views[i].campaign_member.is_none()).collect();
                if !open.is_empty() {
                    ui.separator();
                    ui.weak(lang.tr("Open characters:"));
                    for i in open {
                        if ui.button(views[i].ch().display_name()).clicked() {
                            ui.close();
                            self.adopt(&mut views[i]);
                        }
                    }
                }
            });
        });
        ui.separator();

        let headers = [lang.tr("Name"), lang.tr("Player"), lang.tr("Condition")];
        let roots: Vec<Node<Option<MemberId>>> = self
            .campaign
            .grouped()
            .into_iter()
            .map(|(k, ms)| {
                let mut n = Node::new(format!("kind:{}", k.as_str()), None);
                n.children = ms.iter().map(|m| Node::new(m.id.to_string(), Some(m.id))).collect();
                n
            })
            .collect();
        let selected = self.selected.map(|s| s.to_string());
        let out = {
            let live = &self.live;
            let campaign = &self.campaign;
            let errors = &self.errors;
            crate::tree_table::TreeTable::new("gm_roster_tree", &headers).selected(selected.as_deref()).show(
                ui,
                &roots,
                |n| match n.value.and_then(|id| campaign.member(id)) {
                    None => {
                        let kind = MemberKind::from(n.key.trim_start_matches("kind:").to_owned());
                        crate::tree_table::RowView { cells: vec![format!("{} ({})", lang.tr(kind.plural()), n.children.len())], group: true, ..Default::default() }
                    }
                    Some(m) => {
                        let cond = match (live.get(&m.id), doc_ref(live, views, m.id)) {
                            (Some(l), Some(d)) => format!("P {}/{}  S {}/{}", chummer_core::play::ai::physical_filled(d), l.sheet.physical_cm, chummer_core::play::ai::stun_filled(d), l.sheet.stun_cm),
                            _ => String::new(),
                        };
                        let who = [m.player.as_str(), m.group.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                        let warning = errors.get(&m.id).map(|e| (e.clone(), true));
                        crate::tree_table::RowView { cells: vec![m.name.clone(), who, cond], clickable: true, hover: m.notes.clone(), warning, ..Default::default() }
                    }
                },
                |ui, n| {
                    if let Some(id) = n.value {
                        if self.live.contains_key(&id) && ui.small_button("↗").on_hover_text(lang.tr("Open")).clicked() {
                            *action = Some(Action::Open(id));
                        }
                    }
                },
            )
        };
        if let Some(k) = out.clicked {
            if let Ok(id) = k.parse() {
                self.select_member(id);
            }
        }
        if self.campaign.members.is_empty() {
            ui.weak(lang.tr("No characters yet: use Add to bring in player characters, critters and NPCs."));
        }
        ui.add_space(8.0);
        self.member_fields(ui, engine, lang, views, status, action);
    }

    fn select_member(&mut self, id: MemberId) {
        if self.selected != Some(id) {
            self.confirm_remove = false;
        }
        self.selected = Some(id);
        if let Some(e) = self.campaign.encounters.get(self.encounter) {
            self.combatant = e.combatants.iter().find(|c| c.member == Some(id)).map(|c| c.id);
        }
    }

    fn add_file(&mut self, path: &Path, link: bool, engine: &Arc<Engine>, status: &mut Status) {
        match Character::load(path) {
            Ok(ch) => {
                let m = if link { Member::linked(MemberKind::Player, path, &ch) } else { Member::embedded(MemberKind::Player, &ch) };
                self.add_member(m, ch, engine);
            }
            Err(e) => *status = Some((format!("{}: {e}", path.display()), true)),
        }
    }

    /// Make an open character tab a member: the tab keeps editing it.
    fn adopt(&mut self, v: &mut CharacterView) {
        let ch = v.ch();
        let m = match v.path() {
            Some(p) => Member::linked(MemberKind::Player, &p, ch),
            None => Member::embedded(MemberKind::Player, ch),
        };
        let sheet = v.sheet.clone();
        let id = self.campaign.add(m);
        self.live.insert(id, Live { doc: None, sheet, seen: None, cursor: FeedCursor::default() });
        v.campaign_member = Some(id);
        self.select_member(id);
        self.dirty = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn member_fields(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, action: &mut Option<Action>) {
        let Some(id) = self.selected.filter(|id| self.campaign.member(*id).is_some()) else { return };
        ui.separator();
        let mut changed = false;
        let mut name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        egui::Grid::new("gm_member").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            ui.label(lang.tr("Name"));
            let has_doc = doc_ref(&self.live, views, id).is_some();
            if ui.add_enabled(has_doc, egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY)).changed() {
                if let Some(d) = doc_mut(&mut self.live, views, id) {
                    let key = if d.field("alias").trim().is_empty() { "name" } else { "alias" };
                    d.run(Command::SetField { key: key.into(), value: name.clone() }, status);
                }
            }
            ui.end_row();
            let m = self.campaign.member_mut(id).expect("checked");
            ui.label(lang.tr("Type"));
            crate::combo::Combo::from_id_salt("gm_member_kind").selected_text(lang.tr(m.kind.as_str())).show_ui(ui, |ui| {
                for k in MemberKind::ALL {
                    let label = lang.tr(k.as_str());
                    changed |= crate::combo::selectable_value(ui, &mut m.kind, k, label).changed();
                }
            });
            ui.end_row();
            ui.label(lang.tr("Player"));
            changed |= ui.add(egui::TextEdit::singleline(&mut m.player).desired_width(f32::INFINITY)).changed();
            ui.end_row();
            changed |= self.owner_row(ui, lang, id);
            let m = self.campaign.member_mut(id).expect("checked");
            ui.label(lang.tr("Group"));
            changed |= ui.add(egui::TextEdit::singleline(&mut m.group).desired_width(f32::INFINITY)).changed();
            ui.end_row();
            ui.label(lang.tr("Notes"));
            changed |= ui.add(egui::TextEdit::multiline(&mut m.notes).desired_rows(2).desired_width(f32::INFINITY)).changed();
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(&mut m.visible_to_players, lang.tr("Visible to players")).on_hover_text(lang.tr("For online campaigns")).changed();
            ui.end_row();
            ui.label(lang.tr("File"));
            match &m.character {
                campaign::MemberCharacter::Linked { path } => {
                    let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                    ui.add(egui::Label::new(RichText::new(file).weak()).truncate()).on_hover_text(path.display().to_string())
                }
                campaign::MemberCharacter::Embedded { .. } => ui.weak(lang.tr("In the campaign file")),
            };
            ui.end_row();
        });
        if let Some(e) = self.errors.get(&id) {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        self.dirty |= changed;
        let loaded = self.live.contains_key(&id);
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(loaded, egui::Button::new(lang.tr("Open"))).clicked() {
                *action = Some(Action::Open(id));
            }
            let in_enc = self.campaign.encounters.get(self.encounter).is_some_and(|e| e.has_member(id));
            if ui.add_enabled(loaded && !in_enc && !self.campaign.encounters.is_empty(), egui::Button::new(lang.tr("Add to encounter"))).clicked() {
                self.add_to_encounter(id, views, true);
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.copies).range(1..=20).prefix("× "));
            if ui.add_enabled(loaded, egui::Button::new(lang.tr("Duplicate"))).on_hover_text(lang.tr("Copies with new GUIDs and numbered names")).clicked() {
                self.duplicate(id, engine, views);
            }
        });
        ui.horizontal(|ui| {
            if !self.confirm_remove {
                if ui.button(lang.tr("Remove")).clicked() {
                    self.confirm_remove = true;
                }
            } else {
                ui.label(lang.tr("Remove from the campaign?"));
                if ui.button(lang.tr("Yes")).clicked() {
                    self.remove(id, views);
                }
                if ui.button(lang.tr("No")).clicked() {
                    self.confirm_remove = false;
                }
            }
        });
    }

    fn duplicate(&mut self, id: MemberId, engine: &Arc<Engine>, views: &[CharacterView]) {
        let Some(template) = self.campaign.member(id).cloned() else { return };
        let Some(ch) = doc_ref(&self.live, views, id).map(|d| d.ch().clone()) else { return };
        let ids = self.campaign.add_copies(engine, &template, &ch, self.copies);
        for nid in ids {
            if let Ok(c) = self.campaign.member(nid).expect("added").load_character(None) {
                self.insert(nid, c, engine);
            }
        }
        self.dirty = true;
    }

    fn remove(&mut self, id: MemberId, views: &mut [CharacterView]) {
        self.campaign.remove(id);
        self.live.remove(&id);
        self.errors.remove(&id);
        for v in views.iter_mut().filter(|v| v.campaign_member == Some(id)) {
            // The tab stays open as an ordinary character.
            v.campaign_member = None;
        }
        self.selected = None;
        self.confirm_remove = false;
        self.dirty = true;
    }

    /// Add a member to the current encounter; `select` shows its card.
    fn add_to_encounter(&mut self, id: MemberId, views: &[CharacterView], select: bool) {
        let stats = self.initiative_stats(views, id);
        let name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        let Some(e) = self.campaign.encounters.get_mut(self.encounter) else { return };
        let mut c = Combatant::for_member(id, name);
        let cid = c.id;
        if let Some(s) = stats {
            c.base = s.base;
            c.dice = s.dice;
            c.edge = s.edge;
            c.reaction = s.reaction;
            c.intuition = s.intuition;
        }
        e.combatants.push(c);
        if e.round > 0 {
            let i = e.combatants.len() - 1;
            e.reroll(i, &mut self.rng, stats);
        }
        if select {
            self.combatant = Some(cid);
        }
        self.dirty = true;
    }

    // ----- feed -----

    fn feed(&mut self, ui: &mut egui::Ui, lang: &Language, views: &mut [CharacterView], status: &mut Status) {
        egui::TopBottomPanel::bottom("gm_notes").resizable(true).default_height(180.0).show_inside(ui, |ui| {
            ui.label(crate::theme::strong(ui, lang.tr("GM Notes")));
            egui::ScrollArea::vertical().id_salt("gm_notes_scroll").show(ui, |ui| {
                if ui.add(egui::TextEdit::multiline(&mut self.campaign.gm_notes).desired_width(f32::INFINITY).desired_rows(6)).changed() {
                    self.dirty = true;
                }
            });
        });
        ui.label(crate::theme::strong(ui, lang.tr("Activity")));
        if !self.rolls.is_empty() {
            ui.label(RichText::new(lang.tr("Dice rolls")).color(crate::theme::accent(ui)));
            for r in self.rolls.iter().take(5) {
                ui.label(RichText::new(r).monospace().size(11.5));
            }
            ui.separator();
        }
        if self.is_online() {
            self.online_feed(ui, lang, views, status);
            return;
        }
        egui::ScrollArea::vertical().id_salt("gm_feed_scroll").auto_shrink(false).show(ui, |ui| {
            if self.campaign.log.is_empty() {
                ui.weak(lang.tr("Changes to the campaign's characters show here."));
            }
            for item in self.campaign.log.iter().rev().take(300) {
                let when = chummer_core::chargen::iso_from_unix(item.at.div_euclid(1000));
                let who = item.member.and_then(|m| self.campaign.member(m)).map(|m| m.name.as_str()).unwrap_or("");
                ui.horizontal_wrapped(|ui| {
                    ui.weak(when.get(5..16).unwrap_or("").replace('T', " "));
                    if !who.is_empty() && !item.description.contains(who) {
                        ui.label(RichText::new(who).strong());
                    }
                    ui.label(&item.description);
                });
            }
        });
    }

    // ----- encounter board -----

    #[allow(clippy::too_many_arguments)]
    fn board(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, action: &mut Option<Action>) {
        if self.campaign.encounters.is_empty() {
            self.campaign.encounters.push(Encounter::new("Encounter 1"));
        }
        self.encounter = self.encounter.min(self.campaign.encounters.len() - 1);
        ui.horizontal(|ui| {
            ui.label(crate::theme::strong(ui, lang.tr("Encounter")));
            let names: Vec<String> = self.campaign.encounters.iter().map(|e| e.name.clone()).collect();
            crate::combo::Combo::from_id_salt("gm_encounter").selected_text(names[self.encounter].clone()).width(180.0).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    crate::combo::selectable_value(ui, &mut self.encounter, i, n);
                }
            });
            let e = &mut self.campaign.encounters[self.encounter];
            self.dirty |= ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(140.0)).changed();
            if ui.button(lang.tr("New")).clicked() {
                let n = self.campaign.encounters.len() + 1;
                self.campaign.encounters.push(Encounter::new(format!("Encounter {n}")));
                self.encounter = n - 1;
                self.dirty = true;
            }
            if self.campaign.encounters.len() > 1 && ui.button(lang.tr("Delete")).clicked() {
                self.campaign.encounters.remove(self.encounter);
                self.encounter = 0;
                self.dirty = true;
            }
        });
        ui.add_space(4.0);
        self.initiative_controls(ui, lang, views);
        ui.add_space(4.0);
        self.initiative_table(ui, lang, views);
        ui.add_space(8.0);
        ui.separator();
        // The card: the selected combatant, or the selected member.
        let enc = &self.campaign.encounters[self.encounter];
        let card = self.combatant.and_then(|c| enc.index(c)).map(|i| (enc.combatants[i].member, Some(i)));
        match card {
            Some((None, Some(i))) => self.adhoc_card(ui, lang, i),
            Some((Some(m), _)) => self.member_card(ui, engine, lang, views, status, m, action),
            _ => match self.selected {
                Some(m) if self.live.contains_key(&m) => self.member_card(ui, engine, lang, views, status, m, action),
                _ => {
                    ui.weak(lang.tr("Select a combatant or a character to see its condition monitors and dice pools."));
                }
            },
        }
    }

    fn initiative_controls(&mut self, ui: &mut egui::Ui, lang: &Language, views: &[CharacterView]) {
        let stats: BTreeMap<MemberId, InitStats> = self.live.keys().filter_map(|id| Some((*id, self.initiative_stats(views, *id)?))).collect();
        let e = &mut self.campaign.encounters[self.encounter];
        ui.horizontal_wrapped(|ui| {
            if ui.add(crate::theme::primary_button(ui, format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Roll initiative")))).on_hover_text(lang.tr("Start the next combat round: everyone rolls")).clicked() {
                e.new_round(&mut self.rng, |c| c.member.and_then(|m| stats.get(&m).copied()));
                self.dirty = true;
            }
            if ui.add_enabled(e.round > 0 && e.has_next_pass(), egui::Button::new(lang.tr("Next pass"))).on_hover_text(lang.tr("Everyone loses 10")).clicked() {
                e.next_pass();
                self.dirty = true;
            }
            if ui.add_enabled(e.current().is_some(), egui::Button::new(format!("⏭ {}", lang.tr("Next")))).on_hover_text(lang.tr("Mark the current combatant as acted")).clicked() {
                e.advance();
                self.dirty = true;
            }
            if ui.add_enabled(e.round > 0, egui::Button::new(lang.tr("Reset"))).clicked() {
                e.reset();
                self.dirty = true;
            }
            if e.round > 0 {
                ui.label(RichText::new(format!("{}  ·  {}", lang.tr_fmt("Round {0}", &[&e.round]), lang.tr_fmt("Pass {0}", &[&e.pass]))).strong());
            }
        });
        ui.horizontal_wrapped(|ui| {
            let players: Vec<MemberId> = self.campaign.members.iter().filter(|m| m.kind == MemberKind::Player && self.live.contains_key(&m.id)).map(|m| m.id).collect();
            if ui.add_enabled(!players.is_empty(), egui::Button::new(lang.tr("Add all players"))).clicked() {
                for p in players {
                    if !self.campaign.encounters[self.encounter].has_member(p) {
                        self.add_to_encounter(p, views, false);
                    }
                }
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.adhoc.0).hint_text(lang.tr("Name")).desired_width(120.0));
            ui.label(lang.tr("Initiative"));
            ui.add(egui::DragValue::new(&mut self.adhoc.1).range(0..=40));
            ui.label("+");
            ui.add(egui::DragValue::new(&mut self.adhoc.2).range(1..=5).suffix("d6"));
            if ui.add_enabled(!self.adhoc.0.trim().is_empty(), egui::Button::new(lang.tr("Add"))).on_hover_text(lang.tr("A combatant without a character sheet")).clicked() {
                let e = &mut self.campaign.encounters[self.encounter];
                let c = Combatant::ad_hoc(self.adhoc.0.trim(), self.adhoc.1, self.adhoc.2, 10, 10);
                self.combatant = Some(c.id);
                e.combatants.push(c);
                if e.round > 0 {
                    let i = e.combatants.len() - 1;
                    e.reroll(i, &mut self.rng, None);
                }
                self.adhoc.0.clear();
                self.dirty = true;
            }
        });
    }

    fn initiative_table(&mut self, ui: &mut egui::Ui, lang: &Language, views: &mut [CharacterView]) {
        let enc_i = self.encounter;
        let order = self.campaign.encounters[enc_i].order();
        if order.is_empty() {
            ui.weak(lang.tr("Add combatants: characters from the roster, or quick ones by name."));
            return;
        }
        let current = self.campaign.encounters[enc_i].current();
        let accent = crate::theme::accent(ui);
        let mut remove = None;
        let mut select = None;
        let mut edge_spend: Vec<MemberId> = Vec::new();
        let mut blitz = None;
        egui::Grid::new("gm_init").striped(true).num_columns(11).spacing([8.0, 4.0]).show(ui, |ui| {
            for h in lang.tr_all(["", "Score", "Name", "Roll", "Acted", "Delay", "Seize", "Blitz", "", "", ""]) {
                ui.strong(h);
            }
            ui.end_row();
            for i in order {
                let e = &mut self.campaign.encounters[enc_i];
                let pass = e.pass;
                let c = &mut e.combatants[i];
                let selected = self.combatant == Some(c.id);
                ui.label(if current == Some(i) { RichText::new("▶").color(accent) } else { RichText::new("") });
                self.dirty |= ui.add(egui::DragValue::new(&mut c.score).range(-40..=60)).changed();
                let text = RichText::new(&c.name);
                let text = if selected { text } else if !c.in_pass() && pass > 0 { text.weak() } else if current == Some(i) { text.color(accent).strong() } else { text };
                if crate::combo::selectable_label(ui, selected, text).clicked() {
                    select = Some((c.id, c.member));
                }
                if c.rolled.is_empty() {
                    ui.weak(format!("{} + {}d6", c.base, c.dice));
                } else {
                    let dice: Vec<String> = c.rolled.iter().map(u8::to_string).collect();
                    ui.weak(format!("{} + [{}]", c.base, dice.join(" ")));
                }
                self.dirty |= ui.checkbox(&mut c.acted, "").changed();
                self.dirty |= ui.checkbox(&mut c.delayed, "").changed();
                let mut seized = c.seized;
                if ui.checkbox(&mut seized, "").on_hover_text(lang.tr("Seize the Initiative (spends 1 Edge)")).changed() && seized {
                    c.seized = true;
                    edge_spend.extend(c.member);
                    self.dirty = true;
                }
                let mut blitzed = c.blitzed;
                if ui.add_enabled(pass > 0, egui::Checkbox::new(&mut blitzed, "")).on_hover_text(lang.tr("Blitz: roll 5d6 (spends 1 Edge)")).changed() && blitzed {
                    blitz = Some(i);
                    edge_spend.extend(c.member);
                }
                if ui.small_button("−5").on_hover_text(lang.tr("Interrupt action")).clicked() {
                    c.score -= 5;
                    self.dirty = true;
                }
                if ui.small_button("−10").clicked() {
                    c.score -= 10;
                    self.dirty = true;
                }
                if ui.small_button("×").on_hover_text(lang.tr("Remove")).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = blitz {
            self.campaign.encounters[enc_i].blitz(i, &mut self.rng);
            self.dirty = true;
        }
        for m in edge_spend {
            if let Some(d) = doc_mut(&mut self.live, views, m) {
                if d.created {
                    let _ = d.apply(Command::SpendEdge);
                }
            }
        }
        if let Some((cid, m)) = select {
            self.combatant = Some(cid);
            if m.is_some() {
                self.selected = m;
            }
        }
        if let Some(i) = remove {
            let e = &mut self.campaign.encounters[enc_i];
            if self.combatant == Some(e.combatants[i].id) {
                self.combatant = None;
            }
            e.combatants.remove(i);
            self.dirty = true;
        }
    }

    fn adhoc_card(&mut self, ui: &mut egui::Ui, lang: &Language, i: usize) {
        let p = crate::theme::palette(ui);
        let e = &mut self.campaign.encounters[self.encounter];
        let c = &mut e.combatants[i];
        ui.heading(RichText::new(&c.name).color(crate::theme::accent(ui)));
        ui.weak(lang.tr("A combatant without a character sheet"));
        let t = c.track.get_or_insert_with(Default::default);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(lang.tr("Physical"));
            changed |= ui.add(egui::DragValue::new(&mut t.physical).range(1..=30)).changed();
            ui.label(lang.tr("Stun"));
            changed |= ui.add(egui::DragValue::new(&mut t.stun).range(0..=30)).changed();
        });
        ui.columns(2, |cols| {
            cols[0].label(RichText::new(lang.tr("Physical")).strong());
            changed |= cm_track(&mut cols[0], "adhoc_p", t.physical, 3, &mut t.physical_filled, p.physical);
            cols[1].label(RichText::new(lang.tr("Stun")).strong());
            changed |= cm_track(&mut cols[1], "adhoc_s", t.stun, 3, &mut t.stun_filled, p.stun);
        });
        let wm = campaign::damage::wound_modifier(t.physical_filled, t.stun_filled, t.physical, 3);
        ui.label(format!("{} {wm}", lang.tr("CM Penalty:")));
        ui.add_space(4.0);
        ui.label(RichText::new(lang.tr("Damage")).strong());
        if let Some(a) = self.damage.ui(ui, lang) {
            let tracks = Tracks { physical: t.physical, stun: t.stun, overflow: 0, physical_filled: t.physical_filled, stun_filled: t.stun_filled };
            let r = campaign_ui::resolve(&mut self.rng, a, Defender::default(), tracks, false);
            t.physical_filled = r.physical_filled;
            t.stun_filled = r.stun_filled;
            let line = format!("{} {}", c.name, r.text);
            self.campaign.note(None, AUTHOR, line);
            changed = true;
        }
        ui.label(lang.tr("Notes"));
        let c = &mut self.campaign.encounters[self.encounter].combatants[i];
        changed |= ui.add(egui::TextEdit::multiline(&mut c.notes).desired_rows(2).desired_width(f32::INFINITY)).changed();
        self.dirty |= changed;
    }

    #[allow(clippy::too_many_arguments)]
    fn member_card(&mut self, ui: &mut egui::Ui, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status, id: MemberId, action: &mut Option<Action>) {
        let Some(sheet) = self.live.get(&id).map(|l| l.sheet.clone()) else { return };
        let name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        let kind = self.campaign.member(id).map(|m| m.kind.clone()).unwrap_or_default();
        let GmScreen { live, rng, rolls, award, damage, campaign, improvements, improvement_for, .. } = self;
        let Some(doc) = doc_mut(live, views, id) else { return };
        let p = crate::theme::palette(ui);
        ui.horizontal(|ui| {
            ui.heading(RichText::new(&name).color(crate::theme::accent(ui)));
            ui.weak(format!("{} · {}", lang.tr(kind.as_str()), if doc.created { lang.tr("Career Mode") } else { lang.tr("Create Mode") }));
            if ui.small_button("↗").on_hover_text(lang.tr("Open")).clicked() {
                *action = Some(Action::Open(id));
            }
        });
        let roll_line = |rolls: &mut Vec<String>, line: Option<String>| {
            if let Some(l) = line {
                rolls.insert(0, l);
                rolls.truncate(30);
            }
        };
        ui.columns(2, |cols| {
            // Left: condition monitors, Edge, matrix and vehicles.
            let ui = &mut cols[0];
            let (plabel, slabel) = crate::ai_ui::cm_labels(doc, lang);
            let pcm = sheet.physical_cm;
            let thr = sheet.cm_threshold;
            ui.label(RichText::new(plabel).strong());
            let mut pf = chummer_core::play::ai::physical_filled(doc);
            let mut track = pf.min(pcm);
            if cm_track(ui, &format!("gm_pcm{id}"), pcm, thr, &mut track, p.physical) {
                doc.run(Command::SetPhysicalDamage { filled: track }, status);
            }
            if sheet.cm_overflow > 0 {
                // Overflow boxes past the track: no wound modifier.
                let mut over = (pf - pcm).max(0);
                ui.horizontal(|ui| {
                    ui.weak(lang.tr("Overflow"));
                    if cm_track(ui, &format!("gm_ocm{id}"), sheet.cm_overflow, 0, &mut over, p.physical) {
                        pf = pcm + over;
                        doc.run(Command::SetPhysicalDamage { filled: pf }, status);
                    }
                });
            }
            if sheet.stun_cm > 0 {
                ui.label(RichText::new(slabel).strong());
                let mut sf = chummer_core::play::ai::stun_filled(doc);
                if cm_track(ui, &format!("gm_scm{id}"), sheet.stun_cm, if doc.is_ai() { 0 } else { thr }, &mut sf, p.stun) {
                    doc.run(Command::SetStunDamage { filled: sf }, status);
                }
            }
            ui.horizontal(|ui| {
                ui.label(lang.tr("CM Penalty:"));
                let t = RichText::new(sheet.wound_modifier.to_string()).monospace();
                ui.label(if sheet.wound_modifier != 0 { t.color(crate::theme::warn(ui)) } else { t });
                ui.separator();
                ui.label(lang.tr("Armor"));
                ui.label(RichText::new(sheet.armor.to_string()).monospace());
            });
            ui.add_space(4.0);
            crate::play_ui::edge_track(ui, doc, &sheet, lang);
            if let Some(dev) = matrix::active_commlink(doc).cloned() {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{}: {}", lang.tr("Matrix"), dev.get("name"))).strong());
                let mut f = matrix::filled(&dev);
                if cm_track(ui, &format!("gm_mcm{id}"), matrix::condition_monitor(&dev), 0, &mut f, p.matrix) {
                    doc.run(Command::SetMatrixDamage { device: dev.get("guid"), filled: f }, status);
                }
            }
            let vehicles: Vec<_> = doc.items("vehicles", "vehicle").into_iter().take(4).cloned().collect();
            for v in vehicles {
                ui.add_space(4.0);
                ui.label(RichText::new(v.get("name")).strong());
                let mut f = vehicle::filled(&v);
                if cm_track(ui, &format!("gm_vcm{}", v.get("guid")), vehicle::condition_monitor(&v, &Default::default()), 0, &mut f, p.physical) {
                    doc.run(Command::SetVehicleDamage { vehicle: v.get("guid"), filled: f }, status);
                }
            }

            // Right: dice pools, damage, GM tools.
            let ui = &mut cols[1];
            ui.label(RichText::new(lang.tr("Dice pools")).strong());
            let who = name.as_str();
            let rea_int = sheet.attr("REA") + sheet.attr("INT") + sheet.wound_modifier;
            let soak = chummer_core::calc::soak_body(doc, &sheet) + sheet.armor;
            let fixed = [(lang.tr("Defense"), rea_int), (lang.tr("Damage Resistance"), soak), (lang.tr("Composure"), sheet.composure), (lang.tr("Judge Intentions"), sheet.judge_intentions)];
            egui::Grid::new(("gm_pools", id)).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                let mut cells: Vec<(String, i32)> = fixed.into_iter().collect();
                let mut skills: Vec<_> = sheet.skills.iter().filter(|s| s.rating > 0 && !s.disabled).collect();
                skills.sort_by(|a, b| b.pool.cmp(&a.pool).then(a.name.cmp(&b.name)));
                cells.extend(skills.into_iter().take(6).map(|s| (lang.data_name("skills.xml", "", &s.name), s.pool)));
                for (k, (label, pool)) in cells.into_iter().enumerate() {
                    let line = campaign_ui::pool_roll(ui, rng, lang, who, &label, pool);
                    roll_line(rolls, line);
                    if k % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            let weapons: Vec<_> = doc.items("weapons", "weapon").into_iter().take(5).cloned().collect();
            if !weapons.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(lang.tr("Weapons")).strong());
                for w in &weapons {
                    let st = chummer_core::items::weapon::stats(doc, &sheet, w);
                    ui.horizontal(|ui| {
                        let line = campaign_ui::pool_roll(ui, rng, lang, who, &w.get("name"), st.dice_pool);
                        roll_line(rolls, line);
                        ui.weak(format!("{} AP {}", st.damage, st.ap));
                    });
                }
            }
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr("Damage")).strong());
            if let Some(a) = damage.ui(ui, lang) {
                let d = Defender { body: chummer_core::calc::soak_body(doc, &sheet), armor: sheet.armor, bonus: 0 };
                let t = Tracks {
                    physical: sheet.physical_cm,
                    stun: sheet.stun_cm,
                    overflow: sheet.cm_overflow,
                    physical_filled: chummer_core::play::ai::physical_filled(doc),
                    stun_filled: chummer_core::play::ai::stun_filled(doc),
                };
                let r = campaign_ui::resolve(rng, a, d, t, damage.soak_roll);
                campaign.note(Some(id), AUTHOR, r.text.clone());
                if r.physical_filled != t.physical_filled {
                    doc.run(Command::SetPhysicalDamage { filled: r.physical_filled }, status);
                }
                if r.stun_filled != t.stun_filled {
                    doc.run(Command::SetStunDamage { filled: r.stun_filled }, status);
                }
            }
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr("GM award")).strong());
            if let Some(cmd) = award.ui(ui, lang, doc.created) {
                if doc.run(cmd, status).is_some() {
                    award.note.clear();
                }
            }
            if ui.button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add Improvement"))).on_hover_text(lang.tr("A custom improvement: the GM allows it")).clicked() {
                let store = engine.store_for_character(doc);
                improvements.open_create(&store, lang, "GM");
                *improvement_for = Some(id);
            }
        });
    }

    // ----- windows -----

    fn windows(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status) {
        if let Some(w) = self.critter.as_mut() {
            match w.show(ctx, engine, lang) {
                crate::gm_ui::CritterResult::Open => {}
                crate::gm_ui::CritterResult::Cancel => self.critter = None,
                crate::gm_ui::CritterResult::Created(ch) => {
                    let kind = if ch.field("metatypecategory").contains("Spirit") { MemberKind::Spirit } else { MemberKind::Critter };
                    self.add_member(Member::embedded(kind, &ch), *ch, engine);
                    self.critter = None;
                }
            }
        }
        if let Some(k) = self.kit.as_mut() {
            match k.window(ctx, engine, lang) {
                None => {}
                Some(None) => self.kit = None,
                Some(Some(req)) => match campaign::kit_npc(engine, &req.metatype, &req.kit_xml, &req.name) {
                    Ok(ch) => {
                        let mut m = Member::embedded(MemberKind::Npc, &ch);
                        if req.count > 1 {
                            m.name = req.name.clone();
                            let ids = self.campaign.add_copies(engine, &m, &ch, req.count);
                            for nid in &ids {
                                if let Ok(c) = self.campaign.member(*nid).expect("added").load_character(None) {
                                    self.insert(*nid, c, engine);
                                }
                            }
                            if let Some(first) = ids.first() {
                                self.select_member(*first);
                            }
                            self.dirty = true;
                        } else {
                            self.add_member(m, ch, engine);
                        }
                        self.kit = None;
                    }
                    Err(e) => k.fail(e),
                },
            }
        }
        if let Some(id) = self.improvement_for {
            let GmScreen { live, improvements, .. } = self;
            if let Some(doc) = doc_mut(live, views, id) {
                let store = engine.store_for_character(doc);
                let settings = engine.settings.resolve(&doc.field("settings")).cloned();
                improvements.window(ctx, doc, &store, settings.as_ref(), lang);
            } else {
                self.improvement_for = None;
            }
        }
        let _ = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_lent_to_tabs_save_with_the_campaign() {
        let Ok(engine) = Engine::load() else { return };
        let engine = Arc::new(engine);
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let ch = Character::load(&p).unwrap();
        let mut gm = GmScreen::new_campaign("Test");
        let id = gm.add_member(Member::embedded(MemberKind::Player, &ch), ch, &engine);
        // Edit on the GM screen, then in a tab.
        let mut views: Vec<CharacterView> = Vec::new();
        doc_mut(&mut gm.live, &mut views, id).unwrap().apply(Command::SetPhysicalDamage { filled: 2 }).unwrap();
        let mut v = CharacterView::from_doc(gm.lend(id).unwrap(), &engine);
        v.campaign_member = Some(id);
        views.push(v);
        doc_mut(&mut gm.live, &mut views, id).unwrap().apply(Command::SetStunDamage { filled: 4 }).unwrap();
        gm.sync(&engine, &views);
        assert!(gm.campaign.log.iter().any(|l| l.description == "Set stun damage to 4"));
        assert!(gm.is_dirty(&views));
        let dir = std::env::temp_dir().join(format!("chummer-rs-gm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.chummercampaign");
        gm.save_to(&path, &mut views).unwrap();
        assert!(!gm.is_dirty(&views));
        let hash = views[0].doc().session().unwrap().state_hash();
        gm.give_back(id, views.pop().unwrap().into_doc());
        let back = Campaign::load(&path).unwrap();
        let ch = back.member(id).unwrap().load_character(None).unwrap();
        assert_eq!((ch.physical_cm_filled, ch.stun_cm_filled), (2, 4));
        assert_eq!(chummer_core::command::state_hash(&ch), hash);
        assert!(gm.lend(id).is_some(), "the document came back from the tab");
    }
}
