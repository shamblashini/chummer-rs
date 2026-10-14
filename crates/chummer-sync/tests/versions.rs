//! Apps of other versions, live and by mail: each side is told, once,
//! whose app is older. Through an in-process relay; no internet needed.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_net::campaign::{hello_message, Challenge, DenyReason, Hello, HelloProof, HelloReply, APP_VERSION, CAMPAIGN_ALPN, PROTOCOL_VERSION};
use chummer_net::frame::{read_frame, write_frame, MAX_FRAME};
use chummer_net::invite::{derive_campaign_key, CampaignId, InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, EndpointId, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use chummer_sync::msg::{VersionNotice, SYNC_VERSION};
use chummer_sync::{Authority, AuthorityHost, CharacterId, Event, PlayerConfig, PlayerSession, SyncMode};
use iroh::protocol::Router;
use serde::Serialize;

const T0: u64 = 1_800_000_000;
const WAIT: Duration = Duration::from_secs(30);

fn engine() -> Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data"))).clone()
}

fn munin() -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
    Character::load(&p).unwrap()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-sync-versions-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

async fn relay(name: &str) -> Result<RelayNode> {
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
    let node = RelayNode::spawn(cfg, Arc::new(ManualClock::new(T0))).await?;
    node.wait_online(WAIT).await?;
    Ok(node)
}

async fn endpoint(relay: &RelayNode, key: SecretKey) -> Result<Endpoint> {
    let ep = bind(key, &relay.client_config(), vec![]).await?;
    tokio::time::timeout(WAIT, ep.online()).await?;
    Ok(ep)
}

/// [`Hello`] as 0.6.0 (protocol 2) sends it: no app version.
#[derive(Serialize)]
struct OldHello {
    campaign_id: CampaignId,
    member_key: Option<chummer_net::PublicKey>,
    client_version: u32,
}

/// A handshake by hand, as an app of another version makes it.
async fn handshake<H: Serialize>(ep: &Endpoint, host: EndpointId, campaign: CampaignId, member: &SecretKey, hello: &H) -> Result<HelloReply> {
    let conn = ep.connect(dial_addr(host, None), CAMPAIGN_ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, hello, MAX_FRAME).await?;
    let Challenge { nonce } = read_frame(&mut recv, MAX_FRAME).await?;
    let sig = Some(member.sign(&hello_message(&campaign, &host, &ep.id(), &nonce)));
    write_frame(&mut send, &HelloProof { sig }, MAX_FRAME).await?;
    send.finish()?;
    let reply: HelloReply = read_frame(&mut recv, MAX_FRAME).await?;
    conn.close(0u32.into(), b"");
    Ok(reply)
}

