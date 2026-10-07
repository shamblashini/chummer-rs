//! Files the sync layer keeps: a 4-byte magic, a format version (u16 big
//! endian), then postcard. Written atomically (temporary file + rename).

use std::io;
use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("not a {0} file")]
    Magic(&'static str),
    #[error("unsupported {what} format version {found}")]
    Version { what: &'static str, found: u16 },
    #[error("damaged file: {0}")]
    Decode(#[from] postcard::Error),
    #[error("a stored character could not be read: {0}")]
    Character(#[from] chummer_core::command::RestoreError),
}

pub(crate) fn to_bytes<T: Serialize>(magic: &[u8; 4], version: u16, value: &T) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.extend(version.to_be_bytes());
    out.extend(postcard::to_stdvec(value).expect("sync state serialises"));
    out
}

pub(crate) fn from_bytes<T: DeserializeOwned>(magic: &[u8; 4], what: &'static str, version: u16, bytes: &[u8]) -> Result<T, PersistError> {
    if bytes.len() < 6 || &bytes[..4] != magic {
        return Err(PersistError::Magic(what));
    }
    let found = u16::from_be_bytes([bytes[4], bytes[5]]);
    if found != version {
        return Err(PersistError::Version { what, found });
    }
    Ok(postcard::from_bytes(&bytes[6..])?)
}

/// Writes `bytes` to `path` through a temporary file and a rename, so a
/// crash leaves the old file or the new one, never half of one. Each call
/// has its own temporary file, so saves of one file from several tasks
/// at once do not clash (the last rename wins; callers that care about
/// order hold a lock across taking the state and writing it).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = std::path::PathBuf::from(tmp);
    let written = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}
