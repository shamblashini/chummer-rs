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
    #[arg(long)]
    hostname: Option<String>,
    /// Data directory (overrides the file).
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// How to get the TLS certificate (overrides the file).
    #[arg(long, value_enum)]
    cert_mode: Option<CertMode>,
    /// Let's Encrypt contact email (overrides the file).
    #[arg(long)]
    contact_email: Option<String>,
    /// Local test setup: self-signed certificate for localhost on
    /// unprivileged ports (HTTP 3340, HTTPS 3443, QAD 7842, mailbox 7843).
    #[arg(long)]
    dev: bool,
    /// Print the default configuration as TOML and exit.
    #[arg(long)]
    print_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,iroh=warn,iroh_relay=info".into()),
        )
        .init();
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
        cfg.hostname = "localhost".into();
        cfg.tls.cert_mode = CertMode::SelfSigned;
        cfg.http_bind = "[::]:3340".parse()?;
        cfg.https_bind = "[::]:3443".parse()?;
    }
    if let Some(h) = args.hostname {
        cfg.hostname = h;
    }
    if let Some(d) = args.data_dir {
        cfg.data_dir = d;
    }
    if let Some(m) = args.cert_mode {
        cfg.tls.cert_mode = m;
    }
    if let Some(e) = args.contact_email {
        cfg.tls.contact_email = Some(e);
    }

    let mut node = RelayNode::spawn(cfg.clone(), Arc::new(SystemClock)).await?;
    tracing::info!("relay listening: {}", node.relay_url());
    match node.mailbox_id() {
        Some(id) => {
            tracing::info!("mailbox node id: {id}");
            tracing::info!("give users this relay entry: {}", node.relay_entry());
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
