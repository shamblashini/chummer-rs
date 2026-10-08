//! The container chummer-rs's own files use: `.chumrs` characters
//! ([`crate::chumrs`]) and `.chummercampaign` campaigns
//! ([`crate::campaign`]). `docs/file-format.md` is the full specification.
//!
//! A container is a ZIP archive. Its first entry is `manifest.json`:
//!
//! ```text
//! { "format": "chummer-rs character", "schema_version": 1,
//!   "app_version": "0.4.0", "created": "2026-10-08T12:00:00Z",
//!   "modified": "2026-10-08T12:30:00Z",
//!   "entries": { "character.xml": { "size": 81234, "blake3": "<64 hex>" }, ... } }
//! ```
//!
//! `entries` lists every other entry with its uncompressed size and BLAKE3
//! hash; reading checks both, so a damaged file fails with the entry's
//! name instead of loading something else. Entries the manifest does not
//! list are ignored. Text entries are deflated, images stored.
//!
//! `schema_version` counts incompatible changes to a format. Reading
//! runs the [`Migration`]s from the file's version up to the current one;
//! a file from a newer version is refused with a message saying so.
//!
//! Reading never trusts sizes the archive declares: every entry is read
//! through a limit ([`Limits`]), so a small forged file cannot expand to
//! gigabytes.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The manifest's entry name.
pub const MANIFEST: &str = "manifest.json";

/// The first bytes of a ZIP file (a local file header).
pub const MAGIC: &[u8; 4] = b"PK\x03\x04";

/// Whether `bytes` look like a container (a ZIP file).
pub fn is_container(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// One entry in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryInfo {
    /// Uncompressed size in bytes.
    pub size: u64,
    /// BLAKE3 of the uncompressed bytes, 64 lower-case hex digits.
    pub blake3: String,
}

/// `manifest.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub schema_version: u32,
    /// The chummer-rs version that wrote the file.
    #[serde(default)]
    pub app_version: String,
    /// When the file was first written (UTC, `YYYY-MM-DDTHH:MM:SSZ`).
    #[serde(default)]
    pub created: String,
    /// When it was last written.
    #[serde(default)]
    pub modified: String,
    #[serde(default)]
    pub entries: BTreeMap<String, EntryInfo>,
    /// Format-specific fields; unknown ones are kept as they are.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A container in memory: the manifest and the entries it lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Archive {
    pub manifest: Manifest,
    pub entries: BTreeMap<String, Vec<u8>>,
}

impl Archive {
    pub fn new() -> Archive {
        Archive::default()
    }

    pub fn put(&mut self, name: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.entries.insert(name.into(), bytes.into());
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    /// An entry as UTF-8 text.
    pub fn text(&self, name: &str) -> Result<Option<&str>, ContainerError> {
        match self.get(name) {
            None => Ok(None),
            Some(b) => std::str::from_utf8(b).map(Some).map_err(|_| ContainerError::Corrupt(format!("{name} is not UTF-8 text"))),
        }
    }
}

/// An upgrade of the archive from schema version `from` to `from + 1`.
pub struct Migration {
    pub from: u32,
    /// What changed, for the format documentation and error messages.
    pub what: &'static str,
    pub run: fn(&mut Archive) -> Result<(), ContainerError>,
}

/// How big a container may be.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The file itself.
    pub file: u64,
    /// Entries in the archive (listed or not).
    pub entries: usize,
    /// One entry, uncompressed.
    pub entry: u64,
    /// All entries together, uncompressed.
    pub total: u64,
    /// The manifest.
    pub manifest: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { file: 256 << 20, entries: 4096, entry: crate::chum5lz::MAX_DECOMPRESSED, total: 512 << 20, manifest: 4 << 20 }
    }
}

/// A container format: its `format` name, current version and upgrades.
pub struct Kind {
    pub format: &'static str,
    /// What users call it ("character", "campaign"), for messages.
    pub noun: &'static str,
    pub schema_version: u32,
    /// Upgrades, by `from` version; every version below the current one
    /// that files exist for needs one.
    pub migrations: &'static [Migration],
    pub limits: Limits,
}

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("not a chummer-rs file (no manifest.json): {0}")]
    NotAContainer(String),
    #[error("this is a {found} file, not a {expected} file")]
    WrongFormat { expected: String, found: String },
    #[error("this {noun} file was saved by a newer chummer-rs ({app}, file format version {found}); this version reads up to version {supported}. Update chummer-rs to open it.")]
    TooNew { noun: String, found: u32, supported: u32, app: String },
    #[error("the file is damaged: {0}")]
    Corrupt(String),
    #[error("the file is too large: {0}")]
    TooLarge(String),
    #[error("cannot upgrade the file from format version {0}: {1}")]
    Migration(u32, String),
}

impl From<ContainerError> for std::io::Error {
    fn from(e: ContainerError) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    }
}

/// The time now as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_utc() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    format!("{}Z", crate::chargen::iso_from_unix(secs))
}

