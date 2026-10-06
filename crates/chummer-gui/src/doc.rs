//! An open character as the GUI holds it, and where its changes go.
//!
//! The character is only reachable read-only (`Deref<Target = Character>`).
//! Every change is a [`Command`] sent through [`Doc::apply`], so the GUI
//! cannot change a character any other way. Three backends:
//!
//! - **Local**: a command [`Session`] (a file, or a member of a campaign
//!   that is not online). Undo and redo restore exact earlier states.
//! - **GM**: a character of the campaign authority the GM's app keeps
//!   ([`AuthorityHost`]). Commands go through `gm_edit`: applied at once,
//!   logged with the GM as author and pushed (or mailed) to the owner.
//! - **Player**: the player's copy in a campaign replica
//!   ([`PlayerSession`]). Commands are applied to the copy and queued; the
//!   session sends them when the GM's app is reachable, else mails them.
//!
//! Online characters keep a copy of the backend's state here (the
//! backends sit behind locks) and take the new state when it changes:
//! [`Doc::refresh`] runs every frame and is cheap when nothing changed.
//!
//! Undo and redo are for local characters only. An online change is
//! already logged by the GM's app and may have been sent; taking it back
//! is the GM's "Revert" on the log entry (`Authority::revert`), which the
//! History panel offers on GM characters.

use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use chummer_core::character::Character;
use chummer_core::command::{Command, Rejected, Report, Session};
use chummer_core::engine::Engine;
use chummer_sync::msg::{FeedEntry, Hash};
use chummer_sync::replica::Refused;
use chummer_sync::{AuthorityHost, CharacterId, PlayerSession, SyncMode};

use crate::pdf_ui::Status;

/// Why Undo is off for online characters (a tooltip).
pub const ONLINE_UNDO: &str = "Online character: each change is logged in the campaign at once. The GM can revert a change from the History panel or the GM screen's activity feed.";

/// Where an online character's changes go.
#[derive(Clone)]
pub enum Backend {
    Gm { host: AuthorityHost, id: CharacterId },
    Player { session: PlayerSession, id: CharacterId },
}

struct Online {
    backend: Backend,
    ch: Character,
    /// What the copy was taken at: version and hash (GM), or confirmed
    /// version, hash and outbox length (player).
    seen: (u64, Hash, usize),
    revision: u64,
}

#[allow(clippy::large_enum_variant)] // one per open character; boxing the session buys nothing
enum Kind {
    Local(Session),
    Online(Box<Online>),
}

pub struct Doc {
    kind: Kind,
    engine: Arc<Engine>,
}

impl Deref for Doc {
    type Target = Character;
    fn deref(&self) -> &Character {
        self.ch()
    }
}

/// How an online character stands, for the tab's badge and the History
/// panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub mode: Option<SyncMode>,
    pub pending: usize,
    pub refused: usize,
}

impl SyncState {
    /// A short badge for the tab: ✔ synced, ⟳N pending, ⚠N refused, ⏸
    /// offline.
    pub fn badge(&self) -> String {
        if self.refused > 0 {
            format!("⚠{}", self.refused)
        } else if self.pending > 0 {
            format!("⟳{}", self.pending)
        } else if matches!(self.mode, Some(SyncMode::Offline) | None) {
            "⏸".into()
        } else {
            "✔".into()
        }
    }
}

/// One line of an online character's log.
pub struct LogLine {
    pub at: i64,
    pub text: String,
    /// Set when the GM may revert it: the version that made it.
    pub revert: Option<u64>,
    pub refused: bool,
}

impl Doc {
    pub fn new(ch: Character, engine: Arc<Engine>) -> Doc {
        Doc { kind: Kind::Local(Session::new(ch)), engine }
    }

    /// A character whose commands carry `author` (the campaign's GM).
    pub fn with_author(ch: Character, engine: Arc<Engine>, author: &str) -> Doc {
        Doc { kind: Kind::Local(Session::new(ch).with_author(author)), engine }
    }

    /// A character of an online campaign; `None` when the backend does not
    /// have it.
    pub fn online(backend: Backend, engine: Arc<Engine>) -> Option<Doc> {
        let (ch, seen) = pull(&backend)?;
        Some(Doc { kind: Kind::Online(Box::new(Online { backend, ch, seen, revision: 0 })), engine })
    }

    pub fn ch(&self) -> &Character {
        match &self.kind {
            Kind::Local(s) => s.ch(),
            Kind::Online(o) => &o.ch,
        }
    }

    /// The local session (local characters only).
    pub fn session(&self) -> Option<&Session> {
        match &self.kind {
            Kind::Local(s) => Some(s),
            Kind::Online(_) => None,
        }
    }

