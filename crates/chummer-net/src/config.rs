//! Which relays the app uses.
//!
//! A relay entry is the relay's HTTPS URL plus, optionally, the node id of
//! the mailbox that runs next to it. As text: `https://relay.example.org`
//! or `https://relay.example.org#<mailbox-node-id>` (what `chummer-relay`
//! prints at start-up).

use std::fmt;
use std::str::FromStr;

use iroh::{EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};

/// The project's public relay.
///
/// Placeholder until the owner's server is set up; see `docs/relay.md`.
pub const DEFAULT_RELAY_URL: &str = "https://relay.chummer-rs.example";

/// Node id of the mailbox on [`DEFAULT_RELAY_URL`] (printed by
/// `chummer-relay` on start-up). `None` until the owner's server is set up.
pub const DEFAULT_MAILBOX_ID: Option<&str> = None;

/// iroh's default port for QUIC address discovery on a relay.
pub const DEFAULT_QAD_PORT: u16 = 7842;

/// One relay the app can use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayEntry {
    pub url: RelayUrl,
    /// The node id of the mailbox served next to this relay, if any.
    pub mailbox: Option<EndpointId>,
    /// UDP port of the relay's QUIC address discovery; `None` = default
    /// ([`DEFAULT_QAD_PORT`]). `Some(0)` turns it off for this relay.
    #[serde(default)]
    pub qad_port: Option<u16>,
}

impl RelayEntry {
    pub fn new(url: RelayUrl) -> Self {
        RelayEntry {
            url,
            mailbox: None,
            qad_port: None,
        }
    }

    pub fn with_mailbox(mut self, mailbox: EndpointId) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    pub(crate) fn iroh_config(&self) -> iroh_relay::RelayConfig {
        let quic = match self.qad_port {
            Some(0) => None,
            Some(p) => Some(iroh_relay::RelayQuicConfig::new(p)),
            None => Some(iroh_relay::RelayQuicConfig::new(DEFAULT_QAD_PORT)),
        };
        iroh_relay::RelayConfig::new(self.url.clone(), quic)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RelayEntryError {
    #[error("not a valid relay URL")]
    BadUrl,
    #[error("not a valid mailbox node id")]
    BadMailbox,
}

impl fmt::Display for RelayEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = self.url.as_str();
        f.write_str(url)?;
        if let Some(m) = &self.mailbox {
            write!(f, "#{m}")?;
        }
        Ok(())
    }
}

impl FromStr for RelayEntry {
    type Err = RelayEntryError;

    fn from_str(s: &str) -> Result<Self, RelayEntryError> {
        let (url, mailbox) = match s.trim().split_once('#') {
            Some((u, m)) => (u, Some(m.parse().map_err(|_| RelayEntryError::BadMailbox)?)),
            None => (s.trim(), None),
        };
        Ok(RelayEntry {
            url: url.parse().map_err(|_| RelayEntryError::BadUrl)?,
            mailbox,
            qad_port: None,
        })
    }
}

/// Network settings for one app instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetConfig {
    /// Relays to use. The first reachable one with the lowest latency
    /// becomes the home relay; peers are looked up on all of them.
    pub relays: Vec<RelayEntry>,
    /// Extra trusted CA certificates (DER) for relays with private or
    /// self-signed certificates. The usual web roots are always trusted.
    #[serde(default)]
    pub extra_ca_roots: Vec<Vec<u8>>,
    /// Fixed UDP port for direct connections (both IPv4 and IPv6). `None`
    /// picks a free port.
    #[serde(default)]
    pub port: Option<u16>,
}

impl Default for NetConfig {
    /// The project's public relay only.
    fn default() -> Self {
        let mut entry =
            RelayEntry::new(DEFAULT_RELAY_URL.parse().expect("valid default relay url"));
        entry.mailbox = DEFAULT_MAILBOX_ID.and_then(|m| m.parse().ok());
        NetConfig {
            relays: vec![entry],
            extra_ca_roots: Vec::new(),
            port: None,
        }
    }
}

impl NetConfig {
    /// Only the given relays (no default relay).
    pub fn with_relays(relays: impl IntoIterator<Item = RelayEntry>) -> Self {
        NetConfig {
            relays: relays.into_iter().collect(),
            extra_ca_roots: Vec::new(),
            port: None,
        }
    }

    /// Adds user-configured relays after the existing ones, skipping
    /// duplicates (by URL).
    pub fn add_relays(&mut self, relays: impl IntoIterator<Item = RelayEntry>) {
        for r in relays {
            if !self.relays.iter().any(|e| e.url == r.url) {
                self.relays.push(r);
            }
        }
    }

    /// The first relay that has a mailbox.
    pub fn mailbox(&self) -> Option<&RelayEntry> {
        self.relays.iter().find(|r| r.mailbox.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_text_round_trip() {
        let id = iroh::SecretKey::from_bytes(&[3; 32]).public();
        let e: RelayEntry = format!("https://relay.example.org#{id}").parse().unwrap();
        assert_eq!(e.url.as_str(), "https://relay.example.org/");
        assert_eq!(e.mailbox, Some(id));
        assert_eq!(e.to_string().parse::<RelayEntry>().unwrap(), e);
        let plain: RelayEntry = "https://r.example:8443".parse().unwrap();
        assert_eq!(plain.mailbox, None);
        assert_eq!(
            "https://r.example#nope".parse::<RelayEntry>(),
            Err(RelayEntryError::BadMailbox)
        );
        assert_eq!("::".parse::<RelayEntry>(), Err(RelayEntryError::BadUrl));
    }

    #[test]
    fn default_and_user_relays() {
        let mut cfg = NetConfig::default();
        assert_eq!(cfg.relays.len(), 1);
        assert_eq!(
            cfg.relays[0].url.as_str(),
            "https://relay.chummer-rs.example/"
        );
        cfg.add_relays([
            "https://mine.example".parse().unwrap(),
            DEFAULT_RELAY_URL.parse().unwrap(),
        ]);
        assert_eq!(cfg.relays.len(), 2);
    }
}
