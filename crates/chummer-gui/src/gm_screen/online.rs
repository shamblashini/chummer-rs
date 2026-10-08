//! The GM screen's online side: hosting the campaign, per-player invites
//! and members ("Players & invites"), the authority's activity feed with
//! Revert, and the mailbox.
//!
//! A campaign becomes online the first time the GM hosts it (the
//! authority sidecar is made next to the campaign file, see
//! `chummer_sync::hosted`). From then on every member's character is
//! backed by the authority ([`Backend::Gm`]), hosted or not: the GM's
//! edits are logged and reach players live or through the mailbox.

use std::sync::{Arc, Mutex};

use chummer_core::campaign::MemberId;
use chummer_core::engine::Engine;
use chummer_core::lang::Language;
use chummer_net::invite::{InviteId, Role};
use chummer_net::EndpointId;
use chummer_sync::hosted::{self, HostedCampaign, GM_OWNER};
use chummer_sync::invites::InviteState;
use chummer_sync::MailReport;
use eframe::egui::{self, Color32, RichText};

use super::{FeedRow, GmScreen, Live, AUTHOR};

/// The job making the campaign at `path` online (`bg`).
fn go_online_id(path: &std::path::Path) -> String {
    format!("gm-go-online:{}", path.display())
}

/// What [`GmScreen::go_online`] made on its thread.
type GoneOnline = Result<(HostedCampaign, hosted::Reconciled), String>;

/// A player of an online campaign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayerRow {
    pub name: String,
    /// The endpoint, in full (for the tooltip).
    pub id: String,
    pub connected: bool,
}

/// A mailbox round: (read, applied, sent), or what went wrong.
pub(crate) type MailResult = Result<(usize, usize, usize), String>;

/// The online section, worked out for drawing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OnlineView {
    /// Hosting now.
    pub serving: bool,
    /// The campaign is online (hosted at least once).
    pub online: bool,
    /// While serving: the home relay, once connected.
    pub relay: Option<Option<String>>,
    pub players: Vec<PlayerRow>,
    pub mail_busy: bool,
    /// The last mailbox round: when, and (read, applied, sent) or the error.
    pub mail: Option<(String, MailResult)>,
    /// Every invite and every member added by node id.
    pub invites: Vec<InviteRow>,
    /// The GM's relay mailbox: (messages waiting, puts refused today).
    pub mailbox: Option<(u64, u64)>,
    /// The campaign key's generation (0 until it is first changed).
    pub key_generation: u32,
}

/// How long an unclaimed invite works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Expiry {
    #[default]
    Never,
    Day,
    Week,
    Month,
}

impl Expiry {
    pub const ALL: [Expiry; 4] = [Expiry::Never, Expiry::Day, Expiry::Week, Expiry::Month];

    /// English; goes through `lang.tr`.
    pub fn label(self) -> &'static str {
        match self {
            Expiry::Never => "Until claimed",
            Expiry::Day => "1 day",
            Expiry::Week => "7 days",
            Expiry::Month => "30 days",
        }
    }

    /// Unix seconds when it ends, from `now`.
    pub fn at(self, now: u64) -> Option<u64> {
        let days = match self {
            Expiry::Never => return None,
            Expiry::Day => 1,
            Expiry::Week => 7,
            Expiry::Month => 30,
        };
        Some(now + days * 86_400)
    }
}

/// The New invite form.
#[derive(Debug, Clone, Default)]
pub(crate) struct InviteForm {
    pub label: String,
    /// The character to give the player when they claim the invite.
    pub assign: Option<MemberId>,
    pub expires: Expiry,
}

