//! The persistent node key.
//!
//! Each installation has one ed25519 key pair: the iroh node key. The public
//! half ([`EndpointId`](iroh::EndpointId)) is the user's identity in every
//! campaign; there are no accounts or passwords. The secret half is kept in
//! `node.key` in the user config directory:
//!
//! - Linux/BSD: `$XDG_CONFIG_HOME/chummer-rs/node.key` (`~/.config/...`)
//! - Windows: `%APPDATA%\chummer-rs\node.key`
//! - macOS: `~/Library/Application Support/chummer-rs/node.key`
//!
//! `XDG_CONFIG_HOME` is honoured on every platform when it is set, so tests
//! and portable setups can redirect it (the GUI does the same).
//!
//! The file holds the 32-byte secret as 64 hex characters. Losing it means a
//! new identity: the GM has to invite the player again.

use std::io;
use std::path::{Path, PathBuf};

use iroh::SecretKey;

/// File name of the node key inside [`config_dir`].
pub const KEY_FILE: &str = "node.key";

/// `chummer-rs` inside the user config directory.
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::config_dir)?;
    Some(base.join("chummer-rs"))
}

/// Default path of the node key.
pub fn default_key_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(KEY_FILE))
}

/// Loads the node key at the default path, creating it on first use.
pub fn load_or_create_default() -> io::Result<SecretKey> {
    let path = default_key_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no user config directory found"))?;
    load_or_create(&path)
}

/// Loads the key at `path`, or creates a new random key there.
pub fn load_or_create(path: &Path) -> io::Result<SecretKey> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_key(&text).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a valid node key", path.display()),
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            write_key(path, &key)?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

/// Parses a key file's contents (64 hex characters, surrounding whitespace ignored).
pub fn parse_key(text: &str) -> Option<SecretKey> {
    crate::hex::decode::<32>(text.trim()).map(|b| SecretKey::from_bytes(&b))
}

/// Writes `key` to `path` atomically (temporary file + rename), readable
/// only by the owner on Unix.
pub fn write_key(path: &Path, key: &SecretKey) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("key.tmp");
    let text = format!("{}\n", crate::hex::encode(&key.to_bytes()));
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_same_key() {
        let dir = std::env::temp_dir().join(format!("chummer-net-key-{}", std::process::id()));
        let path = dir.join("sub").join(KEY_FILE);
        let _ = std::fs::remove_dir_all(&dir);
        let a = load_or_create(&path).unwrap();
        let b = load_or_create(&path).unwrap();
        assert_eq!(a.public(), b.public());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&path, "nonsense").unwrap();
        assert!(load_or_create(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
