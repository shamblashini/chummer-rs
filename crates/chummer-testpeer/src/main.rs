//! `chummer-testpeer`: the headless pieces of the relay/mailbox end-to-end
//! tests (`tests/e2e`). Not shipped.
//!
//! - `keygen`: makes (or reads) a node key and prints its node id.
//! - `make-campaign`: a campaign file with one character per player, owned
//!   by that player, and the link players join with.
//! - `player`: a player built on [`PlayerSession`]: makes a scripted series
//!   of karma edits (some meant to be refused), syncs live or by mail, and
//!   writes its state as JSON every second for the orchestrator to check.
//! - `inspect`: the authority sidecar's state as JSON (versions, hashes).
//! - `abuse`: hostile mailbox traffic (oversized, unsealed, forged,
//!   flooding, unsigned, malformed frames) and what the relay answered;
//!   signed with the abuser's own key, or with a leaked invite link's.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chummer_core::campaign::{Campaign, Member, MemberKind};
use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command};
use chummer_core::engine::Engine;
use chummer_net::config::{read_pem_certs, NetConfig, RelayEntry};
use chummer_net::invite::{derive_campaign_key, InviteLink};
use chummer_net::mailbox::{MailboxClient, MAILBOX_ALPN};
use chummer_net::node::dial_addr;
use chummer_net::{EndpointId, SecretKey};
use chummer_sync::hosted;
use chummer_sync::{Authority, Node, PlayerConfig, PlayerSession};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Debug, Clone)]
struct NetArgs {
    /// Relay entry, `https://relay:3443#<mailbox-id>`.
    #[arg(long)]
    relay: String,
    /// PEM file of the relay's self-signed certificate.
    #[arg(long)]
    ca: Option<PathBuf>,
}

impl NetArgs {
    fn config(&self) -> Result<NetConfig> {
        let entry: RelayEntry = self.relay.parse().map_err(|e| anyhow::anyhow!("{}: {e}", self.relay))?;
        let mut cfg = NetConfig::with_relays([entry]);
        if let Some(ca) = &self.ca {
            cfg.extra_ca_roots = read_pem_certs(ca).map_err(anyhow::Error::msg)?;
        }
        Ok(cfg)
    }

