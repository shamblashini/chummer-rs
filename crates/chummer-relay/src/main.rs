//! `chummer-relay`: the chummer-rs public relay and mailbox.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chummer_relay::{CertMode, Config, RelayNode, SystemClock};
use clap::Parser;

/// The chummer-rs relay: connects players and GMs, and keeps sealed mail
/// for those who are offline. See docs/relay.md.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// TOML configuration file.
    #[arg(long, short)]
    config: Option<PathBuf>,
    /// Public DNS name of this server (overrides the file).
    #[arg(long, env = "RELAY_HOSTNAME")]
    hostname: Option<String>,
    /// Data directory (overrides the file).
    #[arg(long, env = "RELAY_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// How to get the TLS certificate (overrides the file).
    #[arg(long, value_enum, env = "RELAY_CERT_MODE")]
    cert_mode: Option<CertMode>,
    /// Let's Encrypt contact email (overrides the file). Optional.
    #[arg(long, env = "RELAY_CONTACT_EMAIL")]
    contact_email: Option<String>,
    /// Local test setup: self-signed certificate for 127.0.0.1 on
    /// unprivileged ports (HTTP 3340, HTTPS 3443, QAD 7842, mailbox 7843).
    #[arg(long)]
    dev: bool,
    /// Print the default configuration as TOML and exit.
    #[arg(long)]
    print_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    chummer_relay::log::init("info,iroh=warn,iroh_relay=info");
    let args = Args::parse();

    let mut cfg = match &args.config {
        Some(p) => Config::from_toml(
            &std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        )
        .with_context(|| format!("parsing {}", p.display()))?,
        None => Config::default(),
    };
    if args.print_config {
        print!("{}", cfg.to_toml());
        return Ok(());
    }
    if args.dev {
        cfg.hostname = "127.0.0.1".into();
        cfg.tls.cert_mode = CertMode::SelfSigned;
        cfg.http_bind = "[::]:3340".parse()?;
        cfg.https_bind = "[::]:3443".parse()?;
    }
    // Empty environment variables (an unset Coolify field) mean "not given".
    if let Some(h) = args.hostname.filter(|h| !h.trim().is_empty()) {
        cfg.hostname = h;
    }
    if let Some(d) = args.data_dir.filter(|d| !d.as_os_str().is_empty()) {
        cfg.data_dir = d;
    }
    if let Some(m) = args.cert_mode {
        cfg.tls.cert_mode = m;
    }
    if let Some(e) = args.contact_email.filter(|e| !e.trim().is_empty()) {
        cfg.tls.contact_email = Some(e);
    }

    let mut node = RelayNode::spawn(cfg.clone(), Arc::new(SystemClock)).await?;
    match node.relay_url() {
        Some(url) => tracing::info!("relay listening: {url}"),
        None => tracing::info!(
            "relay listening on {} behind your proxy (no hostname set)",
            node.local_url()
        ),
    }
    match node.mailbox_id() {
        Some(id) => {
            match node.wait_online(std::time::Duration::from_secs(30)).await {
                Ok(()) => tracing::info!("mailbox node connected to the relay"),
                Err(e) => tracing::warn!("{e:#} (is `hostname` reachable from this server?)"),
            }
            tracing::info!("mailbox node id: {id}");
            match node.relay_url() {
                Some(_) => tracing::info!("give users this relay entry: {}", node.relay_entry()),
                None => tracing::info!(
                    "give users this relay entry: https://<the domain your proxy serves>#{id}"
                ),
            }
        }
        None => tracing::info!("mailbox disabled"),
    }
    if cfg.tls.cert_mode == CertMode::SelfSigned {
        tracing::info!(
            "self-signed certificate: {} (clients must trust it)",
            cfg.data_dir
                .join(chummer_relay::server::SELF_SIGNED_CERT_FILE)
                .display()
        );
    }

    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
        r = node.join() => r?,
    }
    node.shutdown().await
}
