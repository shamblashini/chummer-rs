//! Sync over real chummer-net connections through an in-process relay with
//! a self-signed certificate and its mailbox. No internet needed.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_net::campaign::CAMPAIGN_ALPN;
use chummer_net::invite::{CampaignId, InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use chummer_sync::{Authority, AuthorityHost, CharacterId, Event, PlayerConfig, PlayerSession, SyncMode};
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
    let d = std::env::temp_dir().join(format!("chummer-sync-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

async fn relay(name: &str, tweak: impl FnOnce(&mut Config)) -> Result<RelayNode> {
    let mut cfg = Config {
        hostname: "127.0.0.1".into(),
        data_dir: tmp(&format!("relay-{name}")),
        http_bind: "127.0.0.1:0".parse()?,
        https_bind: "127.0.0.1:0".parse()?,
        qad_bind: "127.0.0.1:0".parse()?,
        mailbox_port: 0,
        ..Config::default()
    };
    cfg.tls.cert_mode = CertMode::SelfSigned;
    tweak(&mut cfg);
    let node = RelayNode::spawn(cfg, Arc::new(ManualClock::new(T0))).await?;
    node.wait_online(WAIT).await?;
    Ok(node)
}

async fn endpoint(relay: &RelayNode, key: SecretKey, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let ep = bind(key, &relay.client_config(), alpns).await?;
    tokio::time::timeout(WAIT, ep.online()).await?;
    Ok(ep)
}

async fn until(what: impl Fn() -> bool) -> Result<()> {
    tokio::time::timeout(WAIT, async {
        while !what() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    Ok(())
}

async fn wait_for(s: &PlayerSession, what: impl Fn(&Event) -> bool) -> Result<Event> {
    tokio::time::timeout(WAIT, async {
        loop {
            match s.next_event().await {
                Some(e) if what(&e) => return Ok(e),
                Some(_) => {}
                None => anyhow::bail!("session ended"),
            }
        }
    })
    .await?
}

#[tokio::test(flavor = "multi_thread")]
async fn live_campaign_through_a_local_relay() -> Result<()> {
    let relay = relay("live", |_| {}).await?;
    let engine = engine();
    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone(), vec![]).await?;
    let mut auth = Authority::new(CampaignId::random(), gm_ep.id(), "GM");
    let npc = CharacterId::new("npc");
    auth.add_character(npc.clone(), None, munin(5))?;
    let host = AuthorityHost::new(auth, engine.clone(), gm_key, Some(tmp("live-gm").join("campaign.authority")));
    let router = Router::builder(gm_ep.clone()).accept(CAMPAIGN_ALPN, host.protocol()).spawn();
    let link = host.invite(Role::Player, "group", None);
    let link: InviteLink = link.to_string().parse()?;

    // The player joins with the invite; the GM then gives them a character.
    let p_key = SecretKey::generate();
    let p_key_bytes = p_key.to_bytes();
    let p_ep = endpoint(&relay, p_key.clone(), vec![]).await?;
    let p_file = tmp("live-p").join("campaign.replica");
    let mut cfg = PlayerConfig::new("Alice", link.clone());
    cfg.path = Some(p_file.clone());
    let player = PlayerSession::new(p_ep.clone(), p_key, engine.clone(), cfg)?;
    assert_eq!(player.connect().await?, Role::Player);
    assert!(player.is_online());
    assert_eq!(player.replica().characters().count(), 0, "the GM's NPC is not shown");
    assert_eq!(host.connected(), [(p_ep.id(), Role::Player)]);

    let mine = CharacterId::new("alice-munin");
    host.authority().add_character(mine.clone(), Some(p_ep.id()), munin(20))?;
    host.changed();
    wait_for(&player, |e| *e == Event::Updated(mine.clone())).await?;
    assert_eq!(player.replica().version(&mine), Some(0));

    // A player edit goes to the authority at once.
    player.edit(&mine, Command::SetField { key: "alias".into(), value: "Raven".into() }).await?;
    assert_eq!(host.authority().character(&mine).unwrap().field("alias"), "Raven");
    assert_eq!(player.replica().outbox(&mine).len(), 0);
    assert_eq!(player.replica().version(&mine), Some(1));

    // A GM edit is pushed to the player.
    player.events();
    let r = host.gm_edit(&mine, gain(100.0, "Good run"))?;
    assert_eq!(r.notify, [p_ep.id()]);
    wait_for(&player, |e| *e == Event::Updated(mine.clone())).await?;
    assert_eq!(player.replica().character(&mine).unwrap().karma, 120);
    assert_eq!(player.replica().confirmed_hash(&mine), host.authority().hash(&mine));
    assert_eq!(player.replica().feed().back().unwrap().to_string(), "GM: Gained 100 karma: Good run");
    let feed: Vec<String> = host.authority().feed().iter().map(|f| f.to_string()).collect();
    assert_eq!(feed.len(), 2, "{feed:?}");
    assert_eq!(feed[1], "GM: Gained 100 karma: Good run");
    assert!(feed[0].starts_with("Alice: "));

    // Rejoining without the token (a known member) works, and the session
    // persisted what it has.
    player.disconnect();
    let mut again = PlayerConfig::new("Alice", InviteLink { invite: None, ..link });
    again.path = Some(p_file);
    let second = PlayerSession::new(p_ep.clone(), SecretKey::from_bytes(&p_key_bytes), engine.clone(), again)?;
    assert_eq!(second.replica().version(&mine), Some(2), "loaded from disk");
    assert_eq!(second.connect().await?, Role::Player);
    assert_eq!(second.replica().confirmed_hash(&mine), host.authority().hash(&mine));
    second.disconnect();
    host.save()?;
    router.shutdown().await?;
    p_ep.close().await;
    relay.shutdown().await
}

#[tokio::test(flavor = "multi_thread")]
async fn play_by_post_through_the_mailbox() -> Result<()> {
    // A small blob limit, so snapshots are cut into chunks.
    let relay = relay("mail", |c| c.limits.max_blob_bytes = 16 * 1024).await?;
    let mailbox_id = relay.mailbox_id().expect("mailbox");
    let engine = engine();

    // The GM's app is "offline" for campaigns: it serves no campaign ALPN,
    // and only collects mail now and then.
    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone(), vec![]).await?;
    let p_key = SecretKey::generate();
    let p_ep = endpoint(&relay, p_key.clone(), vec![]).await?;
    let mut auth = Authority::new(CampaignId::random(), gm_ep.id(), "GM");
    auth.add_member(p_ep.id(), Role::Player, "Alice");
    let c = CharacterId::new("alice-munin");
    auth.add_character(c.clone(), Some(p_ep.id()), munin(10))?;
    let gm_file = tmp("mail-gm").join("campaign.authority");
    let host = AuthorityHost::new(auth, engine.clone(), gm_key.clone(), Some(gm_file.clone()));
    let gm_mail = MailboxClient::connect(&gm_ep, dial_addr(mailbox_id, None)).await?;

    // 1. The GM mails the player their character.
    let r = host.sync_mail(&gm_mail).await?;
    assert!(r.sent > 2, "membership plus a chunked snapshot: {r:?}");

    // 2. The player cannot reach the GM, so sync uses the mailbox.
    let link = InviteLink { host: gm_ep.id(), campaign: host.authority().campaign(), invite: None, relay: None };
    let p_file = tmp("mail-p").join("campaign.replica");
    let mut cfg = PlayerConfig::new("Alice", link.clone());
    cfg.mailbox = Some(mailbox_id);
    cfg.path = Some(p_file.clone());
    cfg.connect_timeout = Duration::from_secs(5);
    let player = PlayerSession::new(p_ep.clone(), p_key.clone(), engine.clone(), cfg.clone())?;
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    assert!(!player.is_online());
    assert_eq!(player.replica().version(&c), Some(0));
    assert_eq!(player.replica().confirmed_hash(&c), host.authority().hash(&c));

    // 3. Offline edits, kept across a restart of the player's app.
    player.edit(&c, gain(5.0, "Side job")).await?;
    player.edit(&c, Command::SetField { key: "alias".into(), value: "Raven".into() }).await?;
    drop(player);
    let player = PlayerSession::new(p_ep.clone(), p_key.clone(), engine.clone(), cfg.clone())?;
    assert_eq!(player.replica().outbox(&c).len(), 2);
    assert_eq!(player.sync().await, SyncMode::Mailbox, "the outbox goes to the mailbox");
    assert!(player.replica().outbox(&c).iter().all(|p| p.mailed));
    // Mailing again sends nothing new.
    assert_eq!(player.send_mail().await?, 0);

    // A stranger's mail to the GM is dropped.
    let s_key = SecretKey::generate();
    let s_ep = endpoint(&relay, s_key.clone(), vec![]).await?;
    let s_mail = MailboxClient::connect(&s_ep, dial_addr(mailbox_id, None)).await?;
    s_mail.put_sealed(&s_key, gm_ep.id(), b"\x01junk").await?;

    // Meanwhile the GM edits the character too.
    host.gm_edit(&c, gain(100.0, "Bonus"))?;

    // 4. The GM's app comes online, applies the mail and mails back.
    let r = host.sync_mail(&gm_mail).await?;
    assert_eq!(r.dropped, 1, "{r:?}");
    assert_eq!(r.handled, 1, "{r:?}");
    assert!(r.sent >= 1);
    {
        let a = host.authority();
        assert_eq!(a.version(&c), Some(3));
        assert_eq!(a.character(&c).unwrap().karma, 115);
        assert_eq!(a.character(&c).unwrap().field("alias"), "Raven");
    }
    // A replayed mailbox delivery would run nothing twice: the same batch
    // submitted again is answered from the dedup set.
    {
        let batch = player.replica().batch(&c).expect("still unconfirmed");
        let mut a = host.authority();
        let s = a.submit(&engine, p_ep.id(), batch).unwrap();
        assert_eq!(s.ack.accepted.len(), 2);
        assert_eq!(a.version(&c), Some(3));
    }

    // 5. The player collects the answers.
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    {
        let r = player.replica();
        assert_eq!(r.outbox(&c).len(), 0);
        assert_eq!(r.version(&c), Some(3));
        assert_eq!(r.confirmed_hash(&c), host.authority().hash(&c));
        assert_eq!(r.character(&c).unwrap().karma, 115);
        assert!(r.feed().iter().any(|f| f.to_string() == "GM: Gained 100 karma: Bonus"), "{:?}", r.feed());
    }
    // Nothing left to mail either way.
    let r = host.sync_mail(&gm_mail).await?;
    assert_eq!((r.fetched, r.sent), (0, 0), "{r:?}");

    // The GM's state survives a restart of the GM's app.
    host.save()?;
    let back = Authority::load(&gm_file)?;
    assert_eq!(back.hash(&c), host.authority().hash(&c));

    gm_mail.close();
    s_mail.close();
    for ep in [gm_ep, p_ep, s_ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

/// The glue end to end: a campaign file hosted by the GM's node, a player
/// joining with the invite, edits both ways, the host going away (the
/// player's edits go to the mailbox), the GM's app restarting from its
/// files and taking the mail in, a revert, and the file written back.
#[tokio::test(flavor = "multi_thread")]
async fn hosted_campaign_file_with_offline_player_edits() -> Result<()> {
    use chummer_core::campaign::{Campaign, Member, MemberKind};
    use chummer_sync::{hosted, HostedCampaign, Node};

    let relay = relay("hosted", |_| {}).await?;
    let engine = engine();
    let dir = tmp("hosted-gm");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("Seattle.chummercampaign");
    let mut campaign = Campaign::new("Seattle");
    let pc = campaign.add(Member::embedded(MemberKind::Player, &munin(10)));
    let npc = campaign.add(Member::embedded(MemberKind::Npc, &munin(0)));
    campaign.save(&file)?;
    assert!(!hosted::is_online(&file));

    // The GM hosts it.
    let gm_key = SecretKey::generate();
    let gm = Node::start(gm_key.clone(), relay.client_config()).await?;
    let (h, rec) = HostedCampaign::open(&campaign, &file, engine.clone(), gm_key.clone(), "GM", |_| None)?;
    assert_eq!(rec.added, [pc, npc]);
    assert!(hosted::is_online(&file));
    gm.serve(&h.host);
    assert!(gm.serving());
    gm.sync_mail(&h.host).await?;
    let link: InviteLink = h.invite(Role::Player, "group", Some(&gm)).to_string().parse()?;

    // The player joins and is given the player character.
    let p_key = SecretKey::generate();
    let p_node = Node::start(p_key.clone(), relay.client_config()).await?;
    let p_file = tmp("hosted-p").join("campaign.replica");
    let mut cfg = PlayerConfig::new("Alice", link);
    cfg.mailbox = p_node.mailbox_id();
    cfg.path = Some(p_file);
    cfg.connect_timeout = Duration::from_secs(5);
    let player = PlayerSession::new(p_node.endpoint().clone(), p_key.clone(), engine.clone(), cfg)?;
    assert_eq!(player.sync().await, SyncMode::Online);
    assert_eq!(player.replica().characters().count(), 0);
    assert_eq!(player.replica().membership().unwrap().campaign_name, "Seattle");
    campaign.member_mut(pc).unwrap().owner = Some(p_node.id().to_string());
    h.reconcile(&campaign, |_| None);
    let c = hosted::character_id(pc);
    wait_for(&player, |e| *e == Event::Updated(c.clone())).await?;
    assert_eq!(player.replica().characters().collect::<Vec<_>>(), [&c], "the NPC stays the GM's");

    // Edits both ways.
    player.edit_now(&c, Command::SetField { key: "alias".into(), value: "Raven".into() })?;
    tokio::time::timeout(WAIT, async {
        while h.host.authority().character(&c).unwrap().field("alias") != "Raven" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    h.host.gm_edit(&c, gain(100.0, "Good run"))?;
    until(|| player.replica().character(&c).unwrap().karma == 110).await?;
    let line = player.replica().feed().back().unwrap().clone();
    assert_eq!(chummer_sync::feed::for_owner(&line), "GM gave you 100 karma: Good run");

    // The GM stops hosting: the player is hung up on and falls back to
    // the mailbox for an offline edit.
    gm.stop_serving();
    tokio::time::timeout(WAIT, async {
        while player.is_online() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    player.edit_now(&c, gain(5.0, "Side job"))?;
    assert_eq!(player.replica().outbox(&c).len(), 1);
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    assert!(player.replica().outbox(&c).iter().all(|p| p.mailed));

    // The GM's app restarts from its files and comes back online.
    h.host.save()?;
    drop(h);
    let campaign = Campaign::load(&file)?;
    let (h, rec) = HostedCampaign::open(&campaign, &file, engine.clone(), gm_key.clone(), "GM", |_| None)?;
    assert!(rec.added.is_empty(), "{rec:?}");
    gm.serve(&h.host);
    let r = gm.sync_mail(&h.host).await?;
    assert_eq!(r.handled, 1, "{r:?}");
    let feed: Vec<String> = h.host.authority().feed().iter().map(|f| f.to_string()).collect();
    assert_eq!(h.host.authority().character(&c).unwrap().karma, 115, "{feed:?} {:?}", player.replica().outbox(&c));
    assert_eq!(player.sync().await, SyncMode::Online);
    assert!(player.replica().outbox(&c).is_empty());
    assert_eq!(player.replica().confirmed_hash(&c), h.host.authority().hash(&c));

    // The GM reverts the side job; the player gets it live.
    let v = h.host.authority().feed().iter().rev().find(|f| f.text.contains("Side job")).and_then(|f| f.version).unwrap();
    player.events();
    h.host.gm_revert(&c, v).map_err(anyhow::Error::msg)?;
    until(|| player.replica().character(&c).unwrap().karma == 110).await?;
    assert_eq!(player.replica().confirmed_hash(&c), h.host.authority().hash(&c));

    // Saving writes the authority's characters into the campaign file.
    let mut campaign = campaign;
    h.write_back(&mut campaign).map_err(anyhow::Error::msg)?;
    campaign.save(&file)?;
    let back = Campaign::load(&file)?;
    let ch = back.member(pc).unwrap().load_character(None)?;
    assert_eq!((ch.karma, ch.field("alias")), (110, "Raven".to_string()));

    player.close();
    p_node.shutdown().await;
    gm.shutdown().await;
    relay.shutdown().await
}