    fn mailbox(&self) -> Result<EndpointId> {
        let entry: RelayEntry = self.relay.parse().map_err(|e| anyhow::anyhow!("{}: {e}", self.relay))?;
        entry.mailbox.context("the relay entry has no #<mailbox-id>")
    }
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print the node id of the key at `key` (made when missing).
    Keygen { key: PathBuf },
    /// Write a campaign with one character (`character`, karma set to
    /// `karma`) per `owner`; print the join link as JSON.
    MakeCampaign {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        character: PathBuf,
        /// A player's node id, or `invite` for a character without an
        /// owner (given to a player through an invite with `--assign`).
        #[arg(long = "owner")]
        owners: Vec<String>,
        #[arg(long)]
        gm_key: PathBuf,
        #[arg(long, default_value_t = 50)]
        karma: i32,
    },
    /// A scripted player.
    Player {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        link: String,
        #[command(flatten)]
        net: NetArgs,
        /// Directory for the replica and the progress file.
        #[arg(long)]
        state: PathBuf,
        /// Where the JSON status is written (every second).
        #[arg(long)]
        status: PathBuf,
        #[arg(long, default_value = "Player")]
        name: String,
        /// Edits to make in all (counted across restarts).
        #[arg(long, default_value_t = 20)]
        edits: u64,
        /// Milliseconds between edits.
        #[arg(long, default_value_t = 500)]
        every_ms: u64,
        /// Seconds between syncs when not connected.
        #[arg(long, default_value_t = 3)]
        sync_secs: u64,
        /// Re-mail unanswered commands after this many seconds.
        #[arg(long)]
        remail_secs: Option<u64>,
        /// Bytes of notes text added to every 5th edit (large ops).
        #[arg(long, default_value_t = 0)]
        big: usize,
    },
    /// The authority sidecar as JSON.
    Inspect { sidecar: PathBuf },
    /// Times state_hash, snapshot and restore on a character (what the
    /// authority does per change and per save).
    Bench { character: PathBuf },
    /// Hostile mailbox traffic.
    Abuse {
        #[arg(long)]
        key: PathBuf,
        #[command(flatten)]
        net: NetArgs,
        /// Who the mail is for (the GM's node id).
        #[arg(long)]
        target: String,
        #[arg(long, value_enum)]
        mode: Abuse,
        /// For `flood`: how many puts at most.
        #[arg(long, default_value_t = 100)]
        count: u32,
        /// For `forged`: the member to pose as.
        #[arg(long)]
        pose_as: Option<String>,
        /// Sign puts with this invite link's member key (a leaked link)
        /// instead of this node's key.
        #[arg(long)]
        signer_link: Option<String>,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Abuse {
    /// One blob larger than the relay allows.
    Oversized,
    /// Bytes that are not a sealed blob.
    Garbage,
    /// Sealed and signed by this key, which is not a member; or, with
    /// --pose-as, claiming another member's id inside a validly sealed
    /// envelope signed by this key.
    Forged,
    /// Sealed by this (member) key but not a sync message.
    MemberGarbage,
    /// Many puts, until the relay refuses.
    Flood,
    /// A put without a signature.
    Unsigned,
    /// Broken frames on raw mailbox streams; then a normal request.
    Frames,
}

fn engine() -> Result<Arc<Engine>> {
    Ok(Arc::new(Engine::load().context("game data (set CHUMMER_RESOURCES)")?))
}

fn load_key(path: &Path) -> Result<SecretKey> {
    chummer_net::identity::load_or_create(path).with_context(|| format!("node key {}", path.display()))
}

fn write_json(path: &Path, v: &impl Serialize) -> Result<()> {
    let text = serde_json::to_string_pretty(v)?;
    chummer_sync::persist::write_atomic(path, text.as_bytes())?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,iroh=warn,iroh_relay=warn".into()))
        .with_writer(std::io::stderr)
        .init();
    match Args::parse().cmd {
        Cmd::Keygen { key } => {
            println!("{}", load_key(&key)?.public());
            Ok(())
        }
        Cmd::MakeCampaign { out, character, owners, gm_key, karma } => make_campaign(&out, &character, &owners, &gm_key, karma),
        Cmd::Player { key, link, net, state, status, name, edits, every_ms, sync_secs, remail_secs, big } => {
            let script = Script { edits, every: Duration::from_millis(every_ms), sync: Duration::from_secs(sync_secs.max(1)), remail: remail_secs.map(Duration::from_secs), big };
            player(&key, &link, &net, &state, &status, &name, script).await
        }
        Cmd::Inspect { sidecar } => inspect(&sidecar),
        Cmd::Bench { character } => {
            let ch = Character::load(&character)?;
            let t = std::time::Instant::now();
            let h = command::state_hash(&ch);
            let hash = t.elapsed();
            let t = std::time::Instant::now();
            let snap = command::snapshot(&ch);
            let snapshot = t.elapsed();
            let t = std::time::Instant::now();
            let back = command::restore(&snap)?;
            let restore = t.elapsed();
            assert_eq!(command::state_hash(&back), h);
            println!("{}", serde_json::json!({ "hash_ms": hash.as_millis(), "snapshot_ms": snapshot.as_millis(), "restore_ms": restore.as_millis(), "snapshot_bytes": snap.len() }));
            Ok(())
        }
        Cmd::Abuse { key, net, target, mode, count, pose_as, signer_link } => abuse(&key, &net, &target, mode, count, pose_as.as_deref(), signer_link.as_deref()).await,
    }
}

fn make_campaign(out: &Path, character: &Path, owners: &[String], gm_key: &Path, karma: i32) -> Result<()> {
    let gm_secret = load_key(gm_key)?;
    let gm = gm_secret.public();
    let mut ch = Character::load(character).with_context(|| format!("loading {}", character.display()))?;
    ch.karma = karma;
    let mut c = Campaign::new("e2e campaign");
    let mut members = Vec::new();
    for (i, o) in owners.iter().enumerate() {
        let mut m = Member::embedded(MemberKind::Player, &ch);
        m.name = format!("PC {i}");
        m.player = format!("P{i}");
        if o != "invite" {
            let owner: EndpointId = o.parse().with_context(|| format!("owner {o}"))?;
            m.owner = Some(owner.to_string());
        }
        members.push(serde_json::json!({ "member": c.add(m).to_string(), "owner": o }));
    }
    c.save(out)?;
    // Players added by node id join with a link without a member key; it
    // carries the GM's campaign key for their mailbox.
    let campaign = hosted::campaign_id(&c);
    let link = InviteLink { host: gm, campaign, member: None, gm_key: Some(derive_campaign_key(&gm_secret, campaign, 0).public()), relay: None };
    println!("{}", serde_json::json!({ "link": link.to_string(), "gm": gm.to_string(), "members": members }));
    Ok(())
}

struct Script {
    edits: u64,
    every: Duration,
    sync: Duration,
    remail: Option<Duration>,
    big: usize,
}

/// What the player has done, kept across restarts.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Progress {
    /// Edits made (accepted locally).
    made: u64,
    /// The karma they added in all.
    gained: i64,
}

#[derive(Debug, Serialize)]
struct CopyStatus {
    id: String,
    version: u64,
    hash: String,
    /// Karma of the confirmed state.
    karma: i32,
    /// Karma shown (outbox applied).
    shown_karma: i32,
    outbox: usize,
    needs_resync: bool,
}

#[derive(Debug, Serialize)]
struct PlayerStatus {
    name: String,
    id: String,
    /// The invite's label the GM sent, once joined.
    label: Option<String>,
    /// Why the GM refused us, if it did.
    denied: Option<String>,
    mode: Option<String>,
    online: bool,
    progress: Progress,
    done: bool,
    refused: usize,
    refused_reasons: Vec<String>,
    copies: Vec<CopyStatus>,
    syncs: u64,
    errors: Vec<String>,
}

fn expense(gain: bool, amount: f64, reason: String) -> Command {
    Command::ManualExpense { karma: true, gain, expense: ManualExpense { amount, reason, ..Default::default() } }
}

async fn player(key: &Path, link: &str, net: &NetArgs, state: &Path, status: &Path, name: &str, script: Script) -> Result<()> {
    std::fs::create_dir_all(state)?;
    let engine = engine()?;
    let secret = load_key(key)?;
    let link: InviteLink = link.parse().map_err(|e| anyhow::anyhow!("{link}: {e}"))?;
    let node = Node::start(secret.clone(), net.config()?).await?;
    let mut cfg = PlayerConfig::new(name, link);
    cfg.mailbox = Some(net.mailbox()?);
    cfg.path = Some(state.join("campaign.replica"));
    cfg.connect_timeout = Duration::from_secs(10);
    if let Some(r) = script.remail {
        cfg.remail_after = r;
    }
    let session = PlayerSession::new(node.endpoint().clone(), secret.clone(), engine, cfg)?;
    let progress_path = state.join("progress.json");
    let mut progress: Progress = std::fs::read_to_string(&progress_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    tracing::info!("player {name} {} starting at {progress:?}", secret.public());

    let syncer = tokio::spawn(session.clone().keep_synced(script.sync));
    let mut edit_tick = tokio::time::interval(script.every);
    let mut status_tick = tokio::time::interval(Duration::from_secs(1));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut errors = Vec::new();
    let mut syncs = 0u64;
    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            _ = edit_tick.tick() => {
                if progress.made >= script.edits {
                    continue;
                }
                let Some(c) = session.replica().characters().next().cloned() else { continue };
                let n = progress.made + 1;
                let amount = (n % 5 + 1) as i64;
                let mut reason = format!("op {n}");
                if script.big > 0 && n.is_multiple_of(5) {
                    reason.push(' ');
                    reason.push_str(&"x".repeat(script.big));
                }
                let r = session.edit_now(&c, expense(true, amount as f64, reason)).map(|r| r.changed).map_err(|e| e.reason);
                match r {
                    Ok(true) => {
                        progress.made = n;
                        progress.gained += amount;
                        write_json(&progress_path, &progress)?;
                    }
                    Ok(false) => errors.push(format!("op {n} changed nothing")),
                    Err(e) => errors.push(format!("op {n}: {e}")),
                }
            }
            _ = status_tick.tick() => {
                syncs += 1;
                for e in session.events() {
                    if let chummer_sync::Event::Error(e) = e {
                        errors.push(e);
                    }
                }
                errors.truncate(50);
                let st = status_of(&session, name, &progress, script.edits, syncs, &errors);
                write_json(status, &st)?;
            }
        }
    }
    session.close();
    syncer.abort();
    let _ = session.save();
    write_json(status, &status_of(&session, name, &progress, script.edits, syncs, &errors))?;
    node.shutdown().await;
    Ok(())
}

