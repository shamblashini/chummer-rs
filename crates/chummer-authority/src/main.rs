//! `chummer-authority`: a GM's campaign served without the GUI, for groups
//! that want their campaign online all the time (play-by-post, players in
//! other time zones). It runs the same glue as the app's "Host online"
//! (`chummer_sync::hosted`): the campaign file plus its `.authority`
//! sidecar, with the GM's node key. See docs/online-design.md and the
//! README's "Online campaigns".
//!
//! Do not host the same campaign in the app and here at the same time:
//! both would write the sidecar. Stop one before starting the other.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use chummer_core::campaign::{Campaign, MemberId};
use chummer_core::engine::Engine;
use chummer_net::config::{OnlineSettings, RelayEntry, DEFAULT_RELAY_URL};
use chummer_net::invite::{InviteLink, Role};
use chummer_net::SecretKey;
use chummer_sync::hosted::{self, HostedCampaign, GM_OWNER};
use chummer_sync::{Authority, Node};
use clap::{Parser, Subcommand, ValueEnum};

/// Serve a chummer-rs campaign around the clock, make invite links, show
/// its state and give characters to players.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// The GM's node key (64 hex characters). Default: node.key in the
    /// chummer-rs config directory, the one the app uses. Copy it here to
    /// host a campaign made in the app.
    #[arg(long, global = true)]
    key: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Serve the campaign until stopped (Ctrl+C or SIGTERM).
    Run {
        campaign: PathBuf,
        /// The GM's name as players see it.
        #[arg(long, default_value = "GM")]
        name: String,
        /// Relay entries (`https://relay.example.org#<mailbox-id>`); replace
        /// the ones from online.json.
        #[arg(long = "relay")]
        relays: Vec<String>,
        /// PEM files with extra trusted relay certificates.
        #[arg(long = "ca")]
        ca_files: Vec<PathBuf>,
        /// Fixed UDP port for direct connections.
        #[arg(long)]
        port: Option<u16>,
        /// Seconds between mailbox rounds.
        #[arg(long, default_value_t = 180)]
        mail_every: u64,
        /// Seconds between writing the characters back into the campaign
        /// file (when they changed).
        #[arg(long, default_value_t = 300)]
        write_back_every: u64,
    },
    /// Print a new invite link. A running host takes it in within seconds.
    Invite {
        campaign: PathBuf,
        #[arg(long, value_enum, default_value_t = RoleArg::Player)]
        role: RoleArg,
        /// A note for yourself ("Thursday group").
        #[arg(long, default_value = "")]
        label: String,
        /// The relay to put in the link, for players whose app does not
        /// have it (default: the first configured one, unless it is the
        /// project's relay).
        #[arg(long)]
        relay: Option<String>,
    },
    /// Show members, characters, owners and the latest activity.
    Status {
        campaign: PathBuf,
        /// Feed lines to show.
        #[arg(long, default_value_t = 15)]
        feed: usize,
    },
    /// Give a member's character to a player (their node id, as the app's
    /// Online Settings shows it, or a joined member's name), or back to
    /// the GM with `gm`. A running host takes it in within seconds.
    Assign { campaign: PathBuf, member: String, owner: String },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum RoleArg {
    Player,
    Gm,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,iroh=warn".into()))
        .init();
    let args = Args::parse();
    // Only `run` makes a new key; the others need the GM's existing one.
    let key = load_key(args.key.as_deref(), matches!(args.cmd, Cmd::Run { .. }))?;
    match args.cmd {
        Cmd::Run { campaign, name, relays, ca_files, port, mail_every, write_back_every } => {
            let mut s = OnlineSettings::load();
            if !relays.is_empty() {
                s.relays = relays;
            }
            if !ca_files.is_empty() {
                s.ca_files = ca_files;
            }
            if port.is_some() {
                s.port = port;
            }
            run(&campaign, key, &name, &s, Duration::from_secs(mail_every.max(10)), Duration::from_secs(write_back_every.max(10))).await
        }
        Cmd::Invite { campaign, role, label, relay } => invite(&campaign, &key, role, &label, relay),
        Cmd::Status { campaign, feed } => status(&campaign, feed),
        Cmd::Assign { campaign, member, owner } => assign(&campaign, &member, &owner),
    }
}

fn load_key(path: Option<&Path>, create: bool) -> Result<SecretKey> {
    let p = match path {
        Some(p) => p.to_owned(),
        None => chummer_net::identity::default_key_path().context("no user config directory found")?,
    };
    if create {
        return chummer_net::identity::load_or_create(&p).with_context(|| format!("node key {}", p.display()));
    }
    let text = std::fs::read_to_string(&p).with_context(|| format!("no node key at {} (pass the GM's with --key)", p.display()))?;
    chummer_net::identity::parse_key(&text).with_context(|| format!("{} is not a valid node key", p.display()))
}

