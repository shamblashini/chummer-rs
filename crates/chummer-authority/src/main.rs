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
use chummer_net::invite::{derive_campaign_key, InviteLink, MemberSecret, Role};
use chummer_net::SecretKey;
use chummer_sync::hosted::{self, HostedCampaign, GM_OWNER};
use chummer_sync::invites::{Invite, InviteOp, InviteState};
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
    /// Per-player invites: create, list, revoke, re-issue, remove. A
    /// running host takes changes in within seconds; otherwise the next
    /// host does when it starts.
    Invite {
        #[command(subcommand)]
        cmd: InviteCmd,
    },
    /// Make a new GM campaign key (members get it through normal sync).
    /// Only needed if the old one may be known to someone it should not.
    RotateKey { campaign: PathBuf },
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

#[derive(Subcommand, Debug)]
enum InviteCmd {
    /// A new invite for one player; prints the link to send them. The
    /// first device that joins with it claims it.
    Create {
        campaign: PathBuf,
        /// Who it is for ("Anna"); shown in lists and to the player.
        #[arg(long)]
        label: String,
        /// A character (campaign member name or id) to give the player
        /// when they claim the invite.
        #[arg(long)]
        assign: Option<String>,
        /// How long the link works if nobody claims it: `7d`, `12h`,
        /// `30m`, or `never`.
        #[arg(long, default_value = "never")]
        expires: String,
        #[arg(long, value_enum, default_value_t = RoleArg::Player)]
        role: RoleArg,
        /// The relay to put in the link, for players whose app does not
        /// have it (default: the first configured one, unless it is the
        /// project's relay).
        #[arg(long)]
        relay: Option<String>,
    },
    /// The invites and their state.
    List { campaign: PathBuf },
    /// Revoke an invite (id, id prefix or label): the player is cut off.
    Revoke { campaign: PathBuf, invite: String },
    /// A new link for the same player (a new device); the old one stops
    /// working. Prints the new link.
    Reissue {
        campaign: PathBuf,
        invite: String,
        #[arg(long, default_value = "never")]
        expires: String,
        #[arg(long)]
        relay: Option<String>,
    },
    /// Delete an invite from the list (its player is cut off too).
    Remove { campaign: PathBuf, invite: String },
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
    let key = load_key(args.key.as_deref(), matches!(args.cmd, Cmd::Run { .. }));
    let key = match (&args.cmd, key) {
        (Cmd::Status { .. } | Cmd::Assign { .. } | Cmd::Invite { cmd: InviteCmd::List { .. } }, Err(_)) => SecretKey::generate(),
        (_, k) => k?,
    };
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
        Cmd::Invite { cmd } => invite(cmd, &key),
        Cmd::RotateKey { campaign } => rotate_key(&campaign, &key),
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
    let mut last_refused = 0;
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
                Ok(r) => {
                    let st = r.status.as_ref().map(|s| format!("; mailbox: {} keys, {} waiting, {} refused today", s.keys, s.waiting, s.refused_today)).unwrap_or_default();
                    let refused_now = r.status.as_ref().map(|s| s.refused_today).unwrap_or(0);
                    if r.fetched + r.sent + r.refused > 0 || refused_now != last_refused {
                        tracing::info!("mail: {} read, {} applied, {} dropped, {} joins refused, {} sent{st}", r.fetched, r.handled, r.dropped, r.refused, r.sent);
                    }
                    last_refused = refused_now;
                }
                Err(e) => tracing::warn!("mailbox: {e}"),
            },
            _ = files.tick() => {
                let n = h.merge_invites();
                if n > 0 {
                    tracing::info!("took in {n} invite change(s)");
                    // The relay learns the new set of keys now.
                    if let Err(e) = node.register_mail_keys(&h.host).await {
                        tracing::warn!("mailbox: {e}");
                    }
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
                // A claim gave a character to a player: the file says so too.
                if h.adopt_owner_changes(&mut campaign) {
                    match campaign.save(path) {
                        Ok(()) => seen = mtime(path),
                        Err(e) => tracing::warn!("{e:#}"),
                    }
                }
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

/// `7d`, `12h`, `30m`, `90s` or `never` from now, as Unix seconds.
fn parse_expiry(text: &str) -> Result<Option<u64>> {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() || t == "never" {
        return Ok(None);
    }
    let (n, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len()));
    let n: u64 = n.parse().with_context(|| format!("not a duration: {text} (try 7d, 12h, 30m or never)"))?;
    let secs = match unit {
        "d" => n * 86_400,
        "h" | "" => n * 3_600,
        "m" => n * 60,
        "s" => n,
        _ => bail!("not a duration: {text} (try 7d, 12h, 30m or never)"),
    };
    Ok(Some(now_secs() + secs))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn when(secs: u64) -> String {
    chummer_core::chargen::iso_from_unix(secs as i64).replace('T', " ")
}

/// The GM's sidecar if the campaign was hosted (checked to be this GM's).
fn sidecar(path: &Path, key: &SecretKey) -> Result<Option<Authority>> {
    let side = hosted::authority_path(path);
    if !side.exists() {
        return Ok(None);
    }
    let a = Authority::load(&side)?;
    if a.gm() != key.public() {
        bail!("{} is hosted with another node key ({}); pass it with --key", path.display(), a.gm().fmt_short());
    }
    Ok(Some(a))
}

/// The invites as the next host will see them: the sidecar's, with the
/// changes still waiting in the invites file applied.
fn current_invites(path: &Path, key: &SecretKey) -> Result<(Option<Authority>, Vec<Invite>)> {
    let a = sidecar(path, key)?;
    let mut list: Vec<Invite> = a.as_ref().map(|a| a.invites().values().cloned().collect()).unwrap_or_default();
    for op in hosted::pending_invite_ops(&hosted::invites_path(path)) {
        match op {
            InviteOp::Create { invite } => {
                if !list.iter().any(|i| i.id == invite.id) {
                    list.push(invite);
                }
            }
            InviteOp::Revoke { id } => {
                if let Some(i) = list.iter_mut().find(|i| i.id == id) {
                    i.revoked.get_or_insert(now_secs());
                }
            }
            InviteOp::Reissue { id, secret, expires } => {
                if let Some(i) = list.iter_mut().find(|i| i.id == id) {
                    if i.secret != secret {
                        i.reissue(expires, now_secs());
                        i.secret = secret;
                    }
                }
            }
            InviteOp::Remove { id } => list.retain(|i| i.id != id),
            InviteOp::RotateKey { .. } => {}
        }
    }
    list.sort_by_key(|i| i.created);
    Ok((a, list))
}

/// The invite `which` names: its id, a prefix of it, or its label.
fn find_invite<'a>(list: &'a [Invite], which: &str) -> Result<&'a Invite> {
    let w = which.trim();
    let by_id: Vec<&Invite> = list.iter().filter(|i| w.len() >= 4 && i.id.to_string().starts_with(&w.to_ascii_lowercase())).collect();
    let found: Vec<&Invite> = if by_id.is_empty() { list.iter().filter(|i| i.label.eq_ignore_ascii_case(w)).collect() } else { by_id };
    match found.as_slice() {
        [i] => Ok(i),
        [] => bail!("no invite {which} (`invite list` shows them)"),
        _ => bail!("several invites match {which}; use the id"),
    }
}