fn status_of(s: &PlayerSession, name: &str, progress: &Progress, edits: u64, syncs: u64, errors: &[String]) -> PlayerStatus {
    let r = s.replica();
    let copies = r
        .characters()
        .map(|c| CopyStatus {
            id: c.to_string(),
            version: r.version(c).unwrap_or(0),
            hash: r.confirmed_hash(c).map(|h| command::hex(&h)).unwrap_or_default(),
            karma: r.confirmed(c).map(|ch| ch.karma).unwrap_or(0),
            shown_karma: r.character(c).map(|ch| ch.karma).unwrap_or(0),
            outbox: r.outbox(c).len(),
            needs_resync: r.needs_resync(c),
        })
        .collect();
    let denied = r.denied().map(|d| format!("{d:?}"));
    drop(r);
    let label = s.label();
    let r = s.replica();
    PlayerStatus {
        name: name.to_owned(),
        id: r.me().map(|m| m.to_string()).unwrap_or_default(),
        label,
        denied,
        mode: s.last_mode().map(|m| format!("{m:?}")),
        online: s.is_online(),
        progress: progress.clone(),
        done: progress.made >= edits,
        refused: r.refused().len(),
        refused_reasons: r.refused().iter().map(|x| x.reason.clone()).collect(),
        copies,
        syncs,
        errors: errors.to_vec(),
    }
}