#[tokio::test(flavor = "multi_thread")]
async fn other_protocol_versions_are_refused_with_who_updates() -> Result<()> {
    let relay = relay("live").await?;
    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone()).await?;
    let campaign = CampaignId::random();
    let host = AuthorityHost::new(Authority::new(campaign, gm_ep.id(), "GM"), engine(), gm_key, None);
    let router = Router::builder(gm_ep.clone()).accept(CAMPAIGN_ALPN, host.protocol()).spawn();
    let (_, link) = host.create_invite(Role::Player, "Anna", None, None, None);
    let member = link.member.as_ref().unwrap().key();
    let p_key = SecretKey::generate();
    let p_ep = endpoint(&relay, p_key.clone()).await?;

    // 0.6.0 (protocol 2): the refusal it can read, saying it is older;
    // twice, as it retries every minute, and the GM is told once.
    let old = OldHello { campaign_id: campaign, member_key: Some(member.public()), client_version: 2 };
    for _ in 0..2 {
        let reply = handshake(&p_ep, gm_ep.id(), campaign, &member, &old).await?;
        let HelloReply::Denied(reason) = reply else { panic!("let in: {reply:?}") };
        assert_eq!(reason, DenyReason::Version { client: 2, server: PROTOCOL_VERSION });
        assert_eq!(reason.to_string(), format!("Your chummer-rs is older than the GM's (campaign protocol {PROTOCOL_VERSION}; yours is 2). Update chummer-rs."));
    }
    let notices = host.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].text, format!("Anna tried to join with an older chummer-rs (campaign protocol 2; yours is {PROTOCOL_VERSION}). They need to update chummer-rs, or use the same version as you."));
    assert!(host.authority().invites().values().all(|i| i.claimed.is_none()), "a refused app claims nothing");

    // A newer app: told the GM's is older, with both app versions.
    let newer = Hello { campaign_id: campaign, member_key: Some(member.public()), client_version: PROTOCOL_VERSION + 1, app_version: "9.0.0".into() };
    let reply = handshake(&p_ep, gm_ep.id(), campaign, &member, &newer).await?;
    let HelloReply::Denied(reason) = reply else { panic!("let in: {reply:?}") };
    assert!(reason.is_version() && reason.is_final());
    assert_eq!(reason.to_string(), format!("The GM's chummer-rs is older (version {APP_VERSION}; yours is 9.0.0). Ask your GM to update, or use the same version."));
    let notices = host.notices();
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert_eq!(notices[1].text, format!("Anna tried to join with a newer chummer-rs (version 9.0.0; yours is {APP_VERSION}). Update chummer-rs, or ask them to use the same version as you."));

    // This app still gets in.
    let mut cfg = PlayerConfig::new("Anna", link.to_string().parse::<InviteLink>()?);
    cfg.path = Some(tmp("live-p").join("campaign.replica"));
    let player = PlayerSession::new(p_ep.clone(), p_key, engine(), cfg)?;
    assert_eq!(player.connect().await?, Role::Player);
    assert_eq!(player.denied(), None);
    assert_eq!(host.notices().len(), 2);
    router.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn mail_of_another_version_gives_one_notice_on_each_side() -> Result<()> {
    let relay = relay("mail").await?;
    let mailbox_id = relay.mailbox_id().expect("mailbox");
    let engine = engine();
    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone()).await?;
    let p_key = SecretKey::generate();
    let p_ep = endpoint(&relay, p_key.clone()).await?;
    let campaign = CampaignId::random();
    let mut auth = Authority::new(campaign, gm_ep.id(), "GM");
    auth.add_member(p_ep.id(), Role::Player, "Alice");
    let c = CharacterId::new("alice-munin");
    auth.add_character(c.clone(), Some(p_ep.id()), munin())?;
    let host = AuthorityHost::new(auth, engine.clone(), gm_key.clone(), None);
    let gm_mail = MailboxClient::connect(&gm_ep, dial_addr(mailbox_id, None)).await?;
    host.sync_mail(&gm_mail).await?;

    // Alice joins by mail (the GM does not serve the campaign live).
    let link = InviteLink { host: gm_ep.id(), campaign, member: None, gm_key: Some(host.authority().gm_keys()[0]), relay: None };
    let mut cfg = PlayerConfig::new("Alice", link);
    cfg.mailbox = Some(mailbox_id);
    cfg.connect_timeout = Duration::from_secs(5);
    let player = PlayerSession::new(p_ep.clone(), p_key.clone(), engine.clone(), cfg)?;
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    host.sync_mail(&gm_mail).await?;
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    assert_eq!(player.replica().version(&c), Some(0));
    player.events();

    // The GM gets two mails in a newer sync format: one notice, and the
    // mail is read (acked) and dropped.
    let p_mail = MailboxClient::connect(&p_ep, dial_addr(mailbox_id, None)).await?;
    let newer = [SYNC_VERSION + 1, 1, 2, 3];
    for _ in 0..2 {
        p_mail.put_sealed(&p_key, &p_key, gm_ep.id(), &newer).await?;
    }
    let r = host.sync_mail(&gm_mail).await?;
    assert_eq!((r.fetched, r.dropped, r.handled), (2, 2, 0), "{r:?}");
    let next = SYNC_VERSION as u32 + 1;
    let notices = host.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].text, format!("Alice mailed with a newer chummer-rs (campaign protocol {next}; yours is {SYNC_VERSION}). Update chummer-rs, or ask them to use the same version as you."));
    assert_eq!(notices[0].peer, p_ep.id());
    p_mail.put_sealed(&p_key, &p_key, gm_ep.id(), &newer).await?;
    let r = host.sync_mail(&gm_mail).await?;
    assert_eq!((r.fetched, r.dropped), (1, 1), "{r:?}");
    assert_eq!(host.notices().len(), 1, "once per member and version");

    // The GM's app mailed its version notice back (once). Alice's app has
    // the GM's sync format, so it has nothing to say about it.
    player.fetch_mail().await?;
    assert_eq!(player.denied(), None);
    assert!(player.events().iter().all(|e| !matches!(e, Event::Denied(_))));

    // Mail from a GM app with a newer sync format: one notice for the
    // player, however many mails.
    let signer = derive_campaign_key(&gm_key, campaign, 0);
    for _ in 0..3 {
        gm_mail.put_sealed(&gm_key, &signer, p_ep.id(), &newer).await?;
    }
    assert_eq!(player.fetch_mail().await?, 3);
    let denied: Vec<DenyReason> = player.events().into_iter().filter_map(|e| if let Event::Denied(d) = e { Some(d) } else { None }).collect();
    assert_eq!(denied.len(), 1, "{denied:?}");
    assert_eq!(denied[0].to_string(), format!("Your chummer-rs is older than the GM's (campaign protocol {next}; yours is {SYNC_VERSION}). Update chummer-rs."));
    assert_eq!(player.denied().as_ref(), Some(&denied[0]));

    // A notice from that GM app names its version: told once more, with it;
    // its other mail after that does not take it away.
    let notice = VersionNotice { app_version: "9.0.0".into(), sync_version: next, protocol_version: next };
    gm_mail.put_sealed(&gm_key, &signer, p_ep.id(), &notice.encode()).await?;
    gm_mail.put_sealed(&gm_key, &signer, p_ep.id(), &newer).await?;
    player.fetch_mail().await?;
    let denied: Vec<DenyReason> = player.events().into_iter().filter_map(|e| if let Event::Denied(d) = e { Some(d) } else { None }).collect();
    assert_eq!(denied.len(), 1, "{denied:?}");
    let text = format!("Your chummer-rs is older than the GM's (version 9.0.0; yours is {APP_VERSION}). Update chummer-rs.");
    assert_eq!(denied[0].to_string(), text);
    assert_eq!(player.denied().map(|d| d.to_string()), Some(text));

    // Not kept on disk: an older app (the GM's version) reads the file.
    assert!(chummer_sync::Replica::from_bytes(&engine, &player.replica().to_bytes())?.denied().is_none());

    // The GM's app speaks ours again: its next mail lifts the notice.
    host.gm_edit(&c, Command::SetField { key: "alias".into(), value: "Raven".into() }).map_err(|e| anyhow::anyhow!(e.reason))?;
    host.sync_mail(&gm_mail).await?;
    player.fetch_mail().await?;
    assert_eq!(player.denied(), None);
    assert_eq!(player.replica().version(&c), Some(1));
    Ok(())
}