fn link_relay(relay: Option<String>) -> Result<Option<chummer_net::RelayUrl>> {
    Ok(match relay {
        Some(r) => Some(r.parse::<RelayEntry>().map_err(|e| anyhow::anyhow!("{r}: {e}"))?.url),
        None => OnlineSettings::load().relays.first().and_then(|r| r.parse::<RelayEntry>().ok()).map(|e| e.url).filter(|u| u.as_str().trim_end_matches('/') != DEFAULT_RELAY_URL.trim_end_matches('/')),
    })
}

fn link_for(campaign: &Campaign, key: &SecretKey, generation: u32, invite: &Invite, relay: Option<chummer_net::RelayUrl>) -> InviteLink {
    let id = hosted::campaign_id(campaign);
    InviteLink { host: key.public(), campaign: id, member: Some(invite.secret.clone()), gm_key: Some(derive_campaign_key(key, id, generation).public()), relay }
}

fn invite(cmd: InviteCmd, key: &SecretKey) -> Result<()> {
    match cmd {
        InviteCmd::Create { campaign: path, label, assign, expires, role, relay } => {
            let campaign = load_campaign(&path)?;
            let (a, _) = current_invites(&path, key)?;
            if label.trim().is_empty() {
                bail!("give the invite a --label (who it is for)");
            }
            let assign = match assign {
                Some(m) => Some(hosted::character_id(find_member(&campaign, &path, &m)?)),
                None => None,
            };
            let role = match role {
                RoleArg::Player => Role::Player,
                RoleArg::Gm => Role::Gm,
            };
            let invite = Invite::new(role, &label, assign, parse_expiry(&expires)?, now_secs());
            hosted::append_invite_op(&hosted::invites_path(&path), &InviteOp::Create { invite: invite.clone() })?;
            let gen = a.as_ref().map(Authority::key_generation).unwrap_or(0);
            println!("{}", link_for(&campaign, key, gen, &invite, link_relay(relay)?));
            eprintln!("invite {} for {}: send the link to that player only; the first device that joins with it claims it", &invite.id.to_string()[..8], invite.label);
        }
        InviteCmd::List { campaign: path } => {
            let campaign = load_campaign(&path)?;
            let (a, list) = current_invites(&path, key)?;
            if list.is_empty() {
                println!("No invites. Make one with `invite create --label <name>`.");
            }
            let now = now_secs();
            for i in &list {
                let state = match i.state(now) {
                    InviteState::Unclaimed { expires: Some(e) } => format!("unclaimed, expires {}", when(e)),
                    InviteState::Unclaimed { expires: None } => "unclaimed".to_owned(),
                    InviteState::Expired => format!("expired {}", i.expires.map(when).unwrap_or_default()),
                    InviteState::Claimed { node, at } => {
                        let name = a.as_ref().and_then(|a| a.members().get(&node)).map(|m| m.name.clone()).filter(|n| !n.is_empty() && *n != i.label);
                        format!("claimed by {}{} on {}", node.fmt_short(), name.map(|n| format!(" ({n})")).unwrap_or_default(), when(at))
                    }
                    InviteState::Revoked { at } => format!("revoked {}", when(at)),
                };
                let seen = match i.state(now) {
                    InviteState::Claimed { node, .. } => a.as_ref().and_then(|a| a.members().get(&node)).and_then(|m| m.last_seen).map(|t| format!("; last seen {}", when(t))).unwrap_or_default(),
                    _ => String::new(),
                };
                let chars: Vec<String> = match (i.state(now), &a) {
                    (InviteState::Claimed { node, .. }, Some(a)) => a.characters().filter(|c| a.owner(c) == Some(node)).map(|c| member_name(&campaign, c)).collect(),
                    _ => i.assign.iter().map(|c| format!("{} (on claim)", member_name(&campaign, c))).collect(),
                };
                let chars = if chars.is_empty() { String::new() } else { format!("; plays {}", chars.join(", ")) };
                println!("{}  {:<16} {state}{seen}{chars}", &i.id.to_string()[..8], i.label);
            }
            if !hosted::pending_invite_ops(&hosted::invites_path(&path)).is_empty() {
                println!("(some changes wait for the host to take them in)");
            }
        }
        InviteCmd::Revoke { campaign: path, invite } => {
            let (_, list) = current_invites(&path, key)?;
            let i = find_invite(&list, &invite)?;
            hosted::append_invite_op(&hosted::invites_path(&path), &InviteOp::Revoke { id: i.id })?;
            println!("revoked {} ({}): their link and device no longer work", &i.id.to_string()[..8], i.label);
        }
        InviteCmd::Reissue { campaign: path, invite, expires, relay } => {
            let campaign = load_campaign(&path)?;
            let (a, list) = current_invites(&path, key)?;
            let mut i = find_invite(&list, &invite)?.clone();
            let secret = MemberSecret::random();
            let expires = parse_expiry(&expires)?;
            hosted::append_invite_op(&hosted::invites_path(&path), &InviteOp::Reissue { id: i.id, secret: secret.clone(), expires })?;
            i.secret = secret;
            let gen = a.as_ref().map(Authority::key_generation).unwrap_or(0);
            println!("{}", link_for(&campaign, key, gen, &i, link_relay(relay)?));
            eprintln!("new link for {}: the old one and the device that used it no longer work", i.label);
        }
        InviteCmd::Remove { campaign: path, invite } => {
            let (_, list) = current_invites(&path, key)?;
            let i = find_invite(&list, &invite)?;
            hosted::append_invite_op(&hosted::invites_path(&path), &InviteOp::Remove { id: i.id })?;
            println!("removed {} ({})", &i.id.to_string()[..8], i.label);
        }
    }
    Ok(())
}