fn inspect(sidecar: &Path) -> Result<()> {
    let a = Authority::load(sidecar).with_context(|| format!("reading {}", sidecar.display()))?;
    let chars: Vec<_> = a
        .characters()
        .map(|c| {
            serde_json::json!({
                "id": c.to_string(),
                "version": a.version(c),
                "hash": a.hash(c).map(|h| command::hex(&h)),
                "karma": a.character(c).map(|ch| ch.karma),
                "owner": a.owner(c).map(|o| o.to_string()),
                "log": a.log(c).len(),
            })
        })
        .collect();
    let members: Vec<_> = a.members().iter().map(|(id, m)| serde_json::json!({ "id": id.to_string(), "name": m.name, "role": format!("{:?}", m.role) })).collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let invites: Vec<_> = a.invites().values().map(|i| serde_json::json!({ "id": i.id.to_string(), "label": i.label, "state": format!("{:?}", i.state(now)) })).collect();
    let behind: Vec<String> = a.members_behind().iter().map(|p| p.to_string()).collect();
    let refused = a.feed().iter().filter(|f| f.rejected.is_some()).count();
    println!("{}", serde_json::json!({ "characters": chars, "members": members, "invites": invites, "behind": behind, "feed": a.feed().len(), "refused": refused }));
    Ok(())
}