    pub fn backend(&self) -> Option<&Backend> {
        match &self.kind {
            Kind::Local(_) => None,
            Kind::Online(o) => Some(&o.backend),
        }
    }

    pub fn is_online(&self) -> bool {
        matches!(self.kind, Kind::Online(_))
    }

    /// A player's character: (campaign id, character id).
    pub fn player_key(&self) -> Option<(String, CharacterId)> {
        match self.backend()? {
            Backend::Player { session, id } => Some((session.link().campaign.to_string(), id.clone())),
            Backend::Gm { .. } => None,
        }
    }

    /// Counts every change, local or arriving from the campaign.
    pub fn revision(&self) -> u64 {
        match &self.kind {
            Kind::Local(s) => s.revision(),
            Kind::Online(o) => o.revision,
        }
    }

    /// Take the backend's newest state, if it changed. Returns whether it
    /// did.
    pub fn refresh(&mut self) -> bool {
        let Kind::Online(o) = &mut self.kind else { return false };
        let now = match &o.backend {
            Backend::Gm { host, id } => {
                let a = host.authority();
                (a.version(id), a.hash(id), 0)
            }
            Backend::Player { session, id } => {
                let r = session.replica();
                (r.version(id), r.confirmed_hash(id), r.outbox(id).len())
            }
        };
        let (Some(v), Some(h), n) = now else { return false };
        if (v, h, n) == o.seen {
            return false;
        }
        let Some((mut ch, seen)) = pull(&o.backend) else { return false };
        ch.file = o.ch.file.clone();
        // A GM's copy is saved into the campaign file; a player's is the
        // campaign's to keep.
        ch.dirty = matches!(o.backend, Backend::Gm { .. });
        o.ch = ch;
        o.seen = seen;
        o.revision += 1;
        true
    }

    /// Run a command; the only way the GUI changes a character.
    pub fn apply(&mut self, cmd: Command) -> Result<Report, Rejected> {
        let engine = self.engine.clone();
        let r = match &mut self.kind {
            Kind::Local(s) => return s.apply(&engine, cmd),
            Kind::Online(o) => match &o.backend {
                Backend::Gm { host, id } => {
                    let a = host.gm_edit(id, cmd)?;
                    Report { description: a.accepted.description, message: None, count: None, changed: a.accepted.changed }
                }
                Backend::Player { session, id } => session.edit_now(id, cmd)?,
            },
        };
        self.refresh();
        Ok(r)
    }

    /// Run a command; a rejection goes to the status bar. Returns the
    /// report when it ran.
    pub fn run(&mut self, cmd: Command, status: &mut Status) -> Option<Report> {
        match self.apply(cmd) {
            Ok(r) => Some(r),
            Err(e) => {
                *status = Some((e.reason, true));
                None
            }
        }
    }

    /// Run a command whose rejection needs no message (the control was
    /// only enabled when it could run). Returns whether it changed
    /// anything.
    pub fn set(&mut self, cmd: Command) -> bool {
        self.apply(cmd).is_ok_and(|r| r.changed)
    }

    pub fn can_undo(&self) -> bool {
        self.session().is_some_and(Session::can_undo)
    }

    pub fn can_redo(&self) -> bool {
        self.session().is_some_and(Session::can_redo)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.session()?.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.session()?.redo_label()
    }

    pub fn undo(&mut self) -> Option<String> {
        match &mut self.kind {
            Kind::Local(s) => s.undo(),
            Kind::Online(_) => None,
        }
    }

    pub fn redo(&mut self) -> Option<String> {
        match &mut self.kind {
            Kind::Local(s) => s.redo(),
            Kind::Online(_) => None,
        }
    }

    /// Saved inside a campaign file: no longer modified.
    pub fn mark_saved(&mut self) {
        match &mut self.kind {
            Kind::Local(s) => s.mark_saved(),
            Kind::Online(o) => o.ch.dirty = false,
        }
    }

