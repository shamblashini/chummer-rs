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
mod rolls;
// The Workspace layout's GM screen; a child module so it can use the
// screen's state.
#[path = "workspace/gm.rs"]
pub(crate) mod workspace;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chummer_core::calc::Sheet;
use chummer_core::campaign::damage::{Attack, Defender, Tracks};
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

pub(crate) use rolls::GmRoll;

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

/// A member as the roster shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RosterRow {
    pub id: MemberId,
    pub name: String,
    /// Player and group.
    pub who: String,
    /// Filled and total boxes of the Physical and Stun tracks, once the
    /// character is loaded.
    pub physical: Option<(i32, i32)>,
    pub stun: Option<(i32, i32)>,
    /// Why the character did not load.
    pub error: Option<String>,
    pub notes: String,
}

impl RosterRow {
    /// "P 3/10  S 0/11".
    pub fn condition(&self) -> String {
        match (self.physical, self.stun) {
            (Some((pf, pcm)), Some((sf, scm))) => format!("P {pf}/{pcm}  S {sf}/{scm}"),
            _ => String::new(),
        }
    }
}

/// Where a name is being edited in place (Workspace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameAt {
    /// The member's roster row.
    Roster(MemberId),
    /// The card's title.
    Card(MemberId),
    /// A combatant's encounter row (or the card of one without a
    /// character).
    Combatant(CombatantId),
}

/// Where the encounter's drop zone leaves its hint for the drag chip.
const DROP_HINT: &str = "gm_encounter_drop_hint";

/// What a roster row asks for (Classic: its menu and buttons).
enum RosterDo {
    Open,
    Join,
    Rename(String),
}

/// The drag payload of a roster member being dragged onto the encounter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DragMember(pub MemberId);

/// A line of the activity feed, newest first.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FeedRow {
    /// Unix ms.
    pub at: i64,
    /// Who or what it is about (shown in bold); may be empty.
    pub who: String,
    pub text: String,
    /// The authority refused the change.
    pub refused: bool,
    /// A note of the GM's (online campaigns show them in italics).
    pub note: bool,
    /// What Revert takes back (online campaigns): the character and the
    /// version.
    pub revert: Option<(chummer_sync::CharacterId, u64)>,
}

/// What the combatant card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Card {
    /// Combatant `i` of the encounter, without a character.
    AdHoc(usize),
    Member(MemberId),
    None,
}

/// The campaign's Save As dialog job (`bg`).
pub const SAVE_DIALOG: &str = "dialog:campaign-save";

/// Linked members a campaign save wrote: (member, file, `Doc::revision`
/// when the copy was taken).
pub type SavedLinked = Vec<(MemberId, PathBuf, u64)>;

