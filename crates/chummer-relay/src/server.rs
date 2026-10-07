//! Runs the relay server and the mailbox node together.

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chummer_net::config::{NetConfig, RelayEntry};
use chummer_net::mailbox::MAILBOX_ALPN;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointId, RelayUrl};
use iroh_relay::server::{
    AcmeConfig, CertConfig, ClientRateLimit, QuicConfig, RelayConfig, Server, ServerConfig,
    TlsConfig as RelayTlsConfig,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::config::{CertMode, Config};
use crate::service::{Clock, MailboxService};
use crate::store::Store;

/// File names inside the data directory.
pub const MAILBOX_KEY_FILE: &str = "mailbox.key";
pub const MAILBOX_DB_FILE: &str = "mailbox.redb";
pub const SELF_SIGNED_CERT_FILE: &str = "self-signed-cert.pem";
pub const SELF_SIGNED_KEY_FILE: &str = "self-signed-key.pem";
pub const ACME_CACHE_DIR: &str = "acme";

/// How often expired mail is purged.
pub const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// A running relay (and mailbox).
pub struct RelayNode {
    server: Server,
    router: Option<Router>,
    url: RelayUrl,
    qad_port: Option<u16>,
    ca_cert: Option<Vec<u8>>,
    _purge: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for RelayNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayNode")
            .field("url", &self.url)
            .field("mailbox", &self.mailbox_id())
            .finish()
    }
}

fn base_tls() -> Result<rustls::ConfigBuilder<rustls::ServerConfig, rustls::server::WantsServerCert>>
{
    Ok(rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("TLS protocol versions")?
    .with_no_client_auth())
}