fn load_campaign(path: &Path) -> Result<Campaign> {
    Campaign::load(path).with_context(|| format!("reading {}", path.display()))
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

async fn run(path: &Path, key: SecretKey, name: &str, settings: &OnlineSettings, mail_every: Duration, write_back_every: Duration) -> Result<()> {
    let engine = Arc::new(Engine::load().context("loading the game data (is the resources folder next to the program?)")?);
    let mut campaign = load_campaign(path)?;
    let (h, rec) = HostedCampaign::open(&campaign, path, engine, key.clone(), name, |_| None)?;
    for (m, e) in &rec.failed {
        tracing::warn!("member {m}: {e}");
    }
    let cfg = settings.net_config().map_err(anyhow::Error::msg)?;
    let node = Node::start(key, cfg).await?;
    node.serve(&h.host);
    tracing::info!("serving \"{}\" as {}", campaign.name, node.id());
    match node.home_relay() {
        Some(r) => tracing::info!("relay: {r}"),
        None => tracing::warn!("no relay reachable yet; still trying"),
    }
    if node.mailbox_id().is_none() {
        tracing::warn!("no relay with a mailbox is configured: offline players get nothing until they connect");
    }
    print_summary(&h.host.authority(), &campaign);

    let mut seen = mtime(path);
    let mut written = versions(&h.host.authority());
    let mut mail = tokio::time::interval(mail_every);
    let mut back = tokio::time::interval(write_back_every);
    back.tick().await;
    let mut files = tokio::time::interval(Duration::from_secs(5));
    let mut events = h.host.subscribe();
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        #[cfg(unix)]
        let stop = async { term.recv().await };
        #[cfg(not(unix))]
        let stop = std::future::pending::<Option<()>>();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = stop => break,
            _ = mail.tick() => match node.sync_mail(&h.host).await {
                Ok(r) if r.fetched + r.sent > 0 => tracing::info!("mail: {} read, {} applied, {} dropped, {} sent", r.fetched, r.handled, r.dropped, r.sent),
                Ok(_) => {}
                Err(e) => tracing::warn!("mailbox: {e}"),
            },
            _ = files.tick() => {
                let n = h.merge_invites();
                if n > 0 {
                    tracing::info!("took in {n} new invite(s)");
                }
                // The campaign file changed (`assign`, or the GM edited it).
                if mtime(path) != seen {
                    match load_campaign(path) {
                        Ok(c) => {
                            campaign = c;
                            let r = h.reconcile(&campaign, |_| None);
                            tracing::info!("campaign file changed: {} added, {} removed, {} owners changed", r.added.len(), r.removed, r.owners);
                        }
                        Err(e) => tracing::warn!("{e:#}"),
                    }
                    seen = mtime(path);
                }
            }
            _ = back.tick() => {
                let now = versions(&h.host.authority());
                if now != written {
                    if mtime(path) != seen {
                        // Take the file's own changes first.
                        if let Ok(c) = load_campaign(path) {
                            campaign = c;
                            h.reconcile(&campaign, |_| None);
                        }
                    }
                    match write_back(&h, &mut campaign, path) {
                        Ok(()) => {
                            written = now;
                            seen = mtime(path);
                        }
                        Err(e) => tracing::warn!("{e:#}"),
                    }
                }
            }
            e = events.recv() => {
                if let Ok(chummer_sync::HostEvent::Membership) = e {
                    let a = h.host.authority();
                    let names: Vec<String> = a.members().iter().filter(|(id, _)| **id != a.gm()).map(|(id, m)| if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() }).collect();
                    tracing::info!("members: {}", names.join(", "));
                }
            }
        }
    }
    tracing::info!("stopping");
    if versions(&h.host.authority()) != written {
        if let Err(e) = write_back(&h, &mut campaign, path) {
            tracing::warn!("{e:#}");
        }
    }
    h.host.save()?;
    node.shutdown().await;
    Ok(())
}

fn versions(a: &Authority) -> Vec<(String, u64)> {
    a.characters().map(|c| (c.0.clone(), a.version(c).unwrap_or(0))).collect()
}

fn write_back(h: &HostedCampaign, campaign: &mut Campaign, path: &Path) -> Result<()> {
    h.write_back(campaign).map_err(anyhow::Error::msg)?;
    campaign.save(path)?;
    h.host.save()?;
    tracing::info!("wrote the characters into {}", path.display());
    Ok(())
}

