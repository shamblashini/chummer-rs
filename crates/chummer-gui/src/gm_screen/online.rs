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

use super::{GmScreen, Live, AUTHOR};
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

    /// The Online section at the top of the feed panel.
    pub(super) fn online_panel(&mut self, ui: &mut egui::Ui, net: &mut Online, engine: &Arc<Engine>, lang: &Language, views: &mut [CharacterView], status: &mut crate::pdf_ui::Status) {
        let serving = self.serving(net);
        ui.horizontal(|ui| {
            ui.label(crate::theme::strong(ui, lang.tr("Online")));
            let mut on = serving;
            let saved = self.path.is_some();
            let r = ui.add_enabled(saved, egui::Checkbox::new(&mut on, lang.tr("Host online")));
            let r = if saved { r.on_hover_text(lang.tr("Players connect to this app; changes sync live")) } else { r.on_disabled_hover_text(lang.tr("Save the campaign to a file first")) };
            if r.changed() {
                if let Err(e) = self.set_hosting(net, engine, views, on) {
                    *status = Some((e, true));
                }
            }
        });
        let Some(o) = &mut self.online else {
            ui.weak(lang.tr("Host the campaign to invite players. Their characters then sync with yours; every change is logged here."));
            return;
        };
        let node = net.node_if_started();
        let connected: Vec<chummer_net::EndpointId> = o.hosted.host.connected().into_iter().map(|(p, _)| p).collect();
        match (&node, serving) {
            (Some(n), true) => {
                let relay = n.home_relay().map(|r| r.to_string()).unwrap_or_else(|| lang.tr("connecting to the relay…"));
                ui.label(RichText::new(format!("● {}", lang.tr("Online"))).color(crate::theme::accent(ui)));
                ui.weak(format!("{} {relay}", lang.tr("Relay:")));
            }
            _ => {
                ui.weak(format!("○ {}", lang.tr("Offline: changes for players wait in the mailbox")));
            }
        }
        ui.horizontal(|ui| {
            if ui.button(lang.tr("Invite player")).on_hover_text(lang.tr("A link for players; it stays valid for the whole group")).clicked() {
                let link = o.hosted.invite(Role::Player, "", node.as_deref());
                o.invite = Some(link.to_string());
            }
            let busy = o.mail.lock().expect("poisoned").busy;
            if ui.add_enabled(!busy, egui::Button::new(lang.tr("Check mail"))).on_hover_text(lang.tr("Collect changes players mailed while you were offline, and mail them yours")).clicked() {
                self.check_mail_later = true;
            }
        });
        if let Some(link) = o.invite.clone() {
            ui.horizontal(|ui| {
                let mut text = link.clone();
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 60.0).font(egui::TextStyle::Monospace));
                if ui.button(lang.tr("Copy")).clicked() {
                    ui.ctx().copy_text(link.clone());
                    *status = Some((lang.tr("Invite link copied."), false));
                }
            });
        }
        {
            let m = o.mail.lock().expect("poisoned");
            if m.busy {
                ui.weak(lang.tr("Checking mail…"));
            } else if let Some((at, r)) = &m.last {
                let when = crate::history_ui::short_time(*at);
                match r {
                    Ok(r) => ui.weak(lang.tr_fmt("Mail at {0}: {1} read, {2} applied, {3} sent", &[&when, &r.fetched, &r.handled, &r.sent])),
                    Err(e) => ui.colored_label(ui.visuals().error_fg_color, format!("{when}: {e}")),
                };
            }
        }
        let members: Vec<(chummer_net::EndpointId, chummer_sync::Member)> = o.hosted.host.authority().members().iter().filter(|(id, _)| **id != o.hosted.host.authority().gm()).map(|(k, v)| (*k, v.clone())).collect();
        if !members.is_empty() {
            ui.label(RichText::new(lang.tr("Players")).strong());
            for (id, m) in members {
                let on = connected.contains(&id);
                let name = if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() };
                let dot = if on { RichText::new("●").color(crate::theme::accent(ui)) } else { RichText::new("○").weak() };
                ui.horizontal(|ui| {
                    ui.label(dot);
                    ui.label(name).on_hover_text(id.to_string());
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
    /// can be) and the GM's own notes, newest first.
    pub(super) fn online_feed(&mut self, ui: &mut egui::Ui, lang: &Language, views: &mut [CharacterView], status: &mut crate::pdf_ui::Status) -> bool {
        let Some(o) = &self.online else { return false };
        enum Row {
            Feed(crate::doc::LogLine, chummer_sync::CharacterId, String),
            Note(String),
        }
        let mut rows: Vec<(i64, Row)> = {
            let a = o.hosted.host.authority();
            a.feed().iter().rev().take(300).map(|f| (f.at, Row::Feed(crate::doc::gm_line(&a, f), f.character.clone(), f.character_name.clone()))).collect()
        };
        rows.extend(self.campaign.log.iter().rev().take(100).filter(|i| !i.author.is_empty() || i.member.is_none()).map(|i| {
            let who = i.member.and_then(|m| self.campaign.member(m)).map(|m| m.name.as_str()).unwrap_or("");
            (i.at, Row::Note(if who.is_empty() || i.description.contains(who) { i.description.clone() } else { format!("{who} {}", i.description) }))
        }));
        rows.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        let mut revert = None;
        egui::ScrollArea::vertical().id_salt("gm_feed_scroll").auto_shrink(false).show(ui, |ui| {
            if rows.is_empty() {
                ui.weak(lang.tr("Changes to the campaign's characters show here."));
            }
            for (at, row) in &rows {
                ui.horizontal_wrapped(|ui| {
                    ui.weak(crate::history_ui::short_time(*at));
                    match row {
                        Row::Feed(l, c, name) => {
                            ui.label(RichText::new(name).strong());
                            if l.refused {
                                ui.colored_label(ui.visuals().error_fg_color, &l.text);
                            } else {
                                ui.label(&l.text);
                            }
                            if let Some(v) = l.revert {
                                if ui.small_button(lang.tr("Revert")).on_hover_text(lang.tr("Take this change back")).clicked() {
                                    revert = Some((c.clone(), v));
                                }
                            }
                        }
                        Row::Note(t) => {
                            ui.label(RichText::new(t).italics());
                        }
                    }
                });
            }
        });
        if let Some((c, v)) = revert {
            match o.hosted.host.gm_revert(&c, v) {
                Ok(r) => {
                    let mut msg = r.applied.accepted.description;
                    if !r.dropped.is_empty() {
                        msg = format!("{msg} — dropped: {}", r.dropped.join("; "));
                    }
                    *status = Some((msg, false));
                    for v in views.iter_mut() {
                        v.doc_mut().refresh();
                    }
                    return true;
                }
                Err(e) => *status = Some((e, true)),
            }
        }
        false
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
