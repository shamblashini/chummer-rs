//! The authority's journal: every change the authority applies is appended
//! to `<sidecar>.journal` and synced to disk before its answer goes out.
//! The authority itself is saved only every couple of seconds (a save
//! compresses every changed character), so without the journal a crash
//! lost changes that players had already been told were accepted.
//!
//! Records are appended after the authority's lock is released (so the
//! GUI never waits for the disk) but before the answer is sent.
//!
//! Saving rotates the journal: under the authority's lock the state is
//! taken and the journal renamed to `<sidecar>.journal.saving`; once the
//! sidecar is written, that file is deleted. On start
//! ([`crate::AuthorityHost::new`]) the entries of both files that are newer
//! than the saved state are applied again ([`crate::Authority::replay`]);
//! commands are deterministic, so this gives exactly the state that was
//! acknowledged.
//!
//! Each record is a 4-byte big-endian length and the postcard form of
//! `(CharacterId, Entry)`. A torn last record (a crash while appending) is
//! ignored: its answer was never sent.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::msg::{CharacterId, Entry};

/// A record larger than this is taken to be damage.
const MAX_RECORD: usize = 64 * 1024 * 1024;

/// `<sidecar>.journal`.
pub fn path_for(sidecar: &Path) -> PathBuf {
    let mut p = sidecar.as_os_str().to_owned();
    p.push(".journal");
    PathBuf::from(p)
}

fn saving_path(sidecar: &Path) -> PathBuf {
    let mut p = sidecar.as_os_str().to_owned();
    p.push(".journal.saving");
    PathBuf::from(p)
}

/// The open journal of one authority file.
#[derive(Debug)]
pub struct Journal {
    sidecar: PathBuf,
    file: Option<File>,
}

impl Journal {
    pub fn new(sidecar: &Path) -> Journal {
        Journal { sidecar: sidecar.to_owned(), file: None }
    }

    /// Appends `records` and syncs them to disk.
    pub fn append(&mut self, records: &[(CharacterId, Entry)]) -> io::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut buf = Vec::new();
        for r in records {
            let bytes = postcard::to_stdvec(r).map_err(io::Error::other)?;
            buf.extend((bytes.len() as u32).to_be_bytes());
            buf.extend(bytes);
        }
        if self.file.is_none() {
            if let Some(dir) = self.sidecar.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)?;
            }
            self.file = Some(OpenOptions::new().create(true).append(true).open(path_for(&self.sidecar))?);
        }
        let f = self.file.as_mut().expect("opened");
        f.write_all(&buf)?;
        f.sync_data()
    }

    /// Before writing the state taken just now: what is in the journal is
    /// in that state, so set it aside (a save that failed earlier left a
    /// `.saving` file; the journal is added to it).
    pub fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let cur = path_for(&self.sidecar);
        if !cur.exists() {
            return Ok(());
        }
        let saving = saving_path(&self.sidecar);
        if saving.exists() {
            let bytes = std::fs::read(&cur)?;
            let mut f = OpenOptions::new().append(true).open(&saving)?;
            f.write_all(&bytes)?;
            f.sync_data()?;
            std::fs::remove_file(&cur)
        } else {
            std::fs::rename(&cur, &saving)
        }
    }

    /// The state was written: the set-aside entries are not needed.
    pub fn saved(&mut self) -> io::Result<()> {
        match std::fs::remove_file(saving_path(&self.sidecar)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Every record of both files, in version order per character
    /// (records are appended outside the authority's lock, so two
    /// answers may land in either order).
    pub fn read(sidecar: &Path) -> Vec<(CharacterId, Entry)> {
        let mut out: Vec<(CharacterId, Entry)> = Vec::new();
        for p in [saving_path(sidecar), path_for(sidecar)] {
            let Ok(mut f) = File::open(&p) else { continue };
            let mut bytes = Vec::new();
            if f.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let mut rest = &bytes[..];
            while rest.len() >= 4 {
                let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
                if len > MAX_RECORD || rest.len() < 4 + len {
                    break;
                }
                match postcard::from_bytes(&rest[4..4 + len]) {
                    Ok(r) => out.push(r),
                    Err(_) => break,
                }
                rest = &rest[4 + len..];
            }
        }
        out.sort_by(|a, b| (&a.0, a.1.version).cmp(&(&b.0, b.1.version)));
        out
    }
}
