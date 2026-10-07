//! The GM screen's online side: hosting the campaign, invites, members,
//! the authority's activity feed with Revert, and the mailbox.
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
use chummer_net::invite::Role;
use chummer_sync::hosted::{self, HostedCampaign, GM_OWNER};
use chummer_sync::MailReport;
use eframe::egui::{self, RichText};

use super::{FeedRow, GmScreen, Live, AUTHOR};

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
    pub invite: Option<String>,
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
    invite: Option<String>,
    /// Members and owners last reconciled.
    signature: Vec<(MemberId, Option<String>)>,
    name: String,
}

impl Drop for GmOnline {
    fn drop(&mut self) {
        if let Some(h) = self.mail_loop.take() {
            h.abort();
        }
        let _ = self.hosted.host.save();
    }
}

impl GmScreen {
    fn signature(&self) -> Vec<(MemberId, Option<String>)> {
        self.campaign.members.iter().map(|m| (m.id, m.owner.clone())).collect()
    }

    pub fn is_online(&self) -> bool {
        self.online.is_some()
    }

    /// Make the campaign online (or reopen its sidecar): its characters
    /// move into the authority and every member's document is backed by it.
    pub fn go_online(&mut self, net: &mut Online, engine: &Arc<Engine>, views: &mut [CharacterView]) -> Result<(), String> {
        if self.online.is_some() {
            return Ok(());
        }
        let path = self.path.clone().ok_or_else(|| hosted::HostedError::NoFile.to_string())?;
        let secret = net.secret()?;
        let current: std::collections::BTreeMap<MemberId, chummer_core::character::Character> =
            self.campaign.members.iter().filter_map(|m| super::doc_ref(&self.live, views, m.id).filter(|d| !d.is_online()).map(|d| (m.id, d.ch().clone()))).collect();
        let (h, rec) = {
            let _g = net.enter();
            HostedCampaign::open(&self.campaign, &path, engine.clone(), secret, AUTHOR, |m| current.get(&m).cloned()).map_err(|e| e.to_string())?
        };
        for (m, e) in rec.failed {
            self.errors.insert(m, e);
        }
        h.adopt_owners(&mut self.campaign);
        self.online = Some(GmOnline { hosted: h, mail: Default::default(), mail_loop: None, invite: None, signature: self.signature(), name: self.campaign.name.clone() });
        self.online_docs(engine, views);
        Ok(())
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
    pub(super) fn online_tick(&mut self, engine: &Arc<Engine>, views: &mut [CharacterView]) {
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
        self.go_online(net, engine, views)?;
        let node = net.node()?;
        let o = self.online.as_mut().expect("online");
        node.serve(&o.hosted.host);
        // A mailbox round now and every few minutes.
        let (host, mail) = (o.hosted.host.clone(), o.mail.clone());
        let task = net.spawn(async move {
            loop {
                mail_round(&node, &host, &mail).await;
                tokio::time::sleep(GM_MAIL_EVERY).await;
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
        let (host, mail) = (o.hosted.host.clone(), o.mail.clone());
        net.spawn(async move { mail_round(&node, &host, &mail).await });
    }

    /// The campaign closes: stop serving it.
    pub fn close_online(&mut self, net: &mut Online) {
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
        let players = {
            let a = o.hosted.host.authority();
            a.members()
                .iter()
                .filter(|(id, _)| **id != a.gm())
                .map(|(id, m)| PlayerRow { name: if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() }, id: id.to_string(), connected: connected.contains(id) })
                .collect()
        };
        let m = o.mail.lock().expect("poisoned");
        let mail = m.last.as_ref().map(|(at, r)| (crate::history_ui::short_time(*at), r.as_ref().map(|r| (r.fetched, r.handled, r.sent)).map_err(Clone::clone)));
        OnlineView { serving, online: true, relay, players, mail_busy: m.busy, mail, invite: o.invite.clone() }
    }

    /// A new invite link for players (shown in the online section).
    pub(crate) fn new_invite(&mut self, net: &Online) {
        let node = net.node_if_started();
        if let Some(o) = &mut self.online {
            let link = o.hosted.invite(Role::Player, "", node.as_deref());
            o.invite = Some(link.to_string());
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
            if ui.button(lang.tr("Invite player")).on_hover_text(lang.tr("A link for players; it stays valid for the whole group")).clicked() {
                self.new_invite(net);
            }
            if ui.add_enabled(!v.mail_busy, egui::Button::new(lang.tr("Check mail"))).on_hover_text(lang.tr("Collect changes players mailed while you were offline, and mail them yours")).clicked() {
                self.ask_mail();
            }
        });
        if let Some(link) = v.invite.clone() {
            ui.horizontal(|ui| {
                let mut text = link.clone();
                // A fixed width: the panel must not grow with the link.
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(200.0).font(egui::TextStyle::Monospace));
                if ui.button(lang.tr("Copy")).clicked() {
                    ui.ctx().copy_text(link.clone());
                    *status = Some((lang.tr("Invite link copied."), false));
                }
            });
        }
        if v.mail_busy {
            ui.weak(lang.tr("Checking mail…"));
        } else if let Some((when, r)) = &v.mail {
            match r {
                Ok((f, h, s)) => ui.weak(lang.tr_fmt("Mail at {0}: {1} read, {2} applied, {3} sent", &[when, f, h, s])),
                Err(e) => ui.colored_label(ui.visuals().error_fg_color, format!("{when}: {e}")),
            };
        }
        if !v.players.is_empty() {
            ui.label(RichText::new(lang.tr("Players")).strong());
            for p in &v.players {
                ui.horizontal(|ui| {
                    ui.label(&p.name).on_hover_text(&p.id);
                    if p.connected {
                        ui.label(RichText::new(lang.tr("connected")).color(crate::theme::accent(ui)));
                    } else {
                        ui.weak(lang.tr("not connected"));
                    }
                });
            }
        }
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

    /// Save: the authority's characters go into the campaign file.
    pub(super) fn write_back(&mut self) -> Result<bool, String> {
        let Some(o) = &self.online else { return Ok(false) };
        o.hosted.write_back(&mut self.campaign)?;
        o.hosted.host.save().map_err(|e| e.to_string())?;
        Ok(true)
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

async fn mail_round(node: &chummer_sync::Node, host: &chummer_sync::AuthorityHost, mail: &Mutex<MailState>) {
    mail.lock().expect("poisoned").busy = true;
    let r = node.sync_mail(host).await.map_err(|e| e.to_string());
    let mut m = mail.lock().expect("poisoned");
    m.busy = false;
    m.last = Some((chummer_core::campaign::now_ms(), r));
}
