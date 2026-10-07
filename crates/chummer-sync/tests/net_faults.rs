//! Sync over real connections when things go wrong: the relay restarts
//! mid-game, mail is lost at the relay, a member stops reading. In process,
//! through a local relay with a self-signed certificate.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_net::campaign::{CampaignClient, Hello, CAMPAIGN_ALPN, PROTOCOL_VERSION};
use chummer_net::invite::{CampaignId, InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use chummer_sync::msg::{self, ClientMessage};
use chummer_sync::{Authority, AuthorityHost, CharacterId, PlayerConfig, PlayerSession, SyncMode};
use iroh::protocol::Router;

const T0: u64 = 1_800_000_000;
const WAIT: Duration = Duration::from_secs(30);

fn engine() -> Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data"))).clone()
}

fn munin(karma: i32) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
    let mut ch = Character::load(&p).unwrap();
    ch.karma = karma;
    ch
}

fn gain(amount: f64, reason: &str) -> Command {
    Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount, reason: reason.into(), ..Default::default() } }
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-sync-faults-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn free_tcp() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn free_udp() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A relay config on fixed ports, so it can be restarted at the same URL.
fn relay_config(name: &str) -> Config {
    let mut cfg = Config {
        hostname: "127.0.0.1".into(),
        data_dir: tmp(&format!("relay-{name}")),
        http_bind: format!("127.0.0.1:{}", free_tcp()).parse().unwrap(),
        https_bind: format!("127.0.0.1:{}", free_tcp()).parse().unwrap(),
        qad_bind: format!("127.0.0.1:{}", free_udp()).parse().unwrap(),
        mailbox_port: 0,
        ..Config::default()
    };
    cfg.tls.cert_mode = CertMode::SelfSigned;
    cfg
}

async fn start(cfg: &Config, clock: Arc<ManualClock>) -> Result<RelayNode> {
    let node = RelayNode::spawn(cfg.clone(), clock).await?;
    node.wait_online(WAIT).await?;
    Ok(node)
}

async fn endpoint(relay: &RelayNode, key: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let ep = bind(key, &relay.client_config(), alpns).await?;
    tokio::time::timeout(WAIT, ep.online()).await?;
    Ok(ep)
}

