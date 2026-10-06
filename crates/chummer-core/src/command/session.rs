//! One open character with its command log, version and undo/redo.

use std::path::Path;

use super::{apply, Applied, Command, Envelope, Rejected};
use crate::character::Character;
use crate::dice::Rng;
use crate::engine::Engine;

/// Undo steps kept per character.
pub const HISTORY_LIMIT: usize = 100;

/// Edits with the same [`Command::coalesce_key`] closer together than this
/// (milliseconds) merge into one step.
const COALESCE_MS: i64 = 1500;

/// One applied command in the log.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub envelope: Envelope,
    /// What it did ("Raised Pistols to 5 (10 karma)").
    pub description: String,
}

/// A character kept as its differences from a neighbouring state: the
/// typed fields in full, and only the top-level document children that
/// differ (the mugshots, often most of a file, are shared). The base is
/// the state the step is undone or redone from, which a stack always has
/// at hand.
struct Delta {
    rest: Character,
    children: Vec<Option<crate::xml::Node>>,
}

impl Delta {
    fn new(mut target: Character, base: &Character) -> Delta {
        let kids = std::mem::take(&mut target.doc.children);
        let children = kids.into_iter().enumerate().map(|(i, n)| if base.doc.children.get(i) == Some(&n) { None } else { Some(n) }).collect();
        Delta { rest: target, children }
    }

    fn rebuild(self, base: &Character) -> Character {
        let mut c = self.rest;
        c.doc.children = self.children.into_iter().enumerate().map(|(i, n)| n.unwrap_or_else(|| base.doc.children[i].clone())).collect();
        c
    }
}

struct UndoStep {
    /// The state before the step, relative to the state after it.
    before: Delta,
    /// A later edit may not merge into this step (it was undone to, or
    /// redone).
    sealed: bool,
}

struct RedoStep {
    entry: LogEntry,
    /// The state after the step, relative to the state before it.
    after: Delta,
}

/// What [`Session::apply`] reports back.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub description: String,
    pub message: Option<String>,
    pub count: Option<usize>,
    /// False when there was nothing to change (nothing was recorded).
    pub changed: bool,
}