fn load_pem(
    cert: &Path,
    key: &Path,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let certs = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("reading certificate {}", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing certificate {}", cert.display()))?;
    if certs.is_empty() {
        bail!("no certificate in {}", cert.display());
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("reading private key {}", key.display()))?;
    Ok((certs, key))
}

/// Makes (once) and loads the self-signed certificate in `data_dir`.
fn self_signed(
    data_dir: &Path,
    hostname: &str,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let cert = data_dir.join(SELF_SIGNED_CERT_FILE);
    let key = data_dir.join(SELF_SIGNED_KEY_FILE);
    if !cert.exists() || !key.exists() {
        let mut names = vec![hostname.to_string()];
        for n in ["localhost", "127.0.0.1", "::1"] {
            if n != hostname {
                names.push(n.to_string());
            }
        }
        let made = rcgen::generate_simple_self_signed(names).context("making self-signed cert")?;
        std::fs::write(&cert, made.cert.pem())?;
        write_private(&key, made.signing_key.serialize_pem().as_bytes())?;
    }
    load_pem(&cert, &key)
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(data)?;
    Ok(())
}

impl RelayNode {
    /// Starts the relay described by `cfg`.
    pub async fn spawn(cfg: Config, clock: Arc<dyn Clock>) -> Result<RelayNode> {
        let host = cfg.hostname.trim();
        if host.is_empty() {
            bail!("`hostname` is required (the public DNS name of this server)");
        }
        std::fs::create_dir_all(&cfg.data_dir)
            .with_context(|| format!("creating data dir {}", cfg.data_dir.display()))?;
        let data_dir = &cfg.data_dir;

        let mut ca_cert = None;
        let cert = match cfg.tls.cert_mode {
            CertMode::LetsEncrypt => {
                let Some(contact) = cfg.tls.contact_email.clone() else {
                    bail!("cert_mode = \"lets-encrypt\" needs tls.contact_email");
                };
                let acme = AcmeConfig::letsencrypt(cfg.tls.acme_production)
                    .domains(vec![host.to_string()])
                    .contact(vec![format!("mailto:{contact}")])
                    .cache_path(data_dir.join(ACME_CACHE_DIR));
                CertConfig::LetsEncrypt {
                    acme_config: acme,
                    server_config_builder: base_tls()?,
                }
            }
            CertMode::Manual => {
                let cert = cfg
                    .tls
                    .cert_path
                    .clone()
                    .unwrap_or_else(|| data_dir.join("cert.pem"));
                let key = cfg
                    .tls
                    .key_path
                    .clone()
                    .unwrap_or_else(|| data_dir.join("key.pem"));
                let (certs, key) = load_pem(&cert, &key)?;
                CertConfig::Manual {
                    server_config: base_tls()?.with_single_cert(certs, key)?,
                }
            }
            CertMode::SelfSigned => {
                let (certs, key) = self_signed(data_dir, host)?;
                ca_cert = Some(certs[0].to_vec());
                CertConfig::Manual {
                    server_config: base_tls()?.with_single_cert(certs, key)?,
                }
            }
        };

        let mut relay = RelayConfig::new(cfg.http_bind);
        relay.tls = Some(RelayTlsConfig::new(cfg.https_bind, cert));
        if let Some(bps) = cfg.relay_rate_limit.and_then(NonZeroU32::new) {
            relay.limits.client_rx = Some(ClientRateLimit::new(bps));
        }
        let mut server_cfg = ServerConfig::default();
        server_cfg.relay = Some(relay);
        server_cfg.quic = cfg.qad_enabled.then(|| QuicConfig::new(cfg.qad_bind));
        server_cfg.metrics_addr = cfg.metrics_bind;
        let server = Server::spawn(server_cfg)
            .await
            .context("starting the relay server")?;

        let https: SocketAddr = server.https_addr().context("relay has no HTTPS address")?;
        let host_part = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let url: RelayUrl = if https.port() == 443 {
            format!("https://{host_part}")
        } else {
            format!("https://{host_part}:{}", https.port())
        }
        .parse()
        .context("relay URL from hostname")?;
        let qad_port = server.quic_addr().map(|a| a.port());

        let mut node = RelayNode {
            server,
            router: None,
            url,
            qad_port,
            ca_cert,
            _purge: None,
        };

        if cfg.mailbox_enabled {
            let (store, moved) =
                Store::open_or_recover(&data_dir.join(MAILBOX_DB_FILE), cfg.limits.clone(), clock.now())
                    .context("opening the mailbox database")?;
            if let Some(m) = moved {
                tracing::error!("the mailbox database was damaged; it was moved to {} and an empty one was made (mail in it is lost; clients send unanswered mail again)", m.display());
            }
            let store = Arc::new(store);
            let key = chummer_net::identity::load_or_create(&data_dir.join(MAILBOX_KEY_FILE))
                .context("loading the mailbox node key")?;
            let mut net = node.client_config_without_mailbox();
            net.port = (cfg.mailbox_port != 0).then_some(cfg.mailbox_port);
            let endpoint = chummer_net::node::bind(key, &net, vec![MAILBOX_ALPN.to_vec()])
                .await
                .context("starting the mailbox node")?;
            let service = MailboxService::new(store.clone(), clock.clone());
            node.router = Some(
                Router::builder(endpoint)
                    .accept(MAILBOX_ALPN, service)
                    .spawn(),
            );
            node._purge = Some(tokio::spawn(async move {
                let mut tick = tokio::time::interval(PURGE_INTERVAL);
                loop {
                    tick.tick().await;
                    let store = store.clone();
                    let now = clock.now();
                    match tokio::task::spawn_blocking(move || store.purge(now)).await {
                        Ok(Ok(n)) if n > 0 => tracing::info!("purged {n} expired messages"),
                        Ok(Err(e)) => tracing::warn!("purge failed: {e}"),
                        _ => {}
                    }
                }
            }));
        }
        Ok(node)
    }

    /// The relay's public URL.
    pub fn relay_url(&self) -> &RelayUrl {
        &self.url
    }

    /// The mailbox node's id, when the mailbox is enabled.
    pub fn mailbox_id(&self) -> Option<EndpointId> {
        self.router.as_ref().map(|r| r.endpoint().id())
    }

    pub fn mailbox_endpoint(&self) -> Option<&Endpoint> {
        self.router.as_ref().map(|r| r.endpoint())
    }

    /// The self-signed certificate (DER) clients must trust, in
    /// `self-signed` mode.
    pub fn self_signed_cert(&self) -> Option<&[u8]> {
        self.ca_cert.as_deref()
    }

    /// The entry for this relay in a client's relay list.
    pub fn relay_entry(&self) -> RelayEntry {
        RelayEntry {
            url: self.url.clone(),
            mailbox: self.mailbox_id(),
            qad_port: Some(self.qad_port.unwrap_or(0)),
        }
    }

    fn client_config_without_mailbox(&self) -> NetConfig {
        let mut entry = self.relay_entry();
        entry.mailbox = None;
        let mut cfg = NetConfig::with_relays([entry]);
        cfg.extra_ca_roots.extend(self.ca_cert.clone());
        cfg
    }

    /// A client configuration that uses only this relay.
    pub fn client_config(&self) -> NetConfig {
        let mut cfg = NetConfig::with_relays([self.relay_entry()]);
        cfg.extra_ca_roots.extend(self.ca_cert.clone());
        cfg
    }

    /// Waits until the mailbox node is connected to the relay.
    pub async fn wait_online(&self, timeout: Duration) -> Result<()> {
        if let Some(ep) = self.mailbox_endpoint() {
            tokio::time::timeout(timeout, ep.online())
                .await
                .context("the mailbox node did not connect to the relay in time")?;
        }
        Ok(())
    }

    /// Stops everything.
    pub async fn shutdown(self) -> Result<()> {
        if let Some(p) = &self._purge {
            p.abort();
        }
        if let Some(r) = &self.router {
            r.shutdown().await.context("stopping the mailbox")?;
        }
        self.server.shutdown().await.context("stopping the relay")?;
        Ok(())
    }

    /// Waits until the relay server stops by itself (an error).
    pub async fn join(&mut self) -> Result<()> {
        match self.server.join().await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => bail!("relay server failed: {e}"),
            Err(e) => bail!("relay server task failed: {e}"),
        }
    }
}

/// The default data directory used in the docs (`/var/lib/chummer-relay`).
pub fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/chummer-relay")
}