/// The file writing of a campaign save ([`GmScreen::save_job`]).
pub type SaveJob = Box<dyn FnOnce() -> Result<SavedLinked, String> + Send>;

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
    /// Which window the dialogs show in (Workspace pop-outs).
    ws_dialogs: crate::workspace::popout::DialogHome,
    /// The GM's dice rolls, newest first.
    rolls: Vec<GmRoll>,
    /// Whose roll the Dice rolls tray shows (`None`: everyone's).
    roll_filter: Option<String>,
    /// The member whose next roll pushes the limit.
    push: Option<MemberId>,
    /// The name being edited in place (Workspace), with the text typed
    /// so far.
    renaming: Option<(RenameAt, String)>,
    /// The Workspace roster's filter.
    filter: String,
    /// The online side, once the campaign was hosted.
    online: Option<online::GmOnline>,
    /// Why an online campaign opened without its online state.
    online_error: Option<String>,
    check_mail_later: bool,
    /// Start hosting once going online is done (`go_online`).
    serve_when_online: bool,
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
            ws_dialogs: Default::default(),
            rolls: Vec::new(),
            roll_filter: None,
            push: None,
            renaming: None,
            filter: String::new(),
            online: None,
            online_error: None,
            check_mail_later: false,
            serve_when_online: false,
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
            if let Err(e) = s.go_online(net, engine, &[], false) {
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
    /// members are saved to their own files. What is quick (taking the
    /// characters into the campaign) happens now; the returned job writes
    /// the files (seconds: the campaign file is compressed) and runs on
    /// another thread. Hand its answer to [`GmScreen::saved`].
    pub fn save_job(&mut self, path: &Path, views: &mut [CharacterView]) -> SaveJob {
        let file = path.to_owned();
        if self.path.as_deref() == Some(path) {
            if let Some(o) = &self.online {
                let linked = o.hosted.write_back_embedded(&mut self.campaign);
                for m in &self.campaign.members {
                    if let Some(d) = doc_mut(&mut self.live, views, m.id) {
                        d.mark_saved();
                    }
                }
                self.dirty = false;
                let (host, campaign) = (o.hosted.host.clone(), self.campaign.clone());
                return Box::new(move || {
                    chummer_sync::hosted::save_linked(host.engine(), linked)?;
                    host.save().map_err(|e| e.to_string())?;
                    campaign.save(&file).map_err(|e| e.to_string())?;
                    Ok(Vec::new())
                });
            }
        }
        let base = path.parent().map(Path::to_owned);
        let mut linked = Vec::new();
        for m in &mut self.campaign.members {
            let Some(doc) = doc_mut(&mut self.live, views, m.id) else { continue };
            match m.linked_path(base.as_deref()) {
                Some(p) => {
                    if doc.dirty {
                        linked.push((m.id, p.clone(), doc.revision(), doc.save_job(p)));
                    }
                    m.name = doc.display_name();
                }
                None => {
                    m.store_character(doc.ch());
                    doc.mark_saved();
                }
            }
        }
        let campaign = self.campaign.clone();
        self.path = Some(path.to_owned());
        self.dirty = false;
        Box::new(move || {
            let mut saved = Vec::new();
            for (id, p, revision, job) in linked {
                job().map_err(|e| format!("Could not save {}: {e}", p.display()))?;
                saved.push((id, p, revision));
            }
            campaign.save(&file).map_err(|e| e.to_string())?;
            Ok(saved)
        })
    }

    /// A [`GmScreen::save_job`] finished: linked members it saved are no
    /// longer modified; on an error the campaign stays modified.
    pub fn saved(&mut self, views: &mut [CharacterView], r: Result<SavedLinked, String>) -> Result<(), String> {
        match r {
            Ok(saved) => {
                for (id, p, revision) in saved {
                    if let Some(d) = doc_mut(&mut self.live, views, id) {
                        d.saved(&p, revision);
                    }
                }
                Ok(())
            }
            Err(e) => {
                self.dirty = true;
                Err(e)
            }
        }
    }

    /// Where Save writes: the campaign's file, or `None` when a file must
    /// be asked for (the first time, or with `save_as`; see
    /// [`GmScreen::ask_file`]).
    pub fn save_target(&self, save_as: bool) -> Result<Option<PathBuf>, String> {
        if save_as && self.is_online() {
            // The authority file belongs to this file name; a copy under
            // another name would come back as a new campaign.
            return Err("An online campaign keeps its file name. To move it, copy the .chummercampaign and .authority files together.".into());
        }
        Ok(match (&self.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => None,
        })
    }

    /// The Save As dialog, on its own thread: the answer is
    /// `bg::take::<Option<PathBuf>>(SAVE_DIALOG)`.
    pub fn ask_file(&self, ctx: &egui::Context) {
        let name = format!("{}.{}", self.campaign.name, campaign::EXTENSION);
        crate::bg::dialog(ctx, SAVE_DIALOG, move || rfd::FileDialog::new().add_filter("chummer-rs campaign", &[campaign::EXTENSION]).set_file_name(name).save_file());
    }



    /// Recompute sheets and take new log entries into the feed for
    /// members whose character changed.
    fn sync(&mut self, engine: &Engine, views: &[CharacterView]) {
        let GmScreen { live, campaign, rolls, .. } = self;
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
            // A rename (here, in the member's tab or by its player) reaches
            // the roster and the encounters.
            let name = doc.display_name();
            campaign.set_member_name(*id, &name);
            for r in rolls.iter_mut().filter(|r| r.member == Some(*id) && r.who != name) {
                r.who = name.clone();
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
        self.begin_frame(engine, views, net);
        self.take_picked(engine, status);
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

    /// Start of a frame (both layouts): what arrived online, the mailbox,
    /// sheets and the feed of members that changed.
    pub fn begin_frame(&mut self, engine: &Arc<Engine>, views: &mut [CharacterView], net: &mut crate::online::Online) {
        crate::trace::time("gm online tick", || self.online_tick(net, engine, views));
        crate::trace::time("gm mail request", || self.take_mail_request(net));
        crate::trace::time("gm sheets + feed", || self.sync(engine, views));
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
            ui.menu_button(format!("{} {}", crate::theme::glyph(crate::theme::glyph("➕")), lang.tr("Add")), |ui| self.add_menu(ui, engine, lang, views, status));
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
        let rows: BTreeMap<MemberId, RosterRow> = self.roster_rows(views).into_iter().flat_map(|(_, rs)| rs).map(|r| (r.id, r)).collect();
        let joinable: Vec<MemberId> = rows.keys().copied().filter(|id| self.can_join(*id)).collect();
        let loaded: Vec<MemberId> = rows.keys().copied().filter(|id| self.live.contains_key(id)).collect();
        // What the row menu and the row buttons ask for, done after.
        let mut from_menu: Option<(MemberId, RosterDo)> = None;
        let mut from_button: Option<(MemberId, RosterDo)> = None;
        let mut menu = |ui: &mut egui::Ui, key: &str| {
            let Ok(id) = key.parse::<MemberId>() else { return };
            if ui.add_enabled(loaded.contains(&id), egui::Button::new(lang.tr("Open"))).clicked() {
                from_menu = Some((id, RosterDo::Open));
                ui.close();
            }
            if ui.add_enabled(joinable.contains(&id), egui::Button::new(lang.tr("Add to encounter"))).clicked() {
                from_menu = Some((id, RosterDo::Join));
                ui.close();
            }
            ui.menu_button(lang.tr("Rename"), |ui| {
                let buf_id = egui::Id::new(("gm_classic_rename", id));
                let mut text: String = ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| rows.get(&id).map(|r| r.name.clone()).unwrap_or_default());
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(180.0));
                r.request_focus();
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    from_menu = Some((id, RosterDo::Rename(text.clone())));
                    ui.data_mut(|d| d.remove::<String>(buf_id));
                    ui.close();
                } else {
                    ui.data_mut(|d| d.insert_temp(buf_id, text));
                }
            });
        };
        let out = {
            crate::tree_table::TreeTable::new("gm_roster_tree", &headers).selected(selected.as_deref()).drag(true).menu(&mut menu).show(
                ui,
                &roots,
                |n| match n.value.and_then(|id| rows.get(&id)) {
                    None => {
                        let kind = MemberKind::from(n.key.trim_start_matches("kind:").to_owned());
                        crate::tree_table::RowView { cells: vec![format!("{} ({})", lang.tr(kind.plural()), n.children.len())], group: true, ..Default::default() }
                    }
                    Some(r) => {
                        let warning = r.error.clone().map(|e| (e, true));
                        crate::tree_table::RowView { cells: vec![r.name.clone(), r.who.clone(), r.condition()], clickable: true, hover: r.notes.clone(), warning, ..Default::default() }
                    }
                },
                |ui, n| {
                    if let Some(id) = n.value {
                        if joinable.contains(&id) && ui.small_button(crate::workspace::icons::PLUS_CIRCLE).on_hover_text(lang.tr("Add to the encounter (or drag onto it)")).clicked() {
                            from_button = Some((id, RosterDo::Join));
                        }
                        if loaded.contains(&id) && ui.small_button("↗").on_hover_text(lang.tr("Open")).clicked() {
                            from_button = Some((id, RosterDo::Open));
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
        if let Some(id) = out.drag_started.and_then(|k| k.parse::<MemberId>().ok()) {
            egui::DragAndDrop::set_payload(ui.ctx(), DragMember(id));
        }
        self.drag_chip(ui.ctx());
        for (id, d) in [from_menu, from_button].into_iter().flatten() {
            match d {
                RosterDo::Open => *action = Some(Action::Open(id)),
                RosterDo::Join => self.add_to_encounter(id, views, true),
                RosterDo::Rename(name) => {
                    self.rename_member(id, &name, views, status);
                }
            }
        }
        if self.campaign.members.is_empty() {
            ui.weak(lang.tr("No characters yet: use Add to bring in player characters, critters and NPCs."));
        }
        ui.add_space(8.0);
        self.member_fields(ui, engine, lang, views, status, action);
    }

    /// The roster grouped by kind, as both layouts show it.
    pub(crate) fn roster_rows(&self, views: &[CharacterView]) -> Vec<(MemberKind, Vec<RosterRow>)> {
        self.campaign
            .grouped()
            .into_iter()
            .map(|(k, ms)| {
                let rows = ms
                    .into_iter()
                    .map(|m| {
                        let tracks = match (self.live.get(&m.id), doc_ref(&self.live, views, m.id)) {
                            (Some(l), Some(d)) => Some(((chummer_core::play::ai::physical_filled(d), l.sheet.physical_cm), (chummer_core::play::ai::stun_filled(d), l.sheet.stun_cm))),
                            _ => None,
                        };
                        RosterRow {
                            id: m.id,
                            name: m.name.clone(),
                            who: [m.player.as_str(), m.group.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "),
                            physical: tracks.map(|t| t.0),
                            stun: tracks.map(|t| t.1),
                            error: self.errors.get(&m.id).cloned(),
                            notes: m.notes.clone(),
                        }
                    })
                    .collect();
                (k, rows)
            })
            .collect()
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

    /// Add the character files picked in the Add menu, once loaded.
    pub(crate) fn take_picked(&mut self, engine: &Arc<Engine>, status: &mut Status) {
        for link in [false, true] {
            for (p, ch) in crate::campaign_ui::picked_characters(link) {
                self.add_loaded(&p, ch, link, engine, status);
            }
        }
    }

    fn add_loaded(&mut self, path: &Path, ch: Result<Character, String>, link: bool, engine: &Arc<Engine>, status: &mut Status) {
        match ch {
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
                    let cmd = campaign::rename_command(d.ch(), &name);
                    d.run(cmd, status);
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
                self.duplicate(id, engine, views, status);
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

    pub(crate) fn duplicate(&mut self, id: MemberId, engine: &Arc<Engine>, views: &mut [CharacterView], status: &mut Status) {
        let Some(template) = self.campaign.member(id).cloned() else { return };
        let Some(ch) = doc_ref(&self.live, views, id).map(|d| d.ch().clone()) else { return };
        let ids = self.campaign.add_copies(engine, &template, &ch, self.copies);
        for nid in ids {
            if let Ok(c) = self.campaign.member(nid).expect("added").load_character(None) {
                self.insert(nid, c, engine);
            }
        }
        self.number_bare(&template.name, views, status);
        self.dirty = true;
    }

    /// Rename member `id` (its character's name, through a command, so
    /// it is undone, saved and synced as any edit). Returns whether it
    /// was renamed.
    pub(crate) fn rename_member(&mut self, id: MemberId, name: &str, views: &mut [CharacterView], status: &mut Status) -> bool {
        let name = name.trim();
        let Some(d) = doc_mut(&mut self.live, views, id) else { return false };
        if name.is_empty() || d.display_name() == name {
            return false;
        }
        let cmd = campaign::rename_command(d.ch(), name);
        if d.run(cmd, status).is_none() {
            return false;
        }
        let shown = d.display_name();
        self.campaign.set_member_name(id, &shown);
        self.dirty = true;
        true
    }

    /// Start editing a name in place, with the name it has now.
    pub(crate) fn start_rename(&mut self, at: RenameAt) {
        let name = match at {
            RenameAt::Roster(m) | RenameAt::Card(m) => self.campaign.member(m).map(|m| m.name.clone()),
            RenameAt::Combatant(c) => self.campaign.encounters.get(self.encounter).and_then(|e| e.index(c).map(|i| e.combatants[i].name.clone())),
        };
        self.renaming = name.map(|n| (at, n));
    }

    /// The text of the name being edited at `at`, if it is.
    pub(crate) fn renaming_at(&mut self, at: RenameAt) -> Option<&mut String> {
        self.renaming.as_mut().filter(|(a, _)| *a == at).map(|(_, t)| t)
    }

    /// The in-place name editor finished: `commit` (Enter, clicking
    /// away) renames, else (Esc) nothing changes.
    pub(crate) fn finish_rename(&mut self, commit: bool, views: &mut [CharacterView], status: &mut Status) {
        let Some((at, text)) = self.renaming.take() else { return };
        if !commit {
            return;
        }
        match at {
            RenameAt::Roster(m) | RenameAt::Card(m) => {
                self.rename_member(m, &text, views, status);
            }
            RenameAt::Combatant(c) => {
                if let Some(i) = self.campaign.encounters.get(self.encounter).and_then(|e| e.index(c)) {
                    self.rename_combatant(i, &text, views, status);
                }
            }
        }
    }

    /// Rename combatant `i` (one without a character; a member's
    /// combatant takes the member's name).
    pub(crate) fn rename_combatant(&mut self, i: usize, name: &str, views: &mut [CharacterView], status: &mut Status) {
        let Some(c) = self.campaign.encounters.get_mut(self.encounter).and_then(|e| e.combatants.get_mut(i)) else { return };
        match c.member {
            Some(m) => {
                self.rename_member(m, name, views, status);
            }
            None if !name.trim().is_empty() => {
                c.name = name.trim().to_owned();
                self.dirty = true;
            }
            None => {}
        }
    }

    /// Add a new character the GM made (a critter, a PACKS NPC) as a
    /// member, numbered when its name is taken: a second "Ganger" comes
    /// in as "Ganger 2" and the first becomes "Ganger 1". Selects it.
    pub(crate) fn add_numbered(&mut self, kind: MemberKind, ch: Character, engine: &Arc<Engine>, views: &mut [CharacterView], status: &mut Status) -> MemberId {
        let ch = self.unique_character(engine, ch);
        let name = ch.display_name();
        let id = self.add_member(Member::embedded(kind, &ch), ch, engine);
        self.number_bare(&name, views, status);
        id
    }

    /// A new member's character named so it can be told apart: a second
    /// "Ganger" comes in as "Ganger 2" (and the first becomes "Ganger
    /// 1", [`GmScreen::number_bare`]).
    fn unique_character(&mut self, engine: &Engine, mut ch: Character) -> Character {
        let name = ch.display_name();
        let unique = self.campaign.unique_name(&name);
        if unique != name {
            let env = chummer_core::command::Envelope::new(campaign::rename_command(&ch, &unique), self.rng.next_u64(), campaign::now_ms(), AUTHOR);
            // Setting a plain field cannot be refused.
            let _ = chummer_core::command::apply(&mut ch, engine, &env);
        }
        ch
    }

    /// Once numbered copies of `name` are in, the one still named just
    /// its base becomes "base 1" (an NPC, critter or the like in the
    /// campaign file; players and linked files keep their names).
    fn number_bare(&mut self, name: &str, views: &mut [CharacterView], status: &mut Status) {
        let base = campaign::strip_number(name).to_owned();
        let numbered = self.campaign.members.iter().any(|m| m.name.strip_prefix(base.as_str()).and_then(|r| r.strip_prefix(' ')).is_some_and(|n| n.parse::<u32>().is_ok()));
        if let Some(id) = self.campaign.unnumbered(&base).filter(|_| numbered) {
            self.rename_member(id, &format!("{base} 1"), views, status);
        }
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
    /// Members already in it are left out. During a round the member
    /// rolls initiative at once.
    pub(crate) fn add_to_encounter(&mut self, id: MemberId, views: &[CharacterView], select: bool) {
        if !self.can_join(id) {
            return;
        }
        let stats = self.initiative_stats(views, id);
        let name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        let Some(e) = self.campaign.encounters.get_mut(self.encounter) else { return };
        let cid = e.join(Combatant::for_member(id, name), &mut self.rng, stats);
        if select {
            self.combatant = Some(cid);
            self.selected = Some(id);
        }
        self.dirty = true;
    }

    /// The encounter board (`body`) as the target of a roster member
    /// being dragged (both layouts): highlighted, with a line saying what
    /// a drop does; a drop adds the member, as Add to encounter does.
    pub(crate) fn encounter_drop_zone(&mut self, ui: &mut egui::Ui, lang: &Language, views: &mut [CharacterView], body: impl FnOnce(&mut GmScreen, &mut egui::Ui, &mut [CharacterView])) {
        let Some(DragMember(id)) = egui::DragAndDrop::payload::<DragMember>(ui.ctx()).map(|p| *p) else {
            body(self, ui, views);
            return;
        };
        let th = crate::theme::current(ui.ctx());
        let (fill, edge, bad, text) = if th.workspace_layout() { (th.ws.selection, th.ws.primary, th.ws.error, th.ws.text) } else { (th.palette.selection, th.palette.accent, th.palette.bad, th.palette.text) };
        let name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        let hint: Result<String, String> = if self.can_join(id) {
            Ok(lang.tr_fmt("Add {0} to the encounter", &[&name]))
        } else if !self.live.contains_key(&id) {
            Err(lang.tr("The character did not load"))
        } else {
            Err(lang.tr_fmt("{0} is in the encounter already", &[&name]))
        };
        let back = ui.painter().add(egui::Shape::Noop);
        let inner = ui.scope(|ui| {
            body(self, ui, views);
            ui.add_space(4.0);
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover()).0
        });
        let zone = inner.response.rect.expand(4.0);
        let over = ui.rect_contains_pointer(zone);
        let color = if hint.is_ok() { edge } else { bad };
        ui.painter().set(back, egui::Shape::rect_filled(zone, egui::CornerRadius::same(6), if over { fill } else { fill.gamma_multiply(0.4) }));
        ui.painter().rect_stroke(zone, egui::CornerRadius::same(6), egui::Stroke::new(if over { 2.0_f32 } else { 1.0 }, color), egui::StrokeKind::Inside);
        let line = match &hint {
            Ok(t) => format!("{}  {t}", crate::workspace::icons::ARROW_BEND_DOWN_RIGHT),
            Err(t) => format!("{}  {t}", crate::workspace::icons::PROHIBIT),
        };
        ui.painter().text(inner.inner.center(), egui::Align2::CENTER_CENTER, line, egui::FontId::proportional(12.5), if hint.is_ok() { text } else { bad });
        if over {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(DROP_HINT), hint.clone()));
            if hint.is_ok() && ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                self.add_to_encounter(id, views, true);
            }
        }
    }

    /// The dragged member's chip under the pointer, with what a drop on
    /// the encounter would do. Drawn by the roster (both layouts).
    pub(crate) fn drag_chip(&self, ctx: &egui::Context) {
        let Some(DragMember(id)) = egui::DragAndDrop::payload::<DragMember>(ctx).map(|p| *p) else { return };
        let Some(pos) = ctx.pointer_hover_pos() else { return };
        let hint: Option<Result<String, String>> = ctx.data_mut(|d| {
            let id = egui::Id::new(DROP_HINT);
            let hint = d.get_temp(id);
            d.remove::<Result<String, String>>(id);
            hint
        });
        let name = self.campaign.member(id).map(|m| m.name.clone()).unwrap_or_default();
        crate::workspace::table::drag_chip(ctx, &crate::theme::current(ctx).ws, pos, &name, hint.as_ref());
    }

    /// Whether member `id` can join the current encounter: loaded and not
    /// in it yet.
    pub(crate) fn can_join(&self, id: MemberId) -> bool {
        self.live.contains_key(&id) && self.campaign.encounters.get(self.encounter).is_some_and(|e| !e.has_member(id))
    }

    /// Add every member of `kind` not in the encounter yet; returns how
    /// many joined.
    pub(crate) fn add_kind_to_encounter(&mut self, kind: &MemberKind, views: &[CharacterView]) -> usize {
        let ids: Vec<MemberId> = self.campaign.members.iter().filter(|m| m.kind == *kind).map(|m| m.id).filter(|id| self.can_join(*id)).collect();
        for id in &ids {
            self.add_to_encounter(*id, views, false);
        }
        ids.len()
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
        if let Some((newest, older)) = self.rolls.split_first() {
            ui.label(RichText::new(lang.tr("Dice rolls")).color(crate::theme::accent(ui)));
            rolls::classic_roll(ui, lang, newest);
            for r in older.iter().take(4) {
                ui.label(RichText::new(r.line(lang)).monospace().size(11.5));
            }
            ui.separator();
        }
        let rows = self.feed_rows();
        let mut revert = None;
        egui::ScrollArea::vertical().id_salt("gm_feed_scroll").auto_shrink(false).show(ui, |ui| {
            if rows.is_empty() {
                ui.weak(lang.tr("Changes to the campaign's characters show here."));
            }
            for r in &rows {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(crate::history_ui::short_time(r.at));
                    if r.note {
                        ui.label(RichText::new(&r.text).italics());
                        return;
                    }
                    if !r.who.is_empty() {
                        ui.label(RichText::new(&r.who).strong());
                    }
                    if r.refused {
                        ui.colored_label(ui.visuals().error_fg_color, &r.text);
                    } else {
                        ui.label(&r.text);
                    }
                    if let Some(v) = &r.revert {
                        if ui.small_button(lang.tr("Revert")).on_hover_text(lang.tr("Take this change back")).clicked() {
                            revert = Some(v.clone());
                        }
                    }
                });
            }
        });
        if let Some((c, v)) = revert {
            self.revert(&c, v, views, status);
        }
    }

    /// The activity feed, newest first: an online campaign's is the
    /// authority's with the GM's notes; otherwise the campaign's log.
    pub(crate) fn feed_rows(&self) -> Vec<FeedRow> {
        if let Some(rows) = self.online_rows() {
            return rows;
        }
        self.campaign
            .log
            .iter()
            .rev()
            .take(300)
            .map(|item| {
                let who = item.member.and_then(|m| self.campaign.member(m)).map(|m| m.name.as_str()).unwrap_or("");
                let who = if !who.is_empty() && !item.description.contains(who) { who.to_owned() } else { String::new() };
                FeedRow { at: item.at, who, text: item.description.clone(), refused: false, note: false, revert: None }
            })
            .collect()
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
        self.encounter_drop_zone(ui, lang, views, |gm, ui, views| gm.initiative_table(ui, lang, views));
        ui.add_space(8.0);
        ui.separator();
        // The card: the selected combatant, or the selected member.
        match self.card() {
            Card::AdHoc(i) => self.adhoc_card(ui, lang, i),
            Card::Member(m) => self.member_card(ui, engine, lang, views, status, m, action),
            Card::None => {
                ui.weak(lang.tr("Select a combatant or a character to see its condition monitors and dice pools."));
            }
        }
    }

    /// What the card shows: the selected combatant, or the selected member.
    pub(crate) fn card(&self) -> Card {
        let enc = self.campaign.encounters.get(self.encounter);
        let combatant = enc.and_then(|e| self.combatant.and_then(|c| e.index(c)).map(|i| (e.combatants[i].member, i)));
        match combatant {
            Some((None, i)) => Card::AdHoc(i),
            Some((Some(m), _)) => Card::Member(m),
            None => match self.selected {
                Some(m) if self.live.contains_key(&m) => Card::Member(m),
                _ => Card::None,
            },
        }
    }

    /// Start the next combat round: everyone rolls.
    pub(crate) fn roll_initiative(&mut self, views: &[CharacterView]) {
        let stats: BTreeMap<MemberId, InitStats> = self.live.keys().filter_map(|id| Some((*id, self.initiative_stats(views, *id)?))).collect();
        let Some(e) = self.campaign.encounters.get_mut(self.encounter) else { return };
        e.new_round(&mut self.rng, |c| c.member.and_then(|m| stats.get(&m).copied()));
        self.dirty = true;
    }

    /// Add every player not in the encounter yet.
    pub(crate) fn add_all_players(&mut self, views: &[CharacterView]) {
        self.add_kind_to_encounter(&MemberKind::Player, views);
    }

    /// Add the combatant without a character typed in `adhoc`.
    pub(crate) fn add_adhoc(&mut self) {
        let Some(e) = self.campaign.encounters.get_mut(self.encounter) else { return };
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

    /// Spend a member's Edge (Seize the Initiative, Blitz).
    fn spend_edge(&mut self, member: Option<MemberId>, views: &mut [CharacterView]) {
        if let Some(d) = member.and_then(|m| doc_mut(&mut self.live, views, m)) {
            if d.created {
                let _ = d.apply(Command::SpendEdge);
            }
        }
    }

    /// Seize the Initiative for combatant `i` (spends 1 Edge).
    pub(crate) fn seize(&mut self, i: usize, views: &mut [CharacterView]) {
        let e = &mut self.campaign.encounters[self.encounter];
        e.combatants[i].seized = true;
        let m = e.combatants[i].member;
        self.spend_edge(m, views);
        self.dirty = true;
    }

    /// Blitz for combatant `i`: 5d6 (spends 1 Edge).
    pub(crate) fn blitz(&mut self, i: usize, views: &mut [CharacterView]) {
        let e = &mut self.campaign.encounters[self.encounter];
        e.blitz(i, &mut self.rng);
        let m = e.combatants[i].member;
        self.spend_edge(m, views);
        self.dirty = true;
    }

    /// Show combatant `cid` on the card (and its member in the roster).
    pub(crate) fn select_combatant(&mut self, cid: CombatantId, member: Option<MemberId>) {
        self.combatant = Some(cid);
        if member.is_some() {
            self.selected = member;
        }
    }

    pub(crate) fn remove_combatant(&mut self, i: usize) {
        let e = &mut self.campaign.encounters[self.encounter];
        if self.combatant == Some(e.combatants[i].id) {
            self.combatant = None;
        }
        e.combatants.remove(i);
        self.dirty = true;
    }

    fn initiative_controls(&mut self, ui: &mut egui::Ui, lang: &Language, views: &[CharacterView]) {
        let mut roll = false;
        ui.horizontal_wrapped(|ui| {
            if ui.add(crate::theme::primary_button(ui, format!("{} {}", crate::theme::glyph(crate::theme::glyph("🎲")), lang.tr("Roll initiative")))).on_hover_text(lang.tr("Start the next combat round: everyone rolls")).clicked() {
                roll = true;
            }
            let e = &mut self.campaign.encounters[self.encounter];
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
        if roll {
            self.roll_initiative(views);
        }
        ui.horizontal_wrapped(|ui| {
            let players = self.campaign.members.iter().any(|m| m.kind == MemberKind::Player && self.live.contains_key(&m.id));
            if ui.add_enabled(players, egui::Button::new(lang.tr("Add all players"))).clicked() {
                self.add_all_players(views);
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.adhoc.0).hint_text(lang.tr("Name")).desired_width(120.0));
            ui.label(lang.tr("Initiative"));
            ui.add(egui::DragValue::new(&mut self.adhoc.1).range(0..=40));
            ui.label("+");
            ui.add(egui::DragValue::new(&mut self.adhoc.2).range(1..=5).suffix("d6"));
            if ui.add_enabled(!self.adhoc.0.trim().is_empty(), egui::Button::new(lang.tr("Add"))).on_hover_text(lang.tr("A combatant without a character sheet")).clicked() {
                self.add_adhoc();
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
        let mut seize = None;
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
                    seize = Some(i);
                }
                let mut blitzed = c.blitzed;
                if ui.add_enabled(pass > 0, egui::Checkbox::new(&mut blitzed, "")).on_hover_text(lang.tr("Blitz: roll 5d6 (spends 1 Edge)")).changed() && blitzed {
                    blitz = Some(i);
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
        if let Some(i) = seize {
            self.seize(i, views);
        }
        if let Some(i) = blitz {
            self.blitz(i, views);
        }
        if let Some((cid, m)) = select {
            self.select_combatant(cid, m);
        }
        if let Some(i) = remove {
            self.remove_combatant(i);
        }
    }

    /// Damage to combatant `i` without a character: no soak.
    pub(crate) fn damage_adhoc(&mut self, i: usize, a: Attack) {
        let c = &mut self.campaign.encounters[self.encounter].combatants[i];
        let t = c.track.get_or_insert_with(Default::default);
        let tracks = Tracks { physical: t.physical, stun: t.stun, overflow: 0, physical_filled: t.physical_filled, stun_filled: t.stun_filled };
        let r = campaign_ui::resolve(&mut self.rng, a, Defender::default(), tracks, false);
        t.physical_filled = r.physical_filled;
        t.stun_filled = r.stun_filled;
        let line = format!("{} {}", c.name, r.text);
        self.campaign.note(None, AUTHOR, line);
        self.dirty = true;
    }

    /// Damage to member `id`, soaked as its sheet says, in the feed.
    pub(crate) fn damage_member(&mut self, id: MemberId, a: Attack, views: &mut [CharacterView], status: &mut Status) {
        let Some(sheet) = self.live.get(&id).map(|l| l.sheet.clone()) else { return };
        let GmScreen { live, rng, damage, campaign, .. } = self;
        let Some(doc) = doc_mut(live, views, id) else { return };
        let (t, r) = campaign_ui::damage_character(rng, a, doc, &sheet, damage.soak_roll);
        campaign.note(Some(id), AUTHOR, r.text.clone());
        for c in campaign_ui::damage_commands(&t, &r) {
            doc.run(c, status);
        }
    }

    /// Give or take karma or nuyen (the award form) to member `id`.
    pub(crate) fn award_member(&mut self, id: MemberId, gain: bool, views: &mut [CharacterView], status: &mut Status) {
        let cmd = self.award.command(gain);
        let Some(doc) = doc_mut(&mut self.live, views, id) else { return };
        if doc.run(cmd, status).is_some() {
            self.award.note.clear();
        }
    }

    /// Open the custom improvement dialog for member `id`.
    pub(crate) fn add_improvement(&mut self, id: MemberId, engine: &Arc<Engine>, views: &[CharacterView], lang: &Language) {
        let Some(doc) = doc_ref(&self.live, views, id) else { return };
        let store = engine.store_for_character(doc);
        self.improvements.open_create(&store, lang, "GM");
        self.improvement_for = Some(id);
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
        let attack = self.damage.ui(ui, lang);
        if let Some(a) = attack {
            self.damage_adhoc(i, a);
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
        let (edge_left, edge) = self.edge_left(id, views);
        let mut roll = None;
        let GmScreen { live, rolls, push, award, damage, improvements, improvement_for, .. } = self;
        let Some(doc) = doc_mut(live, views, id) else { return };
        let p = crate::theme::palette(ui);
        ui.horizontal(|ui| {
            ui.heading(RichText::new(&name).color(crate::theme::accent(ui)));
            ui.weak(format!("{} · {}", lang.tr(kind.as_str()), if doc.created { lang.tr("Career Mode") } else { lang.tr("Create Mode") }));
            if ui.small_button("↗").on_hover_text(lang.tr("Open")).clicked() {
                *action = Some(Action::Open(id));
            }
        });
        let mut attack = None;
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
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.tr("Dice pools")).strong());
                let mut on = *push == Some(id);
                let tip = lang.tr_fmt("The next roll adds Edge ({0}) with the Rule of Six, and spends 1 Edge", &[&edge]);
                if ui.add_enabled(edge_left > 0 || on, egui::Checkbox::new(&mut on, lang.tr("Push the Limit"))).on_hover_text(tip).changed() {
                    *push = on.then_some(id);
                }
            });
            egui::Grid::new(("gm_pools", id)).num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                for (k, p) in campaign_ui::quick_pools(doc, &sheet, lang, 6).into_iter().enumerate() {
                    if campaign_ui::pool_roll(ui, lang, &p.label, p.pool) {
                        roll = Some((p.label.clone(), p.pool));
                    }
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
                        if campaign_ui::pool_roll(ui, lang, &w.get("name"), st.dice_pool) {
                            roll = Some((w.get("name"), st.dice_pool));
                        }
                        ui.weak(format!("{} AP {}", st.damage, st.ap));
                    });
                }
            }
            // The result, next to the pools it came from.
            if let Some(r) = rolls.iter().find(|r| r.member == Some(id)) {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    rolls::classic_roll(ui, lang, r);
                });
            }
            ui.add_space(6.0);
            ui.label(RichText::new(lang.tr("Damage")).strong());
            attack = damage.ui(ui, lang);
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
        if let Some(a) = attack {
            self.damage_member(id, a, views, status);
        }
        if let Some((label, pool)) = roll {
            self.roll_for(Some(id), &name, &label, pool, views);
        }
    }

    // ----- windows -----

    fn windows(&mut self, ctx: &egui::Context, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut Status) {
        if let Some(w) = self.critter.as_mut() {
            match w.show(ctx, engine, lang) {
                crate::gm_ui::CritterResult::Open => {}
                crate::gm_ui::CritterResult::Cancel => self.critter = None,
                crate::gm_ui::CritterResult::Created(ch) => {
                    let kind = if ch.field("metatypecategory").contains("Spirit") { MemberKind::Spirit } else { MemberKind::Critter };
                    self.add_numbered(kind, *ch, engine, views, status);
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
                            self.number_bare(&req.name, views, status);
                            self.dirty = true;
                        } else {
                            self.add_numbered(MemberKind::Npc, ch, engine, views, status);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_feed_encounter_and_card() {
        let Ok(engine) = Engine::load() else { return };
        let engine = Arc::new(engine);
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let ch = Character::load(&p).unwrap();
        let mut gm = GmScreen::new_campaign("Test");
        let mut views: Vec<CharacterView> = Vec::new();
        assert_eq!(gm.card(), Card::None);
        let mut m = Member::embedded(MemberKind::Player, &ch);
        m.player = "Anna".into();
        let id = gm.add_member(m, ch.clone(), &engine);
        let npc = gm.add_member(Member::embedded(MemberKind::Npc, &ch), ch, &engine);
        assert_eq!(gm.card(), Card::Member(npc), "adding selects the member");
        // The roster: grouped by kind, with player and damage.
        doc_mut(&mut gm.live, &mut views, id).unwrap().apply(Command::SetPhysicalDamage { filled: 3 }).unwrap();
        gm.sync(&engine, &views);
        let rows = gm.roster_rows(&views);
        let (kind, players) = &rows[0];
        assert_eq!(*kind, MemberKind::Player);
        let r = &players[0];
        assert_eq!((r.name.as_str(), r.who.as_str()), ("Munin", "Anna"));
        assert_eq!(r.physical.map(|p| p.0), Some(3));
        assert!(r.condition().starts_with("P 3/"));
        // The feed: newest first, the member named once.
        let feed = gm.feed_rows();
        assert_eq!(feed[0].text, "Set physical damage to 3");
        assert_eq!(feed[0].who, "Munin");
        assert!(feed.iter().all(|f| f.revert.is_none()), "no Revert offline");
        // The encounter: players only, then an ad-hoc combatant on the card.
        gm.add_all_players(&views);
        assert_eq!(gm.campaign.encounters[0].combatants.len(), 1);
        gm.roll_initiative(&views);
        assert_eq!(gm.campaign.encounters[0].round, 1);
        assert!(gm.campaign.encounters[0].combatants[0].score > 0);
        gm.adhoc = ("Ganger".into(), 8, 1);
        gm.add_adhoc();
        assert_eq!(gm.card(), Card::AdHoc(1));
        gm.damage_adhoc(1, Attack::parse("4P").unwrap());
        assert_eq!(gm.campaign.encounters[0].combatants[1].track.as_ref().unwrap().physical_filled, 4);
        // Seizing the initiative spends the player's Edge.
        let edge = |gm: &GmScreen| doc_ref(&gm.live, &[], id).unwrap().doc.get_i32("edgeused").unwrap_or(0);
        let before = edge(&gm);
        gm.seize(0, &mut views);
        assert!(gm.campaign.encounters[0].combatants[0].seized);
        assert_eq!(edge(&gm), before + 1);
        gm.remove_combatant(1);
        assert_eq!(gm.card(), Card::Member(npc), "the card falls back to the selected member");
        gm.roll_for(Some(id), "Munin", "Defense", 6, &mut views);
        assert!(gm.rolls[0].line(&Language::default()).starts_with("Munin: Defense 6d6 → "));
        assert_eq!(gm.last_roll(id).map(|r| r.roll.dice.len()), Some(6));
    }

    /// NPCs made from the same kit come in numbered; a rename goes
    /// through the character (undo, the feed) and reaches the encounter;
    /// members join the encounter once, rolling during a round.
    #[test]
    fn numbered_npcs_rename_and_join() {
        let Ok(engine) = Engine::load() else { return };
        let engine = Arc::new(engine);
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let ganger = campaign::named_copy(&engine, &Character::load(&p).unwrap(), "Ganger");
        let mut gm = GmScreen::new_campaign("Test");
        let mut views: Vec<CharacterView> = Vec::new();
        let mut status = None;
        let a = gm.add_numbered(MemberKind::Npc, ganger.clone(), &engine, &mut views, &mut status);
        assert_eq!(gm.campaign.member(a).unwrap().name, "Ganger", "the first keeps the kit's name");
        let b = gm.add_numbered(MemberKind::Npc, ganger.clone(), &engine, &mut views, &mut status);
        let c = gm.add_numbered(MemberKind::Npc, ganger, &engine, &mut views, &mut status);
        gm.sync(&engine, &views);
        let names: Vec<String> = [a, b, c].iter().map(|m| gm.campaign.member(*m).unwrap().name.clone()).collect();
        assert_eq!(names, ["Ganger 1", "Ganger 2", "Ganger 3"]);
        assert_eq!(doc_ref(&gm.live, &views, a).unwrap().display_name(), "Ganger 1", "renumbered on the character");
        // Joining: once each; Add all NPCs adds the rest.
        assert!(gm.can_join(a));
        gm.add_to_encounter(a, &views, true);
        gm.add_to_encounter(a, &views, true);
        assert_eq!(gm.campaign.encounters[0].combatants.len(), 1, "no member twice");
        assert_eq!(gm.card(), Card::Member(a), "a drop or a click shows the card");
        assert!(!gm.can_join(a));
        gm.roll_initiative(&views);
        assert_eq!(gm.add_kind_to_encounter(&MemberKind::Npc, &views), 2);
        assert!(gm.campaign.encounters[0].combatants.iter().all(|c| !c.rolled.is_empty()), "joining during a round rolls");
        // Rename: the character, the roster, the encounter and the feed.
        assert!(gm.rename_member(b, "  Ganger Boss ", &mut views, &mut status));
        assert!(!gm.rename_member(b, "   ", &mut views, &mut status), "no empty names");
        gm.sync(&engine, &views);
        assert_eq!(doc_ref(&gm.live, &views, b).unwrap().display_name(), "Ganger Boss");
        assert_eq!(gm.campaign.member(b).unwrap().name, "Ganger Boss");
        let row = gm.campaign.encounters[0].combatants.iter().find(|x| x.member == Some(b)).unwrap();
        assert_eq!(row.name, "Ganger Boss");
        assert!(gm.feed_rows().iter().any(|f| f.text.contains("Ganger Boss")), "{:?}", gm.feed_rows());
        gm.roll_for(Some(b), "Ganger Boss", "Defense", 4, &mut views);
        // In place: Esc keeps the name, Enter renames.
        gm.start_rename(RenameAt::Roster(c));
        *gm.renaming_at(RenameAt::Roster(c)).unwrap() = "Lookout".into();
        gm.finish_rename(false, &mut views, &mut status);
        assert_eq!(gm.campaign.member(c).unwrap().name, "Ganger 3");
        gm.start_rename(RenameAt::Combatant(gm.campaign.encounters[0].combatants[2].id));
        *gm.renaming.as_mut().map(|(_, t)| t).unwrap() = "Lookout".into();
        gm.finish_rename(true, &mut views, &mut status);
        assert_eq!(gm.campaign.member(c).unwrap().name, "Lookout", "a member's combatant renames the member");
        // Undo takes the rename back everywhere.
        doc_mut(&mut gm.live, &mut views, b).unwrap().undo();
        gm.sync(&engine, &views);
        assert_eq!(gm.campaign.member(b).unwrap().name, "Ganger 2");
        assert!(gm.campaign.encounters[0].combatants.iter().any(|x| x.name == "Ganger 2"));
        assert_eq!(gm.last_roll(b).unwrap().who, "Ganger 2", "its rolls follow the name");
        // Push the Limit: Edge added with the Rule of Six, 1 Edge spent.
        let (left, rating) = gm.edge_left(a, &views);
        assert!(rating > 0 && left > 0);
        gm.push = Some(a);
        gm.roll_for(Some(a), "Ganger 1", "Pistols", 6, &mut views);
        let r = &gm.rolls[0];
        assert_eq!(r.edge, Some(rating));
        assert!(r.roll.dice.len() >= (6 + rating) as usize);
        assert_eq!(gm.push, None, "for one roll");
        assert_eq!(gm.edge_left(a, &views).0, left - 1);
    }

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
        // The files are written by the job (another thread in the app).
        let written = gm.save_job(&path, &mut views)();
        gm.saved(&mut views, written).unwrap();
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
