//! An open character as the GUI holds it: a command [`Session`] plus the
//! engine its commands run with.
//!
//! The character is only reachable read-only (`Deref<Target = Character>`).
//! Every change is a [`Command`] sent through [`Doc::apply`], which runs
//! `chummer_core::command::apply` and records it for undo/redo and the
//! history; so the GUI cannot change a character any other way.

use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use chummer_core::character::Character;
use chummer_core::command::{Command, Rejected, Report, Session};
use chummer_core::engine::Engine;

use crate::pdf_ui::Status;

pub struct Doc {
    session: Session,
    engine: Arc<Engine>,
}

impl Deref for Doc {
    type Target = Character;
    fn deref(&self) -> &Character {
        self.session.ch()
    }
}

impl Doc {
    pub fn new(ch: Character, engine: Arc<Engine>) -> Doc {
        Doc { session: Session::new(ch), engine }
    }

    pub fn ch(&self) -> &Character {
        self.session.ch()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Run a command; the only way the GUI changes a character.
    pub fn apply(&mut self, cmd: Command) -> Result<Report, Rejected> {
        let engine = self.engine.clone();
        self.session.apply(&engine, cmd)
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

    pub fn undo(&mut self) -> Option<String> {
        self.session.undo()
    }

    pub fn redo(&mut self) -> Option<String> {
        self.session.redo()
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        let engine = self.engine.clone();
        self.session.save(&engine, path)
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
        let before = doc.session().state_hash();
        assert!(doc.set(Command::SetField { key: "alias".into(), value: "Doc".into() }));
        assert_eq!(doc.field("alias"), "Doc");
        assert_eq!(doc.session().version(), 1);
        assert!(doc.undo().is_some());
        assert_eq!(doc.session().state_hash(), before);
    }
}