async fn abuse(key: &Path, net: &NetArgs, target: &str, mode: Abuse, count: u32, pose_as: Option<&str>, signer_link: Option<&str>) -> Result<()> {
    let secret = load_key(key)?;
    let signer = match signer_link {
        Some(l) => l.parse::<InviteLink>().map_err(|e| anyhow::anyhow!("{l}: {e}"))?.member.context("the link has no member key")?.key(),
        None => secret.clone(),
    };
    let target: EndpointId = target.parse().context("target node id")?;
    let ep = chummer_net::node::bind(secret.clone(), &net.config()?, vec![]).await?;
    tokio::time::timeout(Duration::from_secs(20), ep.online()).await.context("relay not reachable")?;
    let mb = tokio::time::timeout(Duration::from_secs(20), MailboxClient::connect(&ep, dial_addr(net.mailbox()?, None))).await.context("mailbox timed out")??;
    let mut results = Vec::new();
    fn res(r: Result<u64, chummer_net::NetError>) -> serde_json::Value {
        match r {
            Ok(id) => serde_json::json!({ "stored": id }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    }
    match mode {
        Abuse::Oversized => results.push(res(mb.put(target, vec![0x55; 300 * 1024], &signer).await)),
        Abuse::Garbage => results.push(res(mb.put(target, b"definitely not a sealed box".to_vec(), &signer).await)),
        Abuse::Unsigned => results.push(res(mb.put_with(target, b"no signature".to_vec(), None).await)),
        Abuse::Forged => {
            // A well-formed sync message, signed by a key that is not a
            // member (or claiming to be one inside the envelope).
            let payload = forged_payload(pose_as)?;
            results.push(res(mb.put_sealed(&secret, &signer, target, &payload).await));
        }
        Abuse::MemberGarbage => results.push(res(mb.put_sealed(&secret, &signer, target, b"\x01garbage that is not a chunk").await)),
        Abuse::Flood => {
            for _ in 0..count {
                let r = mb.put_sealed(&secret, &signer, target, &[0u8; 1024]).await;
                let stop = r.is_err();
                results.push(res(r));
                if stop {
                    break;
                }
            }
        }
        Abuse::Frames => {
            let conn = ep.connect(dial_addr(net.mailbox()?, None), MAILBOX_ALPN).await.map_err(|e| anyhow::anyhow!("{e}"))?;
            let bad: Vec<Vec<u8>> = vec![
                u32::MAX.to_be_bytes().to_vec(),                // absurd length
                vec![0, 0, 0, 3, 9, 9, 9],                      // unknown version
                vec![0, 0, 0, 10, 1, 2],                        // truncated
                vec![0, 0, 0, 0],                               // empty frame
                vec![0, 0, 0, 5, 1, 0xff, 0xff, 0xff, 0xff],     // bad postcard
            ];
            for (i, b) in bad.into_iter().enumerate() {
                let r: Result<String> = async {
                    let (mut send, mut recv) = conn.open_bi().await?;
                    send.write_all(&b).await?;
                    send.finish()?;
                    let got = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(1 << 20)).await;
                    Ok(match got {
                        Ok(Ok(v)) => format!("{} bytes back", v.len()),
                        Ok(Err(e)) => format!("stream error: {e}"),
                        Err(_) => "no answer".into(),
                    })
                }
                .await;
                results.push(serde_json::json!({ "frame": i, "result": format!("{r:?}") }));
            }
            // A normal request still works afterwards.
            results.push(res(mb.put_sealed(&secret, &signer, target, b"\x01after the bad frames").await));
        }
    }
    println!("{}", serde_json::json!({ "mode": format!("{mode:?}"), "results": results }));
    mb.close();
    ep.close().await;
    Ok(())
}

/// A Submit for a made-up character, framed as mail chunk.
fn forged_payload(pose_as: Option<&str>) -> Result<Vec<u8>> {
    use chummer_sync::msg::{self, ClientMessage, MailMessage, SubmitBatch};
    let batch = SubmitBatch { character: chummer_sync::CharacterId::new(pose_as.unwrap_or("nobody")), base_version: 0, base_hash: [0; 32], ops: Vec::new() };
    let m = MailMessage::Client(ClientMessage::Submit(batch));
    let parts = chummer_sync::mail::split(&m, chummer_sync::mail::DEFAULT_BLOB_LIMIT);
    if parts.len() != 1 {
        bail!("forged message does not fit one blob");
    }
    let _ = msg::encode(&m);
    Ok(parts.into_iter().next().expect("one part"))
}
