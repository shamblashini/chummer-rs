//! Relay configuration (TOML file, overridable by command-line flags).

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use crate::store::Limits;

/// How the relay gets its TLS certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum CertMode {
    /// Let's Encrypt via ACME (TLS-ALPN-01 on the HTTPS port). Needs
    /// `hostname` pointing at this server and `contact_email`.
    LetsEncrypt,
    /// A certificate and key you provide (PEM files), e.g. from certbot.
    Manual,
    /// A self-signed certificate made on first start and kept in the data
    /// directory. Clients must be given the certificate to trust. For
    /// testing and private groups.
    SelfSigned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    pub cert_mode: CertMode,
    /// ACME account contact, without `mailto:`.
    pub contact_email: Option<String>,
    /// Use Let's Encrypt production (true) or staging (false).
    pub acme_production: bool,
    /// PEM certificate chain for `manual`. Default `<data_dir>/cert.pem`.
    pub cert_path: Option<PathBuf>,
    /// PEM private key for `manual`. Default `<data_dir>/key.pem`.
    pub key_path: Option<PathBuf>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        TlsConfig {
            cert_mode: CertMode::LetsEncrypt,
            contact_email: None,
            acme_production: true,
            cert_path: None,
            key_path: None,
        }
    }
}

/// Everything the relay needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Public DNS name of this server, e.g. `relay.example.org`.
    pub hostname: String,
    /// Where the mailbox database, the mailbox node key and certificates live.
    pub data_dir: PathBuf,
    /// Plain HTTP (captive-portal probe). iroh-relay always serves this.
    pub http_bind: SocketAddr,
    /// HTTPS: the relay itself, and ACME challenges.
    pub https_bind: SocketAddr,
    /// UDP for QUIC address discovery (helps clients hole-punch).
    pub qad_bind: SocketAddr,
    /// Serve QUIC address discovery on `qad_bind`.
    pub qad_enabled: bool,
    /// UDP port of the mailbox node for direct connections. 0 = any free
    /// port (clients then reach the mailbox through the relay, unless hole
    /// punching finds a path).
    pub mailbox_port: u16,
    /// Serve the mailbox at all.
    pub mailbox_enabled: bool,
    /// Prometheus metrics (keep it on localhost). `None` = off.
    pub metrics_bind: Option<SocketAddr>,
    /// Bytes per second each client may send through the relay. `None` =
    /// unlimited.
    pub relay_rate_limit: Option<u32>,
    pub tls: TlsConfig,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hostname: String::new(),
            data_dir: PathBuf::from("chummer-relay-data"),
            http_bind: "[::]:80".parse().expect("addr"),
            https_bind: "[::]:443".parse().expect("addr"),
            qad_bind: "[::]:7842".parse().expect("addr"),
            qad_enabled: true,
            mailbox_port: 7843,
            mailbox_enabled: true,
            metrics_bind: None,
            relay_rate_limit: None,
            tls: TlsConfig::default(),
            limits: Limits::default(),
        }
    }
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config serialises")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip_and_partial_files() {
        let cfg = Config::default();
        assert_eq!(Config::from_toml(&cfg.to_toml()).unwrap(), cfg);
        let partial = Config::from_toml(
            r#"
hostname = "relay.example.org"
[tls]
contact_email = "gm@example.org"
[limits]
max_blob_bytes = 1024
"#,
        )
        .unwrap();
        assert_eq!(partial.hostname, "relay.example.org");
        assert_eq!(partial.tls.cert_mode, CertMode::LetsEncrypt);
        assert_eq!(partial.limits.max_blob_bytes, 1024);
        assert_eq!(partial.limits.max_messages_per_recipient, 1000);
        assert!(Config::from_toml("bogus = 1").is_err());
    }

    #[test]
    fn example_file_parses_to_defaults() {
        let example = include_str!("../../../packaging/relay/relay.example.toml");
        let cfg = Config::from_toml(example).unwrap();
        let mut expect = Config {
            hostname: "relay.example.org".into(),
            data_dir: "/var/lib/chummer-relay".into(),
            ..Config::default()
        };
        expect.tls.contact_email = Some("you@example.org".into());
        assert_eq!(cfg, expect);
    }
}