/// Whether an entry is stored rather than deflated (already compressed
/// images).
fn stored(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// The container's bytes. Fills in the manifest's `format`,
/// `schema_version`, `app_version` and `entries`; `created` and
/// `modified` are the caller's (an empty `created` becomes `modified`).
pub fn encode(kind: &Kind, archive: &Archive) -> Vec<u8> {
    let mut m = archive.manifest.clone();
    m.format = kind.format.to_owned();
    m.schema_version = kind.schema_version;
    m.app_version = env!("CARGO_PKG_VERSION").to_owned();
    if m.modified.is_empty() {
        m.modified = now_utc();
    }
    if m.created.is_empty() {
        m.created = m.modified.clone();
    }
    m.entries = archive.entries.iter().map(|(n, b)| (n.clone(), EntryInfo { size: b.len() as u64, blake3: blake3::hash(b).to_hex().to_string() })).collect();
    let manifest = serde_json::to_vec_pretty(&m).expect("manifests serialise");

    // A fixed timestamp: the manifest has the real times, and two saves
    // of the same state give the same bytes.
    let base = zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default()).unix_permissions(0o644);
    let deflated = base.compression_method(zip::CompressionMethod::Deflated).compression_level(Some(6));
    let plain = base.compression_method(zip::CompressionMethod::Stored);
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(archive.entries.values().map(Vec::len).sum::<usize>() / 3 + 1024)));
    let mut put = |name: &str, bytes: &[u8], opts| {
        w.start_file(name, opts).expect("writing to memory");
        w.write_all(bytes).expect("writing to memory");
    };
    put(MANIFEST, &manifest, deflated);
    for (name, bytes) in &archive.entries {
        put(name, bytes, if stored(name) { plain } else { deflated });
    }
    w.finish().expect("writing to memory").into_inner()
}

/// Read a container of `kind`: check the manifest, the limits and every
/// listed entry's size and hash, then upgrade it to the current version.
pub fn decode(kind: &Kind, bytes: &[u8]) -> Result<Archive, ContainerError> {
    let lim = kind.limits;
    if bytes.len() as u64 > lim.file {
        return Err(ContainerError::TooLarge(format!("{} bytes (at most {})", bytes.len(), lim.file)));
    }
    if !is_container(bytes) {
        return Err(ContainerError::NotAContainer("not a ZIP archive".into()));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ContainerError::Corrupt(format!("ZIP: {e}")))?;
    if zip.len() > lim.entries {
        return Err(ContainerError::TooLarge(format!("{} entries (at most {})", zip.len(), lim.entries)));
    }
    let mut raw: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total = 0u64;
    let mut manifest_bytes = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| ContainerError::Corrupt(format!("ZIP entry {i}: {e}")))?;
        let name = f.name().to_owned();
        if f.is_dir() {
            continue;
        }
        if f.encrypted() {
            return Err(ContainerError::Corrupt(format!("{name} is encrypted")));
        }
        let cap = if name == MANIFEST { lim.manifest } else { lim.entry };
        let mut out = Vec::new();
        (&mut f).take(cap + 1).read_to_end(&mut out).map_err(|e| ContainerError::Corrupt(format!("{name}: {e}")))?;
        if out.len() as u64 > cap {
            return Err(ContainerError::TooLarge(format!("{name} is larger than {cap} bytes")));
        }
        total += out.len() as u64;
        if total > lim.total {
            return Err(ContainerError::TooLarge(format!("more than {} bytes in all", lim.total)));
        }
        if name == MANIFEST {
            if manifest_bytes.replace(out).is_some() {
                return Err(ContainerError::Corrupt("two manifests".into()));
            }
        } else if raw.insert(name.clone(), out).is_some() {
            return Err(ContainerError::Corrupt(format!("{name} is in the archive twice")));
        }
    }
    let manifest_bytes = manifest_bytes.ok_or_else(|| ContainerError::NotAContainer("the archive has no manifest.json".into()))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| ContainerError::Corrupt(format!("manifest.json: {e}")))?;
    if manifest.format != kind.format {
        return Err(ContainerError::WrongFormat { expected: kind.format.to_owned(), found: if manifest.format.is_empty() { "unknown".into() } else { manifest.format.clone() } });
    }
    if manifest.schema_version > kind.schema_version {
        return Err(ContainerError::TooNew { noun: kind.noun.to_owned(), found: manifest.schema_version, supported: kind.schema_version, app: format!("version {}", manifest.app_version) });
    }
    let mut entries = BTreeMap::new();
    for (name, info) in &manifest.entries {
        let b = raw.remove(name).ok_or_else(|| ContainerError::Corrupt(format!("{name} is missing")))?;
        if b.len() as u64 != info.size {
            return Err(ContainerError::Corrupt(format!("{name} has {} bytes, the manifest says {}", b.len(), info.size)));
        }
        if blake3::hash(&b).to_hex().as_str() != info.blake3.to_ascii_lowercase() {
            return Err(ContainerError::Corrupt(format!("{name} does not match its checksum")));
        }
        entries.insert(name.clone(), b);
    }
    let mut a = Archive { manifest, entries };
    migrate(kind, &mut a)?;
    Ok(a)
}

