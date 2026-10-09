//! End-to-end tests on loopback: an in-process relay with a self-signed
//! certificate, its mailbox, and client endpoints. No internet needed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chummer_net::campaign::{CampaignClient, CampaignHandler, CampaignHost, DenyReason, Hello, Welcome, CAMPAIGN_ALPN, PROTOCOL_VERSION};
use chummer_net::invite::{CampaignId, InviteLink, MemberSecret, Role};
use chummer_net::mailbox::{MailboxClient, MailboxError, PutAuth, Registration};
use chummer_net::node::{bind, dial_addr};
use chummer_net::seal::SealError;
use chummer_net::{Endpoint, EndpointId, NetError, PublicKey, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use iroh::protocol::Router;

const T0: u64 = 1_800_000_000;
const WAIT: Duration = Duration::from_secs(20);

async fn relay(
    name: &str,
    clock: Arc<ManualClock>,
    tweak: impl FnOnce(&mut Config),
) -> Result<RelayNode> {
    let dir = std::env::temp_dir().join(format!("chummer-relay-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config {
        hostname: "127.0.0.1".into(),
        data_dir: dir,
        http_bind: "127.0.0.1:0".parse()?,
        https_bind: "127.0.0.1:0".parse()?,
        qad_bind: "127.0.0.1:0".parse()?,
        mailbox_port: 0,
        ..Config::default()
    };
    cfg.tls.cert_mode = CertMode::SelfSigned;
    tweak(&mut cfg);
    let node = RelayNode::spawn(cfg, clock).await?;
    node.wait_online(WAIT).await?;
    Ok(node)
}

async fn endpoint(relay: &RelayNode, alpns: Vec<Vec<u8>>) -> Result<(SecretKey, Endpoint)> {
    let key = SecretKey::generate();
    let ep = bind(key.clone(), &relay.client_config(), alpns).await?;
    tokio::time::timeout(WAIT, ep.online()).await?;
    Ok((key, ep))
}

/// A GM host for tests: admits holders of the invite's member key (the
/// first node to use it claims it) and known members, acks submissions
/// with a prefix, and records what it got.
#[derive(Debug)]
struct TestGm {
    campaign: CampaignId,
    invite: PublicKey,
    claimed: Mutex<Option<EndpointId>>,
    received: Mutex<Vec<(EndpointId, Role, Vec<u8>)>>,
}

impl CampaignHandler for TestGm {
    async fn hello(&self, peer: EndpointId, hello: &Hello, member: Option<PublicKey>) -> Result<Welcome, DenyReason> {
        if hello.campaign_id != self.campaign {
            return Err(DenyReason::NoSuchCampaign);
        }
        if member != Some(self.invite) {
            return Err(DenyReason::NotInvited);
        }
        let mut claimed = self.claimed.lock().unwrap();
        match *claimed {
            Some(p) if p != peer => return Err(DenyReason::Claimed),
            _ => *claimed = Some(peer),
        }
        Ok(Welcome { campaign_id: self.campaign, role: Role::Player, label: "Anna".into(), server_version: PROTOCOL_VERSION })
    }

    async fn submit(&self, peer: EndpointId, role: Role, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        if payload == b"fail" {
            return Err("cannot apply".into());
        }
        self.received.lock().unwrap().push((peer, role, payload.clone()));
        Ok([b"ack:".as_slice(), &payload].concat())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn campaign_hello_submit_push_round_trip() -> Result<()> {
    let relay = relay("campaign", Arc::new(ManualClock::new(T0)), |_| {}).await?;
    let campaign = CampaignId::random();
    let member = MemberSecret::random();
    let (_gm_key, gm_ep) = endpoint(&relay, vec![]).await?;
    let host = CampaignHost::new(TestGm { campaign, invite: member.public(), claimed: Mutex::default(), received: Mutex::default() }, gm_ep.id());
    let router = Router::builder(gm_ep.clone()).accept(CAMPAIGN_ALPN, host.clone()).spawn();

    // The player gets an invite link and dials the GM by node id alone:
    // the address comes from the relay list (RelayLookup).
    let link = InviteLink { host: gm_ep.id(), campaign, member: Some(member), gm_key: None, relay: None };
    let link: InviteLink = link.to_string().parse()?;
    let (_p_key, player_ep) = endpoint(&relay, vec![]).await?;
    let key = link.member.as_ref().unwrap().key();
    let (client, mut pushes) = CampaignClient::join(&player_ep, dial_addr(link.host, link.relay.as_ref()), link.campaign, Some(&key)).await?;
    assert_eq!(client.welcome().role, Role::Player);
    assert_eq!(client.welcome().label, "Anna");
    assert_eq!(client.host_id(), gm_ep.id());

    let ack = client.submit(b"raise Pistols".to_vec()).await?;
    assert_eq!(ack, b"ack:raise Pistols");
    match client.submit(b"fail".to_vec()).await {
        Err(NetError::Remote(e)) => assert_eq!(e, "cannot apply"),
        other => panic!("expected a remote error, got {other:?}"),
    }
    let rtt = client.ping().await?;
    assert!(rtt < WAIT);
    {
        let got = host.handler().received.lock().unwrap();
        assert_eq!(got.as_slice(), [(player_ep.id(), Role::Player, b"raise Pistols".to_vec())]);
    }

    // Server push.
    assert_eq!(host.connected(), [(player_ep.id(), Role::Player)]);
    host.push(player_ep.id(), b"GM gave you 5 karma".to_vec()).await?;
    host.push(player_ep.id(), b"second".to_vec()).await?;
    let mut got = vec![tokio::time::timeout(WAIT, pushes.recv()).await?.unwrap(), tokio::time::timeout(WAIT, pushes.recv()).await?.unwrap()];
    got.sort();
    assert_eq!(got, [b"GM gave you 5 karma".to_vec(), b"second".to_vec()]);

    // Pushing to someone who is not connected fails (the caller then uses the mailbox).
    assert!(host.push(SecretKey::generate().public(), vec![1]).await.is_err());

    // A stranger with another key is denied.
    let (_s_key, stranger_ep) = endpoint(&relay, vec![]).await?;
    match CampaignClient::join(&stranger_ep, dial_addr(gm_ep.id(), None), campaign, Some(&SecretKey::generate())).await {
        Err(NetError::Denied(DenyReason::NotInvited)) => {}
        other => panic!("expected denial, got {other:?}"),
    }
    // The leaked link on another device: the invite is claimed.
    match CampaignClient::join(&stranger_ep, dial_addr(gm_ep.id(), None), campaign, Some(&key)).await {
        Err(NetError::Denied(DenyReason::Claimed)) => {}
        other => panic!("expected the claimed refusal, got {other:?}"),
    }

    // The member rejoins (a new challenge each time).
    client.close();
    let (again, _pushes) = CampaignClient::join(&player_ep, dial_addr(gm_ep.id(), None), campaign, Some(&key)).await?;
    assert_eq!(again.welcome().role, Role::Player);
    again.close();

    router.shutdown().await?;
    player_ep.close().await;
    stranger_ep.close().await;
    relay.shutdown().await
}

/// A hello claiming a member key without proving it (a signature by
/// another key, or none) is refused before the handler sees it.
#[tokio::test(flavor = "multi_thread")]
async fn hello_without_a_valid_proof_is_refused() -> Result<()> {
    use chummer_net::campaign::{Challenge, HelloProof, HelloReply};
    use chummer_net::frame::{read_frame, write_frame, MAX_FRAME};
    let relay = relay("proof", Arc::new(ManualClock::new(T0)), |_| {}).await?;
    let campaign = CampaignId::random();
    let member = MemberSecret::random();
    let (_gm_key, gm_ep) = endpoint(&relay, vec![]).await?;
    let host = CampaignHost::new(TestGm { campaign, invite: member.public(), claimed: Mutex::default(), received: Mutex::default() }, gm_ep.id());
    let router = Router::builder(gm_ep.clone()).accept(CAMPAIGN_ALPN, host.clone()).spawn();
    let (_k, ep) = endpoint(&relay, vec![]).await?;
    for forged in [None, Some(SecretKey::generate())] {
        let conn = ep.connect(dial_addr(gm_ep.id(), None), CAMPAIGN_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        write_frame(&mut send, &Hello { campaign_id: campaign, member_key: Some(member.public()), client_version: PROTOCOL_VERSION }, MAX_FRAME).await?;
        let Challenge { nonce } = read_frame(&mut recv, MAX_FRAME).await?;
        let sig = forged.map(|k| k.sign(&chummer_net::campaign::hello_message(&campaign, &gm_ep.id(), &ep.id(), &nonce)));
        write_frame(&mut send, &HelloProof { sig }, MAX_FRAME).await?;
        send.finish()?;
        let reply: HelloReply = read_frame(&mut recv, MAX_FRAME).await?;
        assert_eq!(reply, HelloReply::Denied(DenyReason::BadProof));
        conn.close(0u32.into(), b"");
    }
    assert!(host.handler().claimed.lock().unwrap().is_none(), "nothing was claimed");
    router.shutdown().await?;
    ep.close().await;
    relay.shutdown().await
}

#[tokio::test(flavor = "multi_thread")]
async fn mailbox_put_fetch_only_by_recipient() -> Result<()> {
    let clock = Arc::new(ManualClock::new(T0));
    let relay = relay("mailbox", clock.clone(), |c| c.limits.max_blob_bytes = 4096).await?;
    let mailbox = relay.mailbox_id().expect("mailbox enabled");
    assert_eq!(relay.client_config().mailbox().unwrap().mailbox, Some(mailbox));

    let (alice_key, alice_ep) = endpoint(&relay, vec![]).await?;
    let (bob_key, bob_ep) = endpoint(&relay, vec![]).await?;
    let (eve_key, eve_ep) = endpoint(&relay, vec![]).await?;
    let alice = MailboxClient::connect(&alice_ep, dial_addr(mailbox, None)).await?;
    let bob = MailboxClient::connect(&bob_ep, dial_addr(mailbox, None)).await?;
    let eve = MailboxClient::connect(&eve_ep, dial_addr(mailbox, None)).await?;

    // Alice's capability for Bob's mailbox: a key Bob registers.
    let cap = SecretKey::generate();
    let st = bob.register([1; 16], vec![cap.public()]).await?;
    assert_eq!(st.keys, 1);

    // Alice leaves a sealed message for Bob, who is "offline".
    let id = alice.put_sealed(&alice_key, &cap, bob_ep.id(), b"outbox: raise Pistols").await?;

    // Eve cannot read or delete Bob's mail: fetch/ack act on her own id.
    let (eve_mail, _) = eve.fetch(100).await?;
    assert!(eve_mail.is_empty());
    assert_eq!(eve.ack(vec![id]).await?, 0);

    // Bob collects, opens and verifies it.
    let (mail, more) = bob.fetch_opened(&bob_key, 100).await?;
    assert!(!more);
    assert_eq!(mail.len(), 1);
    let (item, opened) = &mail[0];
    assert_eq!(item.id, id);
    assert_eq!(item.sender, alice_ep.id());
    assert_eq!(item.key, cap.public());
    let opened = opened.as_ref().expect("opens");
    assert_eq!(opened.sender, alice_ep.id());
    assert_eq!(opened.payload, b"outbox: raise Pistols");
    // Even with the raw blob Eve could not open it.
    assert_eq!(chummer_net::seal::open(&eve_key, &item.blob), Err(SealError::Decrypt));
    assert_eq!(bob.ack(vec![id]).await?, 1);
    assert!(bob.fetch(100).await?.0.is_empty());

    // Limits come back as typed errors.
    match alice.put(bob_ep.id(), vec![0; 4097], &cap).await {
        Err(NetError::Mailbox(MailboxError::TooLarge { max: 4096, .. })) => {}
        other => panic!("expected TooLarge, got {other:?}"),
    }

    // Expiry, driven by the fake clock.
    alice.put_sealed(&alice_key, &cap, bob_ep.id(), b"old news").await?;
    clock.advance(30 * 24 * 60 * 60);
    assert!(bob.fetch(100).await?.0.is_empty());

    for c in [&alice, &bob, &eve] {
        c.close();
    }
    for ep in [alice_ep, bob_ep, eve_ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

/// Who may put mail: a stranger, an unsigned put, a key the recipient did
/// not register, a signature by another key and a put replayed into
/// another mailbox are all refused; a mailbox with no registrations takes
/// nothing; the per-key cap stops a leaked key; a key bound to a node is
/// refused from another; the refusals are counted for the owner.
#[tokio::test(flavor = "multi_thread")]
async fn mailbox_refuses_puts_without_a_registered_key() -> Result<()> {
    let relay = relay("access", Arc::new(ManualClock::new(T0)), |c| c.limits.max_messages_per_key = 5).await?;
    let mailbox = relay.mailbox_id().unwrap();
    let (gm_key, gm_ep) = endpoint(&relay, vec![]).await?;
    let (p_key, p_ep) = endpoint(&relay, vec![]).await?;
    let (_c_key, carol_ep) = endpoint(&relay, vec![]).await?;
    let gm = MailboxClient::connect(&gm_ep, dial_addr(mailbox, None)).await?;
    let p = MailboxClient::connect(&p_ep, dial_addr(mailbox, None)).await?;
    let carol = MailboxClient::connect(&carol_ep, dial_addr(mailbox, None)).await?;
    let (anna, bert) = (MemberSecret::random().key(), MemberSecret::random().key());
    let stranger = SecretKey::generate();
    let refused = |r: Result<u64, NetError>| match r {
        Err(NetError::Mailbox(e)) => e,
        other => panic!("expected a refusal, got {other:?}"),
    };

    // Nobody registered anything yet: no open mode.
    assert_eq!(refused(p.put(gm_ep.id(), vec![1], &anna).await), MailboxError::NotAllowed);
    gm.register([9; 16], vec![anna.public(), bert.public()]).await?;
    carol.register([9; 16], vec![anna.public()]).await?;

    // A stranger's own key, and no signature at all.
    assert_eq!(refused(p.put(gm_ep.id(), vec![1], &stranger).await), MailboxError::NotAllowed);
    assert_eq!(refused(p.put_with(gm_ep.id(), vec![1], None).await), MailboxError::Unsigned);
    // Claiming Anna's key with a signature by another key.
    let mut forged = PutAuth::sign(&stranger, &gm_ep.id(), &p_ep.id(), &[1]);
    forged.key = anna.public();
    assert_eq!(refused(p.put_with(gm_ep.id(), vec![1], Some(forged)).await), MailboxError::BadSignature);
    // A put signed for the GM's mailbox, replayed into Carol's (she takes
    // Anna's key too) or by another uploader.
    let auth = PutAuth::sign(&anna, &gm_ep.id(), &p_ep.id(), &[7]);
    assert_eq!(refused(p.put_with(carol_ep.id(), vec![7], Some(auth.clone())).await), MailboxError::BadSignature);
    assert_eq!(refused(carol.put_with(gm_ep.id(), vec![7], Some(auth.clone())).await), MailboxError::BadSignature);
    // The real one goes in once; the same put again is a replay.
    p.put_with(gm_ep.id(), vec![7], Some(auth.clone())).await?;
    assert_eq!(refused(p.put_with(gm_ep.id(), vec![7], Some(auth)).await), MailboxError::Replayed);

    // Anna's key leaks: the cap stops it at 5 waiting, Bert is unaffected.
    for i in 0..4u8 {
        p.put(gm_ep.id(), vec![i], &anna).await?;
    }
    assert_eq!(refused(p.put(gm_ep.id(), vec![9], &anna).await), MailboxError::KeyFull { max: 5 });
    p.put(gm_ep.id(), vec![1], &bert).await?;

    // The GM revokes Anna: replace the set without her key.
    let st = gm.register([9; 16], vec![bert.public()]).await?;
    assert_eq!(st.keys, 1);
    assert_eq!(st.waiting, 6);
    assert!(st.by_key.contains(&(anna.public(), 5)) && st.by_key.contains(&(bert.public(), 1)), "{st:?}");
    // Stranger, unsigned, forged, replay by Carol, replay, cap (the first
    // refusal came before the GM registered, and Carol counts her own).
    assert_eq!(st.refused_today, 6, "{st:?}");
    assert_eq!(refused(p.put(gm_ep.id(), vec![1], &anna).await), MailboxError::NotAllowed);
    p.put(gm_ep.id(), vec![2], &bert).await?;
    // The GM still collects what was waiting.
    let (items, _) = gm.fetch(100).await?;
    assert_eq!(items.len(), 7);
    gm.ack(items.iter().map(|i| i.id).collect()).await?;
    // Bert claimed his invite on P's device: the GM binds his key to it.
    // A copy of the key on Carol's device is refused outright.
    let st = gm.register([9; 16], vec![Registration::bound(bert.public(), p_ep.id())]).await?;
    assert_eq!(st.keys, 1);
    assert_eq!(refused(carol.put(gm_ep.id(), vec![3], &bert).await), MailboxError::WrongDevice);
    p.put(gm_ep.id(), vec![3], &bert).await?;
    let _ = gm_key;
    let _ = p_key;

    for c in [&gm, &p, &carol] {
        c.close();
    }
    for ep in [gm_ep, p_ep, carol_ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

#[tokio::test(flavor = "multi_thread")]
async fn mailbox_node_key_survives_restart() -> Result<()> {
    let clock = Arc::new(ManualClock::new(T0));
    let first = relay("restart", clock.clone(), |_| {}).await?;
    let id = first.mailbox_id();
    let dir = std::env::temp_dir().join(format!("chummer-relay-it-{}-restart", std::process::id()));
    first.shutdown().await?;
    let mut cfg = Config {
        hostname: "127.0.0.1".into(),
        data_dir: dir.clone(),
        http_bind: "127.0.0.1:0".parse()?,
        https_bind: "127.0.0.1:0".parse()?,
        qad_bind: "127.0.0.1:0".parse()?,
        mailbox_port: 0,
        ..Config::default()
    };
    cfg.tls.cert_mode = CertMode::SelfSigned;
    let second = RelayNode::spawn(cfg, clock).await?;
    assert_eq!(second.mailbox_id(), id);
    // Registrations made before a restart still hold after it.
    second.wait_online(WAIT).await?;
    let (k, ep) = endpoint(&second, vec![]).await?;
    let mb = MailboxClient::connect(&ep, dial_addr(id.unwrap(), None)).await?;
    let cap = SecretKey::generate();
    mb.register([3; 16], vec![cap.public()]).await?;
    mb.put_sealed(&k, &cap, ep.id(), b"one").await?;
    mb.close();
    ep.close().await;
    second.shutdown().await?;
    let mut cfg2 = Config { hostname: "127.0.0.1".into(), data_dir: dir.clone(), http_bind: "127.0.0.1:0".parse()?, https_bind: "127.0.0.1:0".parse()?, qad_bind: "127.0.0.1:0".parse()?, mailbox_port: 0, ..Config::default() };
    cfg2.tls.cert_mode = CertMode::SelfSigned;
    let second = RelayNode::spawn(cfg2, Arc::new(ManualClock::new(T0))).await?;
    second.wait_online(WAIT).await?;
    let (_k2, ep2) = endpoint(&second, vec![]).await?;
    let mb2 = MailboxClient::connect(&ep2, dial_addr(id.unwrap(), None)).await?;
    mb2.put_sealed(&_k2, &cap, ep.id(), b"two").await?;
    assert!(matches!(mb2.put_sealed(&_k2, &SecretKey::generate(), ep.id(), b"spam").await, Err(NetError::Mailbox(MailboxError::NotAllowed))));
    mb2.close();
    ep2.close().await;
    second.shutdown().await?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// Behind a TLS-terminating proxy (`cert_mode = "proxy"`): the relay
/// serves plain HTTP, has no QUIC address discovery, needs no hostname, and
/// its mailbox node connects through the local port. Mail goes through.
#[tokio::test(flavor = "multi_thread")]
async fn proxy_mode_serves_plain_http_and_the_mailbox() -> Result<()> {
    let clock = Arc::new(ManualClock::new(T0));
    let relay = relay("proxy", clock.clone(), |c| {
        c.hostname = String::new();
        c.tls.cert_mode = CertMode::Proxy;
        c.http_bind = "[::]:0".parse().unwrap();
    })
    .await?;
    assert!(relay.relay_url().is_none());
    assert_eq!(relay.local_url().scheme(), "http");
    assert!(relay.self_signed_cert().is_none());
    let entry = relay.relay_entry();
    assert_eq!(entry.url, *relay.local_url());
    assert_eq!(entry.qad_port, Some(0));
    let mailbox = relay.mailbox_id().expect("mailbox enabled");

    let (alice_key, alice_ep) = endpoint(&relay, vec![]).await?;
    // Bob uses another URL for the same relay, as players use the proxy's
    // public one while the mailbox node uses the local port: the relay
    // forwards by node id, so the URLs need not match.
    let mut other = relay.relay_entry();
    other.url = format!("http://localhost:{}", relay.local_url().port().unwrap()).parse()?;
    assert_ne!(other.url, *relay.local_url());
    let bob_key = SecretKey::generate();
    let bob_ep = bind(bob_key.clone(), &chummer_net::config::NetConfig::with_relays([other]), vec![]).await?;
    tokio::time::timeout(WAIT, bob_ep.online()).await?;
    let alice = MailboxClient::connect(&alice_ep, dial_addr(mailbox, None)).await?;
    let bob = MailboxClient::connect(&bob_ep, dial_addr(mailbox, None)).await?;
    let cap = SecretKey::generate();
    bob.register([2; 16], vec![cap.public()]).await?;
    let id = alice.put_sealed(&alice_key, &cap, bob_ep.id(), b"behind the proxy").await?;
    let (mail, _) = bob.fetch_opened(&bob_key, 100).await?;
    assert_eq!(mail.len(), 1);
    assert_eq!(mail[0].0.id, id);
    assert_eq!(mail[0].1.as_ref().expect("opens").payload, b"behind the proxy");
    for c in [&alice, &bob] {
        c.close();
    }
    for ep in [alice_ep, bob_ep] {
        ep.close().await;
    }
    relay.shutdown().await?;

    // With a hostname, the public URL is https://<hostname> (the proxy's).
    let relay = relay_named_proxy(clock).await?;
    assert_eq!(relay.relay_url().map(|u| u.as_str()), Some("https://relay.example.org/"));
    assert_eq!(relay.relay_entry().url.as_str(), "https://relay.example.org/");
    relay.shutdown().await
}

async fn relay_named_proxy(clock: Arc<ManualClock>) -> Result<RelayNode> {
    relay("proxy-named", clock, |c| {
        c.hostname = "relay.example.org".into();
        c.tls.cert_mode = CertMode::Proxy;
    })
    .await
}