/// Syncs until `ok` or the time is up.
async fn sync_until(p: &PlayerSession, ok: impl Fn(&PlayerSession) -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(90), async {
        while !ok(p) {
            p.sync().await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await?;
    Ok(())
}

struct Pbp {
    relay_cfg: Config,
    clock: Arc<ManualClock>,
    relay: RelayNode,
    gm_ep: Endpoint,
    host: AuthorityHost,
    player: PlayerSession,
    p_ep: Endpoint,
    c: CharacterId,
}

/// A play-by-post campaign: the GM serves nothing live, only mail.
async fn pbp(name: &str, tweak: impl FnOnce(&mut Config, &mut PlayerConfig)) -> Result<Pbp> {
    let mut relay_cfg = relay_config(name);
    let gm_key = SecretKey::generate();
    let p_key = SecretKey::generate();
    let link = InviteLink { host: gm_key.public(), campaign: CampaignId([3; 16]), invite: None, relay: None };
    let mut pcfg = PlayerConfig::new("Alice", link);
    pcfg.connect_timeout = Duration::from_secs(3);
    pcfg.path = Some(tmp(&format!("{name}-p")).join("campaign.replica"));
    tweak(&mut relay_cfg, &mut pcfg);
    let clock = Arc::new(ManualClock::new(T0));
    let relay = start(&relay_cfg, clock.clone()).await?;
    pcfg.mailbox = relay.mailbox_id();
    let engine = engine();
    let gm_ep = endpoint(&relay, gm_key.clone(), vec![]).await?;
    let p_ep = endpoint(&relay, p_key.clone(), vec![]).await?;
    let mut auth = Authority::new(CampaignId([3; 16]), gm_ep.id(), "GM");
    auth.add_member(p_ep.id(), Role::Player, "Alice");
    let c = CharacterId::new("alice");
    auth.add_character(c.clone(), Some(p_ep.id()), munin(10))?;
    let host = AuthorityHost::new(auth, engine.clone(), gm_key, None);
    let player = PlayerSession::new(p_ep.clone(), p_key, engine, pcfg)?;
    Ok(Pbp { relay_cfg, clock, relay, gm_ep, host, player, p_ep, c })
}

impl Pbp {
    async fn gm_mail(&self) -> Result<chummer_sync::MailReport> {
        let mb = tokio::time::timeout(WAIT, MailboxClient::connect(&self.gm_ep, dial_addr(self.relay.mailbox_id().unwrap(), None))).await??;
        let r = self.host.sync_mail(&mb).await?;
        mb.close();
        Ok(r)
    }

    fn converged(&self) -> bool {
        let r = self.player.replica();
        let a = self.host.authority();
        r.outbox(&self.c).is_empty() && r.version(&self.c) == a.version(&self.c) && r.confirmed_hash(&self.c) == a.hash(&self.c)
    }

    async fn close(self) -> Result<()> {
        self.player.close();
        self.p_ep.close().await;
        self.gm_ep.close().await;
        self.relay.shutdown().await
    }
}

/// Play-by-post goes on across a restart of the relay: the player's cached
/// mailbox connection breaks, the next sync reconnects, nothing is lost or
/// applied twice.
#[tokio::test(flavor = "multi_thread")]
async fn play_by_post_survives_a_relay_restart() -> Result<()> {
    let mut t = pbp("restart", |_, _| {}).await?;
    let c = t.c.clone();
    t.gm_mail().await?;
    assert_eq!(t.player.sync().await, SyncMode::Mailbox);
    t.player.edit(&c, gain(1.0, "before")).await?;
    assert_eq!(t.player.sync().await, SyncMode::Mailbox);

    // The relay restarts with the mail on disk.
    t.relay.shutdown().await?;
    t.relay = start(&t.relay_cfg, t.clock.clone()).await?;
    t.player.edit(&c, gain(2.0, "during")).await?;
    // The player's next sync may fail on the dead connection; it must
    // reconnect by the one after.
    let _ = t.player.sync().await;
    sync_until(&t.player, |p| p.replica().outbox(&c).iter().all(|o| o.mailed)).await?;
    t.gm_mail().await?;
    sync_until(&t.player, |_| t.converged()).await?;
    assert_eq!(t.host.authority().character(&c).unwrap().karma, 13);
    assert_eq!(t.player.replica().character(&c).unwrap().karma, 13);
    assert_eq!(t.host.authority().version(&c), Some(2), "each edit ran once");
    t.close().await
}

/// Mail that never reaches the GM (expired at the relay, or a relay that
/// lost its data) used to leave the player's commands marked as mailed
/// forever: they were never sent again unless the GM came online live.
#[tokio::test(flavor = "multi_thread")]
async fn mail_lost_at_the_relay_is_sent_again() -> Result<()> {
    let mut t = pbp("lost", |r, p| {
        r.limits.expiry_secs = 3600;
        p.remail_after = Duration::from_secs(1);
    })
    .await?;
    let c = t.c.clone();
    t.gm_mail().await?;
    assert_eq!(t.player.sync().await, SyncMode::Mailbox);
    t.player.edit(&c, gain(4.0, "lost in the mail")).await?;
    assert_eq!(t.player.sync().await, SyncMode::Mailbox);
    assert!(t.player.replica().outbox(&c)[0].mailed);
    // The GM does not look for two hours; the relay drops the mail.
    t.clock.advance(7200);
    let r = t.gm_mail().await?;
    assert_eq!(r.handled, 0, "{r:?}");
    t.clock.advance(1);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // The player's later rounds send it again; this time it arrives.
    assert_eq!(t.player.sync().await, SyncMode::Mailbox);
    let r = t.gm_mail().await?;
    assert!(r.handled >= 1, "{r:?}");
    sync_until(&t.player, |_| t.converged()).await?;
    assert_eq!(t.host.authority().character(&c).unwrap().karma, 14);
    let _ = &mut t;
    t.close().await
}

/// One member whose app stops reading (hung, or far behind) must not stop
/// live pushes to everyone else.
#[tokio::test(flavor = "multi_thread")]
async fn a_member_that_stops_reading_does_not_stall_the_others() -> Result<()> {
    let relay_cfg = relay_config("stall");
    let relay = start(&relay_cfg, Arc::new(ManualClock::new(T0))).await?;
    let engine = engine();
    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone(), vec![]).await?;
    let (a_key, b_key) = (SecretKey::generate(), SecretKey::generate());
    let mut auth = Authority::new(CampaignId([4; 16]), gm_ep.id(), "GM");
    auth.add_member(a_key.public(), Role::Player, "Hung");
    auth.add_member(b_key.public(), Role::Player, "Fine");
    let (ca, cb) = (CharacterId::new("a"), CharacterId::new("b"));
    auth.add_character(ca.clone(), Some(a_key.public()), munin(10))?;
    auth.add_character(cb.clone(), Some(b_key.public()), munin(10))?;
    let host = AuthorityHost::new(auth, engine.clone(), gm_key, None);
    let router = Router::builder(gm_ep.clone()).accept(CAMPAIGN_ALPN, host.protocol()).spawn();
    let link = InviteLink { host: gm_ep.id(), campaign: CampaignId([4; 16]), invite: None, relay: None };

    // A joins with a raw client and never reads its pushes.
    let a_ep = endpoint(&relay, a_key, vec![]).await?;
    let hello = Hello { campaign_id: link.campaign, invite_token: None, client_version: PROTOCOL_VERSION };
    let (a_client, _a_pushes) = CampaignClient::join(&a_ep, dial_addr(gm_ep.id(), None), hello).await?;
    a_client.submit(msg::encode(&ClientMessage::Join { name: "Hung".into(), have: Vec::new() })).await?;
    // B is a normal player.
    let b_ep = endpoint(&relay, b_key.clone(), vec![]).await?;
    let b = PlayerSession::new(b_ep.clone(), b_key, engine.clone(), PlayerConfig::new("Fine", link))?;
    b.connect().await?;

    // The GM edits A's character many times: A's streams pile up.
    for i in 0..400 {
        host.gm_edit(&ca, Command::SetField { key: "alias".into(), value: format!("a{i}") }).map_err(|e| anyhow::anyhow!(e.reason))?;
        if i % 50 == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    // Then B's: B must get it promptly.
    host.gm_edit(&cb, gain(7.0, "for B")).map_err(|e| anyhow::anyhow!(e.reason))?;
    let got = tokio::time::timeout(Duration::from_secs(30), async {
        while b.replica().version(&cb) != host.authority().version(&cb) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(got.is_ok(), "B never got the push (version {:?})", b.replica().version(&cb));
    b.close();
    a_client.close();
    router.shutdown().await?;
    for ep in [a_ep, b_ep, gm_ep] {
        ep.close().await;
    }
    relay.shutdown().await
}