/// Upgrade `a` to `kind`'s current schema version, one step at a time.
pub fn migrate(kind: &Kind, a: &mut Archive) -> Result<(), ContainerError> {
    while a.manifest.schema_version < kind.schema_version {
        let v = a.manifest.schema_version;
        let m = kind.migrations.iter().find(|m| m.from == v).ok_or_else(|| ContainerError::Migration(v, "this version has no upgrade".into()))?;
        (m.run)(a).map_err(|e| ContainerError::Migration(v, format!("{}: {e}", m.what)))?;
        a.manifest.schema_version = v + 1;
    }
    Ok(())
}

/// Write `bytes` to `path` without ever leaving a half-written file: to a
/// temporary file in the same folder, flushed to disk, then renamed over
/// `path` (atomic on one file system).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let tmp = std::path::PathBuf::from(tmp);
    let r = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return r;
    }
    // The rename itself survives a crash once the folder is synced.
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(a: &mut Archive) -> Result<(), ContainerError> {
        let old = a.entries.remove("old.txt").ok_or_else(|| ContainerError::Corrupt("old.txt is missing".into()))?;
        a.put("new.txt", old);
        Ok(())
    }

    const TEST_V1: Kind = Kind { format: "chummer-rs test", noun: "test", schema_version: 1, migrations: &[], limits: Limits { file: 1 << 20, entries: 8, entry: 1000, total: 2000, manifest: 4096 } };
    const TEST_V2: Kind = Kind { schema_version: 2, migrations: &[Migration { from: 1, what: "old.txt is now new.txt", run: v2 }], ..TEST_V1 };

    fn sample() -> Archive {
        let mut a = Archive::new();
        a.put("old.txt", b"hello".to_vec());
        a.put("pic.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
        a
    }

    #[test]
    fn round_trip_and_deterministic() {
        let mut a = sample();
        a.manifest.modified = "2026-01-01T00:00:00Z".into();
        let bytes = encode(&TEST_V1, &a);
        assert!(is_container(&bytes));
        assert_eq!(bytes, encode(&TEST_V1, &a));
        let back = decode(&TEST_V1, &bytes).unwrap();
        assert_eq!(back.entries, a.entries);
        assert_eq!(back.manifest.created, "2026-01-01T00:00:00Z");
        assert_eq!(back.manifest.entries["old.txt"].size, 5);
    }

    #[test]
    fn migrates_old_versions_and_refuses_newer_ones() {
        let old = encode(&TEST_V1, &sample());
        let up = decode(&TEST_V2, &old).unwrap();
        assert_eq!(up.manifest.schema_version, 2);
        assert_eq!(up.get("new.txt"), Some(&b"hello"[..]));
        assert!(up.get("old.txt").is_none());
        let new = encode(&TEST_V2, &up);
        let e = decode(&TEST_V1, &new).unwrap_err();
        assert!(matches!(e, ContainerError::TooNew { found: 2, supported: 1, .. }), "{e}");
        assert!(e.to_string().contains("newer chummer-rs"), "{e}");
    }

    #[test]
    fn wrong_format_and_garbage() {
        let other = Kind { format: "chummer-rs other", ..TEST_V1 };
        let bytes = encode(&other, &sample());
        assert!(matches!(decode(&TEST_V1, &bytes), Err(ContainerError::WrongFormat { .. })));
        assert!(matches!(decode(&TEST_V1, b"<character/>"), Err(ContainerError::NotAContainer(_))));
        assert!(decode(&TEST_V1, b"PK\x03\x04garbage").is_err());
    }

    #[test]
    fn detects_damage() {
        // A stored entry: flip a byte of its data in place.
        let mut a = Archive::new();
        a.put("pic.png", b"0123456789abcdef".to_vec());
        let mut bytes = encode(&TEST_V1, &a);
        let at = bytes.windows(16).position(|w| w == b"0123456789abcdef").unwrap();
        bytes[at + 3] ^= 1;
        let e = decode(&TEST_V1, &bytes).unwrap_err();
        assert!(matches!(e, ContainerError::Corrupt(_)), "{e}");
    }

    #[test]
    fn limits() {
        let mut a = Archive::new();
        a.put("big.txt", vec![b'a'; 1001]);
        assert!(matches!(decode(&TEST_V1, &encode(&TEST_V1, &a)), Err(ContainerError::TooLarge(_))));
        let mut a = Archive::new();
        for i in 0..9 {
            a.put(format!("{i}.txt"), b"x".to_vec());
        }
        assert!(matches!(decode(&TEST_V1, &encode(&TEST_V1, &a)), Err(ContainerError::TooLarge(_))));
    }

    #[test]
    fn atomic_write_replaces() {
        let dir = std::env::temp_dir().join(format!("chummer-container-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.bin");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary files left");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