    /// Save to `path` (an online character: a copy of it).
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        let engine = self.engine.clone();
        match &mut self.kind {
            Kind::Local(s) => s.save(&engine, path),
            Kind::Online(o) => {
                let mut copy = o.ch.clone();
                engine.save(&mut copy, path)?;
                if matches!(o.backend, Backend::Gm { .. }) {
                    o.ch.file = Some(path.to_owned());
                    o.ch.dirty = false;
                }
                Ok(())
            }
        }
    }

    /// An online character's sync state (players only).
    pub fn sync_state(&self) -> Option<SyncState> {
        match self.backend()? {
            Backend::Player { session, id } => {
                let r = session.replica();
                Some(SyncState { mode: session.last_mode(), pending: r.outbox(id).len(), refused: r.refused().iter().filter(|x| x.character == *id).count() })
            }
            Backend::Gm { .. } => None,
        }
    }

    /// The player's refused commands for this character.
    pub fn refused(&self) -> Vec<Refused> {
        match self.backend() {
            Some(Backend::Player { session, id }) => session.replica().refused().iter().filter(|r| r.character == *id).cloned().collect(),
            _ => Vec::new(),
        }
    }

    pub fn dismiss_refused(&mut self) {
        if let Some(Backend::Player { session, .. }) = self.backend() {
            session.dismiss_refused();
        }
    }

    /// An online character's log, newest first: for a player, their
    /// character's feed as its owner reads it ("GM gave you 100 karma:
    /// note"); for the GM, the authority's feed for it, with what can be
    /// reverted.
    pub fn log(&self) -> Vec<LogLine> {
        match self.backend() {
            Some(Backend::Player { session, id }) => {
                let r = session.replica();
                r.feed().iter().rev().filter(|f| f.character == *id).map(|f| LogLine { at: f.at, text: chummer_sync::feed::for_owner(f), revert: None, refused: f.rejected.is_some() }).collect()
            }
            Some(Backend::Gm { host, id }) => {
                let a = host.authority();
                a.feed().iter().rev().filter(|f| f.character == *id).map(|f| gm_line(&a, f)).collect()
            }
            None => Vec::new(),
        }
    }

    /// GM characters: revert the change that made `version`.
    pub fn revert(&mut self, version: u64) -> Result<String, String> {
        let r = match self.backend() {
            Some(Backend::Gm { host, id }) => host.gm_revert(id, version)?,
            _ => return Err("only the GM can revert changes".into()),
        };
        self.refresh();
        Ok(r.applied.accepted.description)
    }
}

/// A line of the GM's feed, with the version to revert when it can be.
pub fn gm_line(a: &chummer_sync::Authority, f: &FeedEntry) -> LogLine {
    let revert = f.version.filter(|v| a.can_revert(&f.character, *v));
    LogLine { at: f.at, text: f.to_string(), revert, refused: f.rejected.is_some() }
}

fn pull(b: &Backend) -> Option<(Character, (u64, Hash, usize))> {
    match b {
        Backend::Gm { host, id } => {
            let a = host.authority();
            Some((a.character(id)?.clone(), (a.version(id)?, a.hash(id)?, 0)))
        }
        Backend::Player { session, id } => {
            let r = session.replica();
            Some((r.character(id)?.clone(), (r.version(id)?, r.confirmed_hash(id)?, r.outbox(id).len())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_only_through_commands() {
        let Ok(engine) = Engine::load() else { return };
        let engine = Arc::new(engine);
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin.chum5");
        let mut doc = Doc::new(Character::load(&p).unwrap(), engine);
        let before = doc.session().unwrap().state_hash();
        assert!(doc.set(Command::SetField { key: "alias".into(), value: "Doc".into() }));
        assert_eq!(doc.field("alias"), "Doc");
        assert_eq!(doc.session().unwrap().version(), 1);
        assert!(doc.undo().is_some());
        assert_eq!(doc.session().unwrap().state_hash(), before);
    }

    #[test]
    fn gm_characters_go_through_the_authority() {
        let Ok(engine) = Engine::load() else { return };
        let engine = Arc::new(engine);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let _g = rt.enter();
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
        let key = chummer_net::SecretKey::generate();
        let mut auth = chummer_sync::Authority::new(chummer_net::invite::CampaignId::random(), key.public(), "GM");
        let id = CharacterId::new("m");
        auth.add_character(id.clone(), None, Character::load(&p).unwrap()).unwrap();
        let host = AuthorityHost::new(auth, engine.clone(), key, None);
        let mut doc = Doc::online(Backend::Gm { host: host.clone(), id: id.clone() }, engine).unwrap();
        let karma = doc.karma;
        let award = Command::ManualExpense { karma: true, gain: true, expense: chummer_core::career::ManualExpense { amount: 7.0, reason: "Run".into(), ..Default::default() } };
        assert!(doc.set(award));
        assert_eq!(doc.karma, karma + 7);
        assert_eq!(host.authority().version(&id), Some(1));
        assert!(!doc.can_undo() && doc.undo().is_none(), "online: no undo");
        let log = doc.log();
        assert!(log[0].text.ends_with(": Gained 7 karma: Run"), "{}", log[0].text);
        let v = log[0].revert.expect("revertable");
        let rev0 = doc.revision();
        doc.revert(v).unwrap();
        assert_eq!(doc.karma, karma);
        assert!(doc.revision() > rev0);
        // An edit made elsewhere (another tab, the network) shows up.
        host.gm_edit(&id, Command::SetField { key: "alias".into(), value: "Raven".into() }).unwrap();
        assert!(doc.refresh());
        assert_eq!(doc.field("alias"), "Raven");
        assert!(!doc.refresh(), "nothing new");
    }
}