/// One row of the Players & invites list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKey {
    Invite(InviteId),
    /// A member the GM added by node id (no invite).
    Member(EndpointId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowState {
    Unclaimed { expires: Option<String> },
    Expired,
    /// By which device (short node id, with the player's own name) and when.
    Claimed { by: String, on: String },
    Revoked { on: String },
    /// Added by node id.
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InviteRow {
    pub key: RowKey,
    /// The GM's name for the player ("Anna").
    pub label: String,
    pub state: RowState,
    /// The node id in full (tooltip).
    pub node: Option<String>,
    pub connected: bool,
    pub last_seen: Option<String>,
    /// The characters the player has.
    pub characters: Vec<String>,
    /// The character they get on claiming.
    pub assign: Option<String>,
    /// Mail from them waiting in the GM's mailbox.
    pub waiting: u64,
}

/// A step that needs a yes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    Revoke(InviteId),
    Reissue(InviteId),
    Remove(RowKey),
    /// A new GM campaign key.
    RotateKey,
}

/// What the Players & invites list asked for (done after drawing).
enum InviteDo {
    OpenForm,
    CloseForm,
    Create,
    Copy(InviteId),
    Ask(Confirm),
    Do(Confirm),
    Cancel,
    Dismiss,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn when(secs: u64) -> String {
    crate::history_ui::short_time(secs as i64 * 1000)
}
use crate::doc::{Backend, Doc};
use crate::online::{Online, GM_MAIL_EVERY};
use crate::view::CharacterView;

#[derive(Default)]
struct MailState {
    busy: bool,
    /// When (Unix ms) and how the last round went.
    last: Option<(i64, Result<MailReport, String>)>,
}

pub struct GmOnline {
    pub hosted: HostedCampaign,
    mail: Arc<Mutex<MailState>>,
    mail_loop: Option<tokio::task::AbortHandle>,
    /// The New invite form, while open.
    form: Option<InviteForm>,
    /// The link just made (for whom, the link), with Copy.
    shown: Option<(String, String)>,
    /// A step waiting for a yes.
    confirm: Option<Confirm>,
    /// Members and owners last reconciled.
    signature: Vec<(MemberId, Option<String>)>,
    name: String,
}

impl Drop for GmOnline {
    fn drop(&mut self) {
        if let Some(h) = self.mail_loop.take() {
            h.abort();
        }
        // The last save on another thread (it may compress characters);
        // the app waits for it at exit.
        let host = self.hosted.host.clone();
        if !crate::bg::run(format!("save:authority:{}", self.name), "Saving the campaign's online state…", move || host.save().map_err(|e| e.to_string())) {
            let _ = self.hosted.host.save();
        }
    }
}

impl GmScreen {
    fn signature(&self) -> Vec<(MemberId, Option<String>)> {
        self.campaign.members.iter().map(|m| (m.id, m.owner.clone())).collect()
    }

    pub fn is_online(&self) -> bool {
        self.online.is_some()
    }

    /// Why the campaign could not go online, if it could not.
    #[cfg(test)]
    pub(crate) fn online_error(&self) -> Option<&str> {
        self.online_error.as_deref()
    }

    /// The campaign's online state, once it is online.
    #[cfg(test)]
    pub(crate) fn hosted(&self) -> Option<&HostedCampaign> {
        self.online.as_ref().map(|o| &o.hosted)
    }

    /// Make the campaign online (or reopen its sidecar): its characters
    /// move into the authority and every member's document is backed by it.
    /// Starts making the campaign online on another thread (loading or
    /// making the authority compresses every character: seconds); the
    /// next frames take it in ([`GmScreen::online_tick`]). With `serve`,
    /// hosting starts once it is there.
    pub fn go_online(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &[CharacterView], serve: bool) -> Result<(), String> {
        if self.online.is_some() {
            return Ok(());
        }
        let path = self.path.clone().ok_or_else(|| hosted::HostedError::NoFile.to_string())?;
        if crate::bg::busy(&go_online_id(&path)) {
            self.serve_when_online |= serve;
            return Ok(());
        }
        let secret = net.secret()?;
        let engine = engine.clone();
        let current: std::collections::BTreeMap<MemberId, chummer_core::character::Character> =
            self.campaign.members.iter().filter_map(|m| super::doc_ref(&self.live, views, m.id).filter(|d| !d.is_online()).map(|d| (m.id, d.ch().clone()))).collect();
        let campaign = self.campaign.clone();
        let rt = net.handle();
        crate::bg::run(go_online_id(&path), "Going online…", move || -> GoneOnline {
            let _g = rt.enter();
            let _s = crate::trace::span("go online (HostedCampaign::open)");
            HostedCampaign::open(&campaign, &path, engine, secret, AUTHOR, |m| current.get(&m).cloned()).map_err(|e| e.to_string())
        });
        self.serve_when_online = serve;
        Ok(())
    }

    /// The authority made by [`GmScreen::go_online`], once there (the app
    /// asks every frame, whatever page is in front).
    pub(crate) fn take_online(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &mut [CharacterView]) {
        if self.online.is_some() {
            return;
        }
        let Some(path) = self.path.clone() else { return };
        let Some(r) = crate::bg::take::<GoneOnline>(&go_online_id(&path)) else { return };
        let serve = std::mem::take(&mut self.serve_when_online);
        let (h, rec) = match r {
            Ok(x) => x,
            Err(e) => {
                self.online_error = Some(e);
                return;
            }
        };
        for (m, e) in rec.failed {
            self.errors.insert(m, e);
        }
        h.adopt_owners(&mut self.campaign);
        self.online = Some(GmOnline { hosted: h, mail: Default::default(), mail_loop: None, form: None, shown: None, confirm: None, signature: self.signature(), name: self.campaign.name.clone() });
        crate::trace::time("online docs", || self.online_docs(engine, views));
        if serve {
            if let Err(e) = self.set_hosting(net, engine, views, true) {
                self.online_error = Some(e);
            }
        }
    }

    /// Every member's document backed by the authority.
    fn online_docs(&mut self, engine: &Arc<Engine>, views: &mut [CharacterView]) {
        let Some(o) = &self.online else { return };
        let host = o.hosted.host.clone();
        for m in self.campaign.members.clone() {
            let id = hosted::character_id(m.id);
            let backend = || Backend::Gm { host: host.clone(), id: id.clone() };
            if let Some(v) = views.iter_mut().find(|v| v.campaign_member == Some(m.id)) {
                if !v.doc().is_online() {
                    if let Some(d) = Doc::online(backend(), engine.clone()) {
                        *v.doc_mut() = d;
                    }
                }
                continue;
            }
            match self.live.get_mut(&m.id) {
                Some(l) if l.doc.as_ref().is_some_and(Doc::is_online) => {}
                Some(l) => {
                    if let Some(d) = Doc::online(backend(), engine.clone()) {
                        l.doc = Some(d);
                        l.seen = None;
                    }
                }
                None => {
                    if let Some(d) = Doc::online(backend(), engine.clone()) {
                        let sheet = engine.sheet(&d);
                        self.errors.remove(&m.id);
                        self.live.insert(m.id, Live { doc: Some(d), sheet, seen: None, cursor: Default::default() });
                    }
                }
            }
        }
    }

    /// Each frame: take roster and owner changes into the authority, and
    /// let documents not on screen take what arrived.
    pub(super) fn online_tick(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &mut [CharacterView]) {
        self.take_online(net, engine, views);
        // A claim gave a player a character: the campaign file says so too.
        if let Some(o) = &mut self.online {
            if o.hosted.adopt_owner_changes(&mut self.campaign) {
                self.dirty = true;
                o.signature = self.campaign.members.iter().map(|m| (m.id, m.owner.clone())).collect();
            }
        }
        let sig = self.signature();
        let Some(o) = &mut self.online else { return };
        if sig != o.signature || self.campaign.name != o.name {
            let live = &self.live;
            let current = |m: MemberId| super::doc_ref(live, views, m).filter(|d| !d.is_online()).map(|d| d.ch().clone());
            let rec = o.hosted.reconcile(&self.campaign, current);
            o.signature = sig;
            o.name = self.campaign.name.clone();
            for (m, e) in rec.failed {
                self.errors.insert(m, e);
            }
            self.online_docs(engine, views);
        }
        for l in self.live.values_mut() {
            if let Some(d) = l.doc.as_mut() {
                d.refresh();
            }
        }
        for v in views.iter_mut().filter(|v| v.campaign_member.is_some()) {
            v.doc_mut().refresh();
        }
    }

    pub fn serving(&self, net: &Online) -> bool {
        self.online.is_some() && net.node_if_started().is_some_and(|n| n.serving())
    }

    /// Host online on or off.
    pub fn set_hosting(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &mut [CharacterView], on: bool) -> Result<(), String> {
        if !on {
            self.serve_when_online = false;
            if let Some(n) = net.node_if_started() {
                n.stop_serving();
            }
            if let Some(o) = &mut self.online {
                if let Some(h) = o.mail_loop.take() {
                    h.abort();
                }
            }
            return Ok(());
        }
        let Some(o) = self.online.as_mut() else {
            // Hosting starts once the authority is ready.
            return self.go_online(net, engine, views, true);
        };
        let node = net.node()?;
        node.serve(&o.hosted.host);
        // A mailbox round now and every few minutes.
        let (host, mail) = (o.hosted.clone(), o.mail.clone());
        let task = net.spawn(async move {
            loop {
                mail_round(&node, &host, &mail).await;
                let next = tokio::time::Instant::now() + GM_MAIL_EVERY;
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep_until(next) => break,
                        // A live claim: the relay binds the invite's key to the device now.
                        _ = host.host.mail_keys_changed() => {
                            let _ = node.register_mail_keys(&host.host).await;
                        }
                    }
                }
            }
        });
        if let Some(old) = o.mail_loop.replace(task.abort_handle()) {
            old.abort();
        }
        Ok(())
    }

    fn check_mail(&mut self, net: &mut Online) {
        let Some(o) = &self.online else { return };
        let node = match net.node() {
            Ok(n) => n,
            Err(e) => {
                o.mail.lock().expect("poisoned").last = Some((chummer_core::campaign::now_ms(), Err(e)));
                return;
            }
        };
        let (host, mail) = (o.hosted.clone(), o.mail.clone());
        net.spawn(async move { mail_round(&node, &host, &mail).await });
    }

    /// The campaign closes: stop serving it.
    pub fn close_online(&mut self, net: &mut Online) {
        if let Some(p) = &self.path {
            // Going online still: its authority is dropped when it lands
            // (saved, not served).
            let _ = crate::bg::take::<GoneOnline>(&go_online_id(p));
        }
        if self.online.is_some() {
            if let Some(n) = net.node_if_started() {
                n.stop_serving();
            }
        }
        self.online = None;
    }

    /// What the online section shows.
    pub(crate) fn online_view(&self, net: &Online) -> OnlineView {
        let serving = self.serving(net);
        let Some(o) = &self.online else { return OnlineView { serving, ..Default::default() } };
        let node = net.node_if_started();
        let connected: Vec<chummer_net::EndpointId> = o.hosted.host.connected().into_iter().map(|(p, _)| p).collect();
        let relay = match (&node, serving) {
            (Some(n), true) => Some(n.home_relay().map(|r| r.to_string())),
            _ => None,
        };
        let status = o.hosted.host.mailbox_status();
        let waiting = |k: &chummer_net::PublicKey| status.as_ref().and_then(|s| s.by_key.iter().find(|(x, _)| x == k)).map(|(_, n)| *n).unwrap_or(0);
        let now = now_secs();
        let (players, invites, key_generation) = {
            let a = o.hosted.host.authority();
            let char_name = |c: &chummer_sync::CharacterId| hosted::member_id(c).and_then(|m| self.campaign.member(m)).map(|m| m.name.clone()).unwrap_or_else(|| c.to_string());
            let plays = |node: &EndpointId| a.characters().filter(|c| a.owner(c) == Some(*node)).map(char_name).collect::<Vec<_>>();
            let players: Vec<PlayerRow> = a
                .members()
                .iter()
                .filter(|(id, _)| **id != a.gm())
                .map(|(id, m)| PlayerRow { name: a.invite_of(id).map(|i| i.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() }), id: id.to_string(), connected: connected.contains(id) })
                .collect();
            let mut list: Vec<&chummer_sync::invites::Invite> = a.invites().values().collect();
            list.sort_by_key(|i| (i.created, i.label.clone()));
            let mut rows: Vec<InviteRow> = list
                .into_iter()
                .map(|i| {
                    let node = i.claimed.as_ref().map(|c| c.node);
                    let member = node.and_then(|n| a.members().get(&n));
                    let state = match i.state(now) {
                        InviteState::Unclaimed { expires } => RowState::Unclaimed { expires: expires.map(when) },
                        InviteState::Expired => RowState::Expired,
                        InviteState::Claimed { node, at } => {
                            let name = member.map(|m| m.name.clone()).filter(|n| !n.is_empty() && *n != i.label);
                            RowState::Claimed { by: match name {
                                Some(n) => format!("{n} ({})", node.fmt_short()),
                                None => node.fmt_short().to_string(),
                            }, on: when(at) }
                        }
                        InviteState::Revoked { at } => RowState::Revoked { on: when(at) },
                    };
                    InviteRow {
                        key: RowKey::Invite(i.id),
                        label: i.label.clone(),
                        state,
                        node: node.map(|n| n.to_string()),
                        connected: node.is_some_and(|n| connected.contains(&n)),
                        last_seen: member.and_then(|m| m.last_seen).map(when),
                        characters: node.map(|n| plays(&n)).unwrap_or_default(),
                        assign: i.assign.as_ref().map(char_name),
                        waiting: waiting(&i.key()),
                    }
                })
                .collect();
            rows.extend(a.members().iter().filter(|(id, m)| **id != a.gm() && m.invite.is_none()).map(|(id, m)| InviteRow {
                key: RowKey::Member(*id),
                label: if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() },
                state: RowState::Direct,
                node: Some(id.to_string()),
                connected: connected.contains(id),
                last_seen: m.last_seen.map(when),
                characters: plays(id),
                assign: None,
                waiting: waiting(id),
            }));
            (players, rows, a.key_generation())
        };
        let m = o.mail.lock().expect("poisoned");
        let mail = m.last.as_ref().map(|(at, r)| (crate::history_ui::short_time(*at), r.as_ref().map(|r| (r.fetched, r.handled, r.sent)).map_err(Clone::clone)));
        let mailbox = status.map(|s| (s.waiting, s.refused_today));
        OnlineView { serving, online: true, relay, players, mail_busy: m.busy, mail, invites, mailbox, key_generation }
    }

    /// Opens the New invite form (the Invite buttons).
    pub(crate) fn new_invite(&mut self, _net: &Online) {
        if let Some(o) = &mut self.online {
            o.form.get_or_insert_with(InviteForm::default);
            o.confirm = None;
        }
    }

    /// After an invite changed: the relay learns the new set of keys at
    /// once (a revoked key stops putting mail now).
    fn register_soon(&self, net: &Online) {
        let (Some(o), Some(node)) = (&self.online, net.node_if_started()) else { return };
        if node.mailbox_id().is_none() {
            return;
        }
        let host = o.hosted.host.clone();
        // A failure shows with the next mailbox round, which registers too.
        net.spawn(async move {
            let _ = node.register_mail_keys(&host).await;
        });
    }

    /// Players & invites: one invite (link) per player, with its state and
    /// actions, and the members added by node id. Both layouts; `ws` picks
    /// the Workspace look.
    pub(crate) fn invites_ui(&mut self, ui: &mut egui::Ui, net: &mut Online, lang: &Language, status: &mut crate::pdf_ui::Status, ws: bool) {
        let Some(o) = &self.online else { return };
        let v = self.online_view(net);
        let (form, shown, confirm) = (o.form.clone(), o.shown.clone(), o.confirm);
        let pal = crate::theme::ws(ui);
        let (muted, accent, error, text) = if ws { (pal.muted, pal.accent, pal.error, pal.text) } else { (ui.visuals().weak_text_color(), crate::theme::accent(ui), ui.visuals().error_fg_color, ui.visuals().text_color()) };
        let small = |t: String, c: Color32| RichText::new(t).size(11.5).color(c);
        let button = |ui: &mut egui::Ui, glyph: &str, label: &str| -> egui::Response {
            if ws {
                crate::workspace::widgets::button(ui, Some(glyph), label, crate::workspace::widgets::Look::Ghost, 22.0)
            } else {
                ui.small_button(label)
            }
        };
        let mut todo: Option<InviteDo> = None;
        let mut form = form;

        ui.label(small(lang.tr("Each player gets their own link. The first device that opens it joins as that player; the link then works for nobody else."), muted));
        // New invite, and the campaign key (what the GM's mail to players
        // is signed with) on the same line.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if form.is_none() && button(ui, crate::workspace::icons::USER_PLUS, &lang.tr("New invite…")).on_hover_text(lang.tr("A link for one player")).clicked() {
                todo = Some(InviteDo::OpenForm);
            }
            if confirm != Some(Confirm::RotateKey)
                && button(ui, crate::workspace::icons::KEY, &lang.tr("New campaign key…"))
                    .on_hover_text(lang.tr_fmt("Campaign key: generation {0}. Only needed if the campaign key may be known to someone it should not.", &[&v.key_generation]))
                    .clicked()
            {
                todo = Some(InviteDo::Ask(Confirm::RotateKey));
            }
        });
        if confirm == Some(Confirm::RotateKey) {
            ui.label(small(
                lang.tr_fmt("Make a new campaign key (now generation {0})? Your mail to the players is signed with it. Make a new one only if the current one may be known to someone it should not. Players get the new key with their next sync, live or by mail; until then your mail to them is signed with the old one. Links not used yet keep working; after two new keys in a row, an unused older link only joins live (give that player a New link).", &[&v.key_generation]),
                error,
            ));
            ui.horizontal(|ui| {
                let yes = lang.tr("New campaign key");
                let go = if ws { crate::workspace::widgets::button(ui, None, &yes, crate::workspace::widgets::Look::Primary, 22.0) } else { ui.button(RichText::new(&yes).color(error)) };
                if go.clicked() {
                    todo = Some(InviteDo::Do(Confirm::RotateKey));
                }
                if button(ui, crate::workspace::icons::X, &lang.tr("Cancel")).clicked() {
                    todo = Some(InviteDo::Cancel);
                }
            });
        }
        if let Some(f) = form.as_mut() {
            let frame = if ws { crate::workspace::widgets::card_frame(&pal) } else { egui::Frame::group(ui.style()) };
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("gm_invite_form").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(lang.tr("Player"));
                    let r = ui.add(egui::TextEdit::singleline(&mut f.label).hint_text(lang.tr("Name, e.g. Anna")).desired_width(170.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !f.label.trim().is_empty() {
                        todo = Some(InviteDo::Create);
                    }
                    ui.end_row();
                    ui.label(lang.tr("Character"));
                    let none = lang.tr("None yet");
                    let current = f.assign.and_then(|m| self.campaign.member(m)).map(|m| m.name.clone()).unwrap_or_else(|| none.clone());
                    crate::combo::Combo::from_id_salt("gm_invite_assign").width(170.0).selected_text(current).show_ui(ui, |ui| {
                        if crate::combo::selectable_label(ui, f.assign.is_none(), &none).clicked() {
                            f.assign = None;
                        }
                        for m in &self.campaign.members {
                            let taken = hosted::owner_of(m).is_some();
                            let name = if taken { format!("{} ({})", m.name, lang.tr("played")) } else { m.name.clone() };
                            if crate::combo::selectable_label(ui, f.assign == Some(m.id), name).clicked() {
                                f.assign = Some(m.id);
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(lang.tr("Link works"));
                    crate::combo::Combo::from_id_salt("gm_invite_expiry").width(170.0).selected_text(lang.tr(f.expires.label())).show_ui(ui, |ui| {
                        for e in Expiry::ALL {
                            if crate::combo::selectable_label(ui, f.expires == e, lang.tr(e.label())).clicked() {
                                f.expires = e;
                            }
                        }
                    });
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    let ok = !f.label.trim().is_empty();
                    let create = if ws {
                        ui.add_enabled_ui(ok, |ui| crate::workspace::widgets::button(ui, Some(crate::workspace::icons::LINK), &lang.tr("Create link"), crate::workspace::widgets::Look::Primary, 24.0)).inner
                    } else {
                        ui.add_enabled(ok, egui::Button::new(lang.tr("Create link")))
                    };
                    if create.clicked() {
                        todo = Some(InviteDo::Create);
                    }
                    if button(ui, crate::workspace::icons::X, &lang.tr("Cancel")).clicked() {
                        todo = Some(InviteDo::CloseForm);
                    }
                });
            });
        }
        if let Some((who, link)) = &shown {
            ui.label(small(lang.tr_fmt("Link for {0}: send it to that player only.", &[who]), accent));
            // On its own line, as wide as the panel: the link is long and
            // must not widen the panel.
            let mut t = link.clone();
            // (Less the frame's margins, or the panel grows a little every frame.)
            ui.add(egui::TextEdit::singleline(&mut t).desired_width((ui.available_width() - 16.0).max(60.0)).font(egui::TextStyle::Monospace));
            ui.horizontal(|ui| {
                if button(ui, crate::workspace::icons::COPY, &lang.tr("Copy")).clicked() {
                    ui.ctx().copy_text(link.clone());
                    *status = Some((lang.tr("Invite link copied."), false));
                }
                if button(ui, crate::workspace::icons::X, &lang.tr("Hide")).clicked() {
                    todo = Some(InviteDo::Dismiss);
                }
            });
        }
        if v.invites.is_empty() {
            ui.label(small(lang.tr("No invites yet."), muted));
        }
        for row in &v.invites {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let dot = match (&row.state, row.connected) {
                    (_, true) => accent,
                    (RowState::Revoked { .. } | RowState::Expired, _) => error,
                    _ => muted,
                };
                crate::workspace::widgets::dot(ui, dot, 7.0);
                let name = ui.label(RichText::new(&row.label).strong().color(text));
                if let Some(n) = &row.node {
                    name.on_hover_text(n);
                }
                let (tag, c) = match &row.state {
                    _ if row.connected => (lang.tr("online"), accent),
                    RowState::Unclaimed { .. } => (lang.tr("waiting for the player"), muted),
                    RowState::Expired => (lang.tr("expired"), error),
                    RowState::Claimed { .. } | RowState::Direct => (lang.tr("offline"), muted),
                    RowState::Revoked { .. } => (lang.tr("revoked"), error),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(small(tag, c));
                });
            });
            let mut facts: Vec<String> = Vec::new();
            match &row.state {
                RowState::Unclaimed { expires: Some(e) } => facts.push(lang.tr_fmt("not used yet; expires {0}", &[e])),
                RowState::Unclaimed { expires: None } => facts.push(lang.tr("not used yet")),
                RowState::Expired => facts.push(lang.tr("expired before anyone used it")),
                RowState::Claimed { by, on } => facts.push(lang.tr_fmt("joined from {0} on {1}", &[by, on])),
                RowState::Revoked { on } => facts.push(lang.tr_fmt("revoked {0}", &[on])),
                RowState::Direct => facts.push(lang.tr("added by node id")),
            }
            if let (Some(seen), false) = (&row.last_seen, row.connected) {
                facts.push(lang.tr_fmt("last seen {0}", &[seen]));
            }
            if !row.characters.is_empty() {
                facts.push(lang.tr_fmt("plays {0}", &[&row.characters.join(", ")]));
            } else if let Some(a) = &row.assign {
                facts.push(lang.tr_fmt("gets {0} on joining", &[a]));
            }
            if row.waiting > 0 {
                facts.push(lang.tr_fmt("{0} mailed changes waiting", &[&row.waiting]));
            }
            ui.label(small(facts.join(" · "), muted));
            let here = |c: Confirm| confirm == Some(c);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                match row.key {
                    RowKey::Invite(id) => {
                        let live = !matches!(row.state, RowState::Revoked { .. } | RowState::Expired);
                        if live && button(ui, crate::workspace::icons::COPY, &lang.tr("Copy link")).on_hover_text(lang.tr("The current link of this invite")).clicked() {
                            todo = Some(InviteDo::Copy(id));
                        }
                        if button(ui, crate::workspace::icons::ARROWS_CLOCKWISE, &lang.tr("New link")).on_hover_text(lang.tr("For a new device: the old link and the device that used it stop working")).clicked() {
                            todo = Some(InviteDo::Ask(Confirm::Reissue(id)));
                        }
                        if live && button(ui, crate::workspace::icons::PROHIBIT, &lang.tr("Revoke")).on_hover_text(lang.tr("Cut this player off: their link and device stop working")).clicked() {
                            todo = Some(InviteDo::Ask(Confirm::Revoke(id)));
                        }
                    }
                    RowKey::Member(_) => {}
                }
                let remove = if ws { crate::workspace::widgets::icon_button(ui, crate::workspace::icons::TRASH, 22.0) } else { ui.small_button(lang.tr("Remove")) };
                if remove.on_hover_text(lang.tr("Remove: take this player out of the campaign and the list")).clicked() {
                    todo = Some(InviteDo::Ask(Confirm::Remove(row.key)));
                }
            });
            let asked = match row.key {
                RowKey::Invite(id) => [Confirm::Revoke(id), Confirm::Reissue(id), Confirm::Remove(row.key)].into_iter().find(|c| here(*c)),
                RowKey::Member(_) => here(Confirm::Remove(row.key)).then_some(Confirm::Remove(row.key)),
            };
            if let Some(c) = asked {
                let (q, yes) = match c {
                    Confirm::Revoke(_) => (lang.tr_fmt("Revoke {0}? Their device is cut off at once and the link stops working.", &[&row.label]), lang.tr("Revoke")),
                    Confirm::Reissue(_) => (lang.tr_fmt("Give {0} a new link? The old link and the device that used it stop working; the new device gets their characters.", &[&row.label]), lang.tr("New link")),
                    Confirm::Remove(_) | Confirm::RotateKey => (lang.tr_fmt("Remove {0} from the campaign? Their characters stay; give them to someone else.", &[&row.label]), lang.tr("Remove")),
                };
                ui.label(small(q, error));
                ui.horizontal(|ui| {
                    let go = if ws { crate::workspace::widgets::button(ui, None, &yes, crate::workspace::widgets::Look::Primary, 22.0) } else { ui.button(RichText::new(&yes).color(error)) };
                    // Asked just now: bring the question into view (the
                    // Classic list scrolls).
                    let id = egui::Id::new("gm_invite_confirm_shown");
                    if ui.ctx().data(|d| d.get_temp::<Confirm>(id)) != Some(c) {
                        ui.ctx().data_mut(|d| d.insert_temp(id, c));
                        go.scroll_to_me(Some(egui::Align::Center));
                    }
                    if go.clicked() {
                        todo = Some(InviteDo::Do(c));
                    }
                    if button(ui, crate::workspace::icons::X, &lang.tr("Cancel")).clicked() {
                        todo = Some(InviteDo::Cancel);
                    }
                });
            }
        }
        if let Some((waiting, refused)) = v.mailbox {
            ui.add_space(4.0);
            let mut t = lang.tr_fmt("Mailbox: {0} waiting", &[&waiting]);
            if refused > 0 {
                t = format!("{t} · {}", lang.tr_fmt("{0} refused today (not from your players)", &[&refused]));
            }
            ui.label(small(t, muted));
        }
        // Do it.
        let node = net.node_if_started();
        let Some(o) = self.online.as_mut() else { return };
        o.form = form;
        let mut changed = false;
        match todo {
            None => {}
            Some(InviteDo::OpenForm) => o.form = Some(InviteForm::default()),
            Some(InviteDo::CloseForm) => o.form = None,
            Some(InviteDo::Dismiss) => o.shown = None,
            Some(InviteDo::Cancel) => o.confirm = None,
            Some(InviteDo::Ask(c)) => o.confirm = Some(c),
            Some(InviteDo::Create) => {
                if let Some(f) = o.form.take() {
                    let (_, link) = o.hosted.create_invite(f.label.trim(), f.assign.map(hosted::character_id), f.expires.at(now_secs()), node.as_deref());
                    o.shown = Some((f.label.trim().to_owned(), link.to_string()));
                    changed = true;
                }
            }
            Some(InviteDo::Copy(id)) => {
                if let Some(link) = o.hosted.invite_link(&id, node.as_deref()) {
                    ui.ctx().copy_text(link.to_string());
                    *status = Some((lang.tr("Invite link copied."), false));
                }
            }
            Some(InviteDo::Do(c)) => {
                o.confirm = None;
                let label = |id: &InviteId| o.hosted.host.authority().invite(id).map(|i| i.label.clone()).unwrap_or_default();
                let r = match c {
                    Confirm::Revoke(id) => o.hosted.revoke_invite(&id).map(|_| lang.tr_fmt("Revoked {0}.", &[&label(&id)])),
                    Confirm::Reissue(id) => {
                        let who = label(&id);
                        o.hosted.reissue_invite(&id, None, node.as_deref()).map(|l| {
                            o.shown = Some((who.clone(), l.to_string()));
                            lang.tr_fmt("New link for {0}; the old one no longer works.", &[&who])
                        })
                    }
                    Confirm::Remove(RowKey::Invite(id)) => {
                        let who = label(&id);
                        o.hosted.remove_invite(&id);
                        Ok(lang.tr_fmt("Removed {0}.", &[&who]))
                    }
                    Confirm::Remove(RowKey::Member(p)) => {
                        o.hosted.remove_member(&p);
                        Ok(lang.tr("Removed the player."))
                    }
                    Confirm::RotateKey => {
                        let gen = o.hosted.rotate_campaign_key();
                        Ok(lang.tr_fmt("The campaign key is now generation {0}; players get it with their next sync.", &[&gen]))
                    }
                };
                *status = Some(match r {
                    Ok(m) => (m, false),
                    Err(e) => (e, true),
                });
                changed = true;
            }
        }
        if changed {
            self.register_soon(net);
        }
    }

    /// Check the mailbox on the next frame.
    pub(crate) fn ask_mail(&mut self) {
        self.check_mail_later = true;
    }

    /// Host online on or off; an error goes to the status line.
    pub(crate) fn toggle_hosting(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &mut [CharacterView], on: bool, status: &mut crate::pdf_ui::Status) {
        match self.set_hosting(net, engine, views, on) {
            Ok(()) => self.online_error = None,
            Err(e) => *status = Some((e, true)),
        }
    }

    /// The Online section at the top of the feed panel.
    pub(super) fn online_panel(&mut self, ui: &mut egui::Ui, net: &mut Online, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut crate::pdf_ui::Status) {
        let v = self.online_view(net);
        ui.horizontal(|ui| {
            ui.label(crate::theme::strong(ui, lang.tr("Online")));
            let mut on = v.serving;
            let saved = self.path.is_some();
            let r = ui.add_enabled(saved, egui::Checkbox::new(&mut on, lang.tr("Host online")));
            let r = if saved { r.on_hover_text(lang.tr("Players connect to this app; changes sync live")) } else { r.on_disabled_hover_text(lang.tr("Save the campaign to a file first")) };
            if r.changed() {
                self.toggle_hosting(net, engine, views, on, status);
            }
        });
        if let Some(e) = &self.online_error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if !v.online {
            ui.weak(lang.tr("Host the campaign to invite players. Their characters then sync with yours; every change is logged here."));
            return;
        }
        match &v.relay {
            Some(relay) => {
                let relay = relay.clone().unwrap_or_else(|| lang.tr("connecting to the relay…"));
                ui.label(RichText::new(lang.tr("Online: players can connect")).color(crate::theme::accent(ui)));
                ui.weak(format!("{} {relay}", lang.tr("Relay:")));
            }
            None => {
                ui.weak(lang.tr("Offline: changes for players wait in the mailbox"));
            }
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!v.mail_busy, egui::Button::new(lang.tr("Check mail"))).on_hover_text(lang.tr("Collect changes players mailed while you were offline, and mail them yours")).clicked() {
                self.ask_mail();
            }
        });
        if v.mail_busy {
            ui.weak(lang.tr("Checking mail…"));
        } else if let Some((when, r)) = &v.mail {
            match r {
                Ok((f, h, s)) => ui.weak(lang.tr_fmt("Mail at {0}: {1} read, {2} applied, {3} sent", &[when, f, h, s])),
                Err(e) => ui.colored_label(ui.visuals().error_fg_color, format!("{when}: {e}")),
            };
        }
        egui::CollapsingHeader::new(RichText::new(lang.tr("Players & invites")).strong()).id_salt("gm_players_invites").default_open(true).show(ui, |ui| {
            // Its own scroll area: the activity feed below keeps its room.
            egui::ScrollArea::vertical().id_salt("gm_players_invites_scroll").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                self.invites_ui(ui, net, lang, status, false);
            });
        });
        ui.separator();
    }

    /// The "Played by" row of the member fields (online campaigns).
    pub(super) fn owner_row(&mut self, ui: &mut egui::Ui, lang: &Language, id: MemberId) -> bool {
        let Some(o) = &self.online else { return false };
        let players: Vec<(String, String)> = {
            let a = o.hosted.host.authority();
            a.members().iter().filter(|(_, m)| m.role == Role::Player).map(|(k, m)| (k.to_string(), if m.name.is_empty() { k.fmt_short().to_string() } else { m.name.clone() })).collect()
        };
        let Some(m) = self.campaign.member_mut(id) else { return false };
        let current = hosted::owner_of(m).map(|o| o.to_string());
        let label = current.as_ref().map(|c| players.iter().find(|(k, _)| k == c).map(|(_, n)| n.clone()).unwrap_or_else(|| c[..10].to_owned())).unwrap_or_else(|| lang.tr("GM (not shared)"));
        let mut changed = false;
        ui.label(lang.tr("Played by"));
        crate::combo::Combo::from_id_salt("gm_member_owner").selected_text(label).show_ui(ui, |ui| {
            if crate::combo::selectable_label(ui, current.is_none(), lang.tr("GM (not shared)")).clicked() && current.is_some() {
                m.owner = Some(GM_OWNER.into());
                changed = true;
            }
            for (k, n) in &players {
                if crate::combo::selectable_label(ui, current.as_deref() == Some(k), n).clicked() && current.as_deref() != Some(k) {
                    m.owner = Some(k.clone());
                    changed = true;
                }
            }
        });
        ui.end_row();
        changed
    }

    /// The activity feed of an online campaign: the authority's (every
    /// player's changes and the GM's, with authors; Revert on those that
    /// can be) and the GM's own notes, newest first. `None` offline.
    pub(super) fn online_rows(&self) -> Option<Vec<FeedRow>> {
        let o = self.online.as_ref()?;
        let mut rows: Vec<FeedRow> = {
            let a = o.hosted.host.authority();
            a.feed()
                .iter()
                .rev()
                .take(300)
                .map(|f| {
                    let l = crate::doc::gm_line(&a, f);
                    FeedRow { at: f.at, who: f.character_name.clone(), text: l.text, refused: l.refused, note: false, revert: l.revert.map(|v| (f.character.clone(), v)) }
                })
                .collect()
        };
        rows.extend(self.campaign.log.iter().rev().take(100).filter(|i| !i.author.is_empty() || i.member.is_none()).map(|i| {
            let who = i.member.and_then(|m| self.campaign.member(m)).map(|m| m.name.as_str()).unwrap_or("");
            let text = if who.is_empty() || i.description.contains(who) { i.description.clone() } else { format!("{who} {}", i.description) };
            FeedRow { at: i.at, who: String::new(), text, refused: false, note: true, revert: None }
        }));
        rows.sort_by_key(|r| std::cmp::Reverse(r.at));
        Some(rows)
    }

    /// Revert: take a player's change back through the authority; later
    /// changes are applied again on top. Returns true if it worked.
    pub(crate) fn revert(&mut self, c: &chummer_sync::CharacterId, v: u64, views: &mut [CharacterView], status: &mut crate::pdf_ui::Status) -> bool {
        let Some(o) = &self.online else { return false };
        match o.hosted.host.gm_revert(c, v) {
            Ok(r) => {
                let mut msg = r.applied.accepted.description;
                if !r.dropped.is_empty() {
                    msg = format!("{msg} — dropped: {}", r.dropped.join("; "));
                }
                *status = Some((msg, false));
                for v in views.iter_mut() {
                    v.doc_mut().refresh();
                }
                true
            }
            Err(e) => {
                *status = Some((e, true));
                false
            }
        }
    }

    pub(super) fn take_mail_request(&mut self, net: &mut Online) {
        if std::mem::take(&mut self.check_mail_later) {
            self.check_mail(net);
        }
    }
}

async fn mail_round(node: &chummer_sync::Node, hosted: &HostedCampaign, mail: &Mutex<MailState>) {
    mail.lock().expect("poisoned").busy = true;
    // Invite changes made with `chummer-authority invite ...` meanwhile
    // (reading and saving files: off the async threads).
    let h = hosted.clone();
    let _ = tokio::task::spawn_blocking(move || h.merge_invites()).await;
    let r = node.sync_mail(&hosted.host).await.map_err(|e| e.to_string());
    let mut m = mail.lock().expect("poisoned");
    m.busy = false;
    m.last = Some((chummer_core::campaign::now_ms(), r));
}