/// An open character. The character is only reachable read-only, and
/// every change is an [`Envelope`] run by [`apply`], so the log replayed
/// on the loaded file gives the same character.
///
/// The version is the number of commands in the log, and is not saved in
/// the `.chum5`. Undo restores the state before the last command exactly
/// (and takes that command off the log); redo puts it back.
pub struct Session {
    ch: Character,
    log: Vec<LogEntry>,
    /// The states before the last `undo.len()` log entries.
    undo: Vec<UndoStep>,
    redo: Vec<RedoStep>,
    /// Counts every change (applies, undos, redos), for "did anything
    /// change since I last looked".
    revision: u64,
    seeds: Rng,
    author: String,
    limit: usize,
    /// The clock, Unix milliseconds; replaceable for tests.
    clock: fn() -> i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Session {
    pub fn new(ch: Character) -> Session {
        // Seeds for envelopes come from one generator per session, so two
        // commands in the same clock tick still differ.
        let seed = Rng::from_time().next_u64() ^ (&ch as *const Character as u64);
        Session::with_seed(ch, seed)
    }

    pub fn with_seed(ch: Character, seed: u64) -> Session {
        Session {
            ch,
            log: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            seeds: Rng::seeded(seed),
            author: String::new(),
            limit: HISTORY_LIMIT,
            clock: now_ms,
        }
    }

    /// Use another clock (Unix milliseconds), e.g. a fixed one in tests.
    pub fn with_clock(mut self, clock: fn() -> i64) -> Session {
        self.clock = clock;
        self
    }

    pub fn with_author(mut self, author: impl Into<String>) -> Session {
        self.author = author.into();
        self
    }

    pub fn ch(&self) -> &Character {
        &self.ch
    }

    /// Commands applied (and not undone) since the character was opened.
    pub fn version(&self) -> u64 {
        self.log.len() as u64
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    /// Undone commands that Redo would put back, the next one last.
    pub fn redo_entries(&self) -> impl DoubleEndedIterator<Item = &LogEntry> {
        self.redo.iter().map(|r| &r.entry)
    }

    pub fn envelopes(&self) -> Vec<Envelope> {
        self.log.iter().map(|e| e.envelope.clone()).collect()
    }

    pub fn state_hash(&self) -> [u8; 32] {
        super::state_hash(&self.ch)
    }

    /// Wrap a command in an envelope with a fresh seed and the time.
    pub fn envelope(&mut self, cmd: Command) -> Envelope {
        Envelope::new(cmd, self.seeds.next_u64(), (self.clock)(), self.author.clone())
    }

    /// Apply a command and record it.
    pub fn apply(&mut self, engine: &Engine, cmd: Command) -> Result<Report, Rejected> {
        let env = self.envelope(cmd);
        self.apply_envelope(engine, env)
    }

    /// Apply a ready envelope (one from another machine, or a test's).
    pub fn apply_envelope(&mut self, engine: &Engine, env: Envelope) -> Result<Report, Rejected> {
        let Applied { description, message, count, changed, before } = apply(&mut self.ch, engine, &env)?;
        if changed {
            self.record(env, description.clone(), *before);
        }
        Ok(Report { description, message, count, changed })
    }

    fn record(&mut self, env: Envelope, description: String, before: Character) {
        self.redo.clear();
        self.revision += 1;
        let key = env.cmd.coalesce_key();
        let merge = key.is_some()
            && self.undo.last().is_some_and(|u| !u.sealed)
            && self.log.last().is_some_and(|l| l.envelope.cmd.coalesce_key() == key && env.at - l.envelope.at <= COALESCE_MS && l.envelope.author == env.author);
        if merge {
            // The newer value replaces the older; the step still undoes
            // to the state before the first edit. `before` is the state
            // that step's delta was taken against.
            *self.log.last_mut().expect("checked") = LogEntry { envelope: env, description };
            let step = self.undo.pop().expect("checked");
            let first = step.before.rebuild(&before);
            self.undo.push(UndoStep { before: Delta::new(first, &self.ch), sealed: false });
            return;
        }
        self.log.push(LogEntry { envelope: env, description });
        self.undo.push(UndoStep { before: Delta::new(before, &self.ch), sealed: false });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// What Undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.can_undo().then(|| self.log.last().map(|l| l.description.as_str())).flatten()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|r| r.entry.description.as_str())
    }

    /// Restore the state before the last command. Returns its description.
    pub fn undo(&mut self) -> Option<String> {
        let step = self.undo.pop()?;
        let entry = self.log.pop().expect("an undo step has a log entry");
        let before = step.before.rebuild(&self.ch);
        let after = self.swap_in(before);
        if let Some(u) = self.undo.last_mut() {
            u.sealed = true;
        }
        let label = entry.description.clone();
        self.redo.push(RedoStep { entry, after: Delta::new(after, &self.ch) });
        Some(label)
    }

    /// Put back the last undone command. Returns its description.
    pub fn redo(&mut self) -> Option<String> {
        let RedoStep { entry, after } = self.redo.pop()?;
        let after = after.rebuild(&self.ch);
        let before = self.swap_in(after);
        self.undo.push(UndoStep { before: Delta::new(before, &self.ch), sealed: true });
        let label = entry.description.clone();
        self.log.push(entry);
        Some(label)
    }

    /// Make `state` current, keeping the file it is saved as; returns the
    /// old state.
    fn swap_in(&mut self, mut state: Character) -> Character {
        state.file = self.ch.file.clone();
        state.dirty = true;
        self.revision += 1;
        std::mem::replace(&mut self.ch, state)
    }

    /// Save to `path` (with the export totals refreshed, as
    /// [`Engine::save`] does) and remember it as the character's file.
    /// The character itself is not changed: the totals go into a copy.
    pub fn save(&mut self, engine: &Engine, path: &Path) -> std::io::Result<()> {
        let mut copy = self.ch.clone();
        engine.save(&mut copy, path)?;
        self.ch.file = Some(path.to_owned());
        self.ch.dirty = false;
        Ok(())
    }
}