fn print_summary(a: &Authority, c: &Campaign) {
    let member_name = |id: &chummer_net::EndpointId| a.members().get(id).map(|m| if m.name.is_empty() { id.fmt_short().to_string() } else { m.name.clone() }).unwrap_or_else(|| id.fmt_short().to_string());
    println!("Campaign: {} ({})", c.name, a.campaign());
    println!("GM node:  {}", a.gm());
    println!("Members:");
    for (id, m) in a.members() {
        println!("  {:<20} {:?}  {}", member_name(id), m.role, id);
    }
    println!("Characters:");
    for cid in a.characters() {
        let name = hosted::member_id(cid).and_then(|m| c.member(m)).map(|m| m.name.clone()).unwrap_or_else(|| cid.to_string());
        let owner = a.owner(cid).map(|o| member_name(&o)).unwrap_or_else(|| "GM".into());
        println!("  {:<28} {:<16} v{}  {}", name, owner, a.version(cid).unwrap_or(0), cid);
    }
}

fn invite(path: &Path, key: &SecretKey, role: RoleArg, label: &str, relay: Option<String>) -> Result<()> {
    let campaign = load_campaign(path)?;
    let side = hosted::authority_path(path);
    if side.exists() {
        let a = Authority::load(&side)?;
        if a.gm() != key.public() {
            bail!("{} is hosted with another node key ({}); pass it with --key", path.display(), a.gm().fmt_short());
        }
    }
    let role = match role {
        RoleArg::Player => Role::Player,
        RoleArg::Gm => Role::Gm,
    };
    let token = hosted::append_invite(&hosted::invites_path(path), role, label)?;
    let relay = match relay {
        Some(r) => Some(r.parse::<RelayEntry>().map_err(|e| anyhow::anyhow!("{r}: {e}"))?.url),
        None => OnlineSettings::load().relays.first().and_then(|r| r.parse::<RelayEntry>().ok()).map(|e| e.url).filter(|u| u.as_str().trim_end_matches('/') != DEFAULT_RELAY_URL.trim_end_matches('/')),
    };
    let link = InviteLink { host: key.public(), campaign: hosted::campaign_id(&campaign), invite: Some(token), relay };
    println!("{link}");
    Ok(())
}

fn status(path: &Path, feed: usize) -> Result<()> {
    let campaign = load_campaign(path)?;
    let side = hosted::authority_path(path);
    if !side.exists() {
        println!("{} has not been hosted yet (no {}).", path.display(), side.display());
        return Ok(());
    }
    let a = Authority::load(&side)?;
    print_summary(&a, &campaign);
    println!("Invites: {}", a.invites().iter().count());
    let behind = a.members_behind();
    if !behind.is_empty() {
        println!("Waiting for mail: {}", behind.iter().map(|p| p.fmt_short().to_string()).collect::<Vec<_>>().join(", "));
    }
    println!("Activity:");
    for f in a.feed().iter().rev().take(feed).collect::<Vec<_>>().into_iter().rev() {
        let when = chummer_core::chargen::iso_from_unix(f.at.div_euclid(1000));
        println!("  {} {:<20} {}", when.replace('T', " "), f.character_name, f);
    }
    Ok(())
}

fn assign(path: &Path, member: &str, owner: &str) -> Result<()> {
    let mut campaign = load_campaign(path)?;
    let matches: Vec<MemberId> = campaign.members.iter().filter(|m| m.id.to_string() == member || m.name.eq_ignore_ascii_case(member)).map(|m| m.id).collect();
    let id = match matches.as_slice() {
        [id] => *id,
        [] => bail!("no member {member} in {}", path.display()),
        _ => bail!("several members are called {member}; use the id (`status` lists them)"),
    };
    let owner_text = if owner.eq_ignore_ascii_case(GM_OWNER) {
        GM_OWNER.to_owned()
    } else if let Ok(node) = owner.parse::<chummer_net::EndpointId>() {
        node.to_string()
    } else {
        // A joined member's name.
        let side = hosted::authority_path(path);
        let a = Authority::load(&side).context("no node id given and the campaign was not hosted yet")?;
        let found: Vec<_> = a.members().iter().filter(|(_, m)| m.name.eq_ignore_ascii_case(owner)).map(|(id, _)| *id).collect();
        match found.as_slice() {
            [p] => p.to_string(),
            [] => bail!("{owner} is neither a node id nor a member's name"),
            _ => bail!("several members are called {owner}; use the node id"),
        }
    };
    let m = campaign.member_mut(id).expect("found");
    m.owner = Some(owner_text.clone());
    campaign.save(path)?;
    println!("{} now belongs to {}", campaign.member(id).map(|m| m.name.as_str()).unwrap_or(""), if owner_text == GM_OWNER { "the GM" } else { &owner_text });
    Ok(())
}