fn rotate_key(path: &Path, key: &SecretKey) -> Result<()> {
    let Some(a) = sidecar(path, key)? else { bail!("{} has not been hosted yet; it has no campaign key to rotate", path.display()) };
    let pending = hosted::pending_invite_ops(&hosted::invites_path(path)).iter().filter_map(|op| if let InviteOp::RotateKey { generation } = op { Some(*generation) } else { None }).max();
    let generation = pending.unwrap_or(a.key_generation()).max(a.key_generation()) + 1;
    hosted::append_invite_op(&hosted::invites_path(path), &InviteOp::RotateKey { generation })?;
    println!("the campaign key moves to generation {generation}; members get it with their next sync");
    Ok(())
}

fn member_name(c: &Campaign, id: &chummer_sync::CharacterId) -> String {
    hosted::member_id(id).and_then(|m| c.member(m)).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
}

fn find_member(campaign: &Campaign, path: &Path, member: &str) -> Result<MemberId> {
    let matches: Vec<MemberId> = campaign.members.iter().filter(|m| m.id.to_string() == member || m.name.eq_ignore_ascii_case(member)).map(|m| m.id).collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => bail!("no member {member} in {}", path.display()),
        _ => bail!("several members are called {member}; use the id (`status` lists them)"),
    }
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
    let now = now_secs();
    let count = |f: fn(&InviteState) -> bool| a.invites().values().filter(|i| f(&i.state(now))).count();
    println!(
        "Invites: {} unclaimed, {} claimed, {} revoked, {} expired (`invite list` for details)",
        count(|s| matches!(s, InviteState::Unclaimed { .. })),
        count(|s| matches!(s, InviteState::Claimed { .. })),
        count(|s| matches!(s, InviteState::Revoked { .. })),
        count(|s| matches!(s, InviteState::Expired))
    );
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
    let id = find_member(&campaign, path, member)?;
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
