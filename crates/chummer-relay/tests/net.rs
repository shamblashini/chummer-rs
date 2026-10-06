//! End-to-end tests on loopback: an in-process relay with a self-signed
//! certificate, its mailbox, and client endpoints. No internet needed.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chummer_net::campaign::{
    CampaignClient, CampaignHandler, CampaignHost, Hello, Welcome, CAMPAIGN_ALPN, PROTOCOL_VERSION,
};
use chummer_net::invite::{CampaignId, InviteLink, InviteStore, InviteToken, Role};
use chummer_net::mailbox::{MailboxClient, MailboxError};
use chummer_net::node::{bind, dial_addr};
use chummer_net::seal::SealError;
use chummer_net::{Endpoint, EndpointId, NetError, SecretKey};
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

/// A GM host for tests: admits invite holders and known members, acks
/// submissions with a prefix, and records what it got.
#[derive(Debug)]
struct TestGm {
    campaign: CampaignId,
    invites: InviteStore,
    members: Mutex<Vec<(EndpointId, Role)>>,
    received: Mutex<Vec<(EndpointId, Role, Vec<u8>)>>,
}

impl CampaignHandler for TestGm {
    async fn hello(&self, peer: EndpointId, hello: &Hello) -> Result<Welcome, String> {
        if hello.campaign_id != self.campaign {
            return Err("no such campaign".into());
        }
        let mut members = self.members.lock().unwrap();
        let role = match members.iter().find(|(id, _)| *id == peer) {
            Some((_, role)) => *role,
            None => {
                let role = hello
                    .invite_token
                    .and_then(|t| self.invites.redeem(&t))
                    .ok_or("not invited")?;
                members.push((peer, role));
                role
            }
        };
        Ok(Welcome {
            campaign_id: self.campaign,
            role,
            server_version: PROTOCOL_VERSION,
        })
    }

    async fn submit(
        &self,
        peer: EndpointId,
        role: Role,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        if payload == b"fail" {
            return Err("cannot apply".into());
        }
        self.received
            .lock()
            .unwrap()
            .push((peer, role, payload.clone()));
        Ok([b"ack:".as_slice(), &payload].concat())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn campaign_hello_submit_push_round_trip() -> Result<()> {
    let relay = relay("campaign", Arc::new(ManualClock::new(T0)), |_| {}).await?;
    let campaign = CampaignId::random();
    let mut invites = InviteStore::default();
    let token = invites.create(Role::Player, "group", T0);
    let host = CampaignHost::new(TestGm {
        campaign,
        invites,
        members: Mutex::default(),
        received: Mutex::default(),
    });

    let (_gm_key, gm_ep) = endpoint(&relay, vec![]).await?;
    let router = Router::builder(gm_ep.clone())
        .accept(CAMPAIGN_ALPN, host.clone())
        .spawn();

    // The player gets an invite link and dials the GM by node id alone:
    // the address comes from the relay list (RelayLookup).
    let link = InviteLink {
        host: gm_ep.id(),
        campaign,
        invite: Some(token),
        relay: None,
    };
    let link: InviteLink = link.to_string().parse()?;
    let (_p_key, player_ep) = endpoint(&relay, vec![]).await?;
    let hello = Hello {
        campaign_id: link.campaign,
        invite_token: link.invite,
        client_version: PROTOCOL_VERSION,
    };
    let (client, mut pushes) = CampaignClient::join(
        &player_ep,
        dial_addr(link.host, link.relay.as_ref()),
        hello.clone(),
    )
    .await?;
    assert_eq!(client.welcome().role, Role::Player);
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
        assert_eq!(
            got.as_slice(),
            [(player_ep.id(), Role::Player, b"raise Pistols".to_vec())]
        );
    }

    // Server push.
    assert_eq!(host.connected(), [(player_ep.id(), Role::Player)]);
    host.push(player_ep.id(), b"GM gave you 5 karma".to_vec())
        .await?;
    host.push(player_ep.id(), b"second".to_vec()).await?;
    let mut got = vec![
        tokio::time::timeout(WAIT, pushes.recv()).await?.unwrap(),
        tokio::time::timeout(WAIT, pushes.recv()).await?.unwrap(),
    ];
    got.sort();
    assert_eq!(got, [b"GM gave you 5 karma".to_vec(), b"second".to_vec()]);

    // Pushing to someone who is not connected fails (the caller then uses the mailbox).
    assert!(host
        .push(SecretKey::generate().public(), vec![1])
        .await
        .is_err());

    // A stranger with a wrong token is denied.
    let (_s_key, stranger_ep) = endpoint(&relay, vec![]).await?;
    let bad = Hello {
        invite_token: Some(InviteToken::random()),
        ..hello.clone()
    };
    match CampaignClient::join(&stranger_ep, dial_addr(gm_ep.id(), None), bad).await {
        Err(NetError::Denied(reason)) => assert_eq!(reason, "not invited"),
        other => panic!("expected denial, got {other:?}"),
    }

    // A member rejoins without the token.
    client.close();
    let rejoin = Hello {
        invite_token: None,
        ..hello
    };
    let (again, _pushes) =
        CampaignClient::join(&player_ep, dial_addr(gm_ep.id(), None), rejoin).await?;
    assert_eq!(again.welcome().role, Role::Player);
    again.close();

    router.shutdown().await?;
    player_ep.close().await;
    stranger_ep.close().await;
    relay.shutdown().await
}

#[tokio::test(flavor = "multi_thread")]
async fn mailbox_put_fetch_only_by_recipient() -> Result<()> {
    let clock = Arc::new(ManualClock::new(T0));
    let relay = relay("mailbox", clock.clone(), |c| c.limits.max_blob_bytes = 4096).await?;
    let mailbox = relay.mailbox_id().expect("mailbox enabled");
    assert_eq!(
        relay.client_config().mailbox().unwrap().mailbox,
        Some(mailbox)
    );

    let (alice_key, alice_ep) = endpoint(&relay, vec![]).await?;
    let (bob_key, bob_ep) = endpoint(&relay, vec![]).await?;
    let (eve_key, eve_ep) = endpoint(&relay, vec![]).await?;
    let alice = MailboxClient::connect(&alice_ep, dial_addr(mailbox, None)).await?;
    let bob = MailboxClient::connect(&bob_ep, dial_addr(mailbox, None)).await?;
    let eve = MailboxClient::connect(&eve_ep, dial_addr(mailbox, None)).await?;

    // Alice leaves a sealed message for Bob, who is "offline".
    let id = alice
        .put_sealed(&alice_key, bob_ep.id(), b"outbox: raise Pistols")
        .await?;

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
    let opened = opened.as_ref().expect("opens");
    assert_eq!(opened.sender, alice_ep.id());
    assert_eq!(opened.payload, b"outbox: raise Pistols");
    // Even with the raw blob Eve could not open it.
    assert_eq!(
        chummer_net::seal::open(&eve_key, &item.blob),
        Err(SealError::Decrypt)
    );
    assert_eq!(bob.ack(vec![id]).await?, 1);
    assert!(bob.fetch(100).await?.0.is_empty());

    // Limits come back as typed errors.
    match alice.put(bob_ep.id(), vec![0; 4097]).await {
        Err(NetError::Mailbox(MailboxError::TooLarge { max: 4096, .. })) => {}
        other => panic!("expected TooLarge, got {other:?}"),
    }

    // Expiry, driven by the fake clock.
    alice
        .put_sealed(&alice_key, bob_ep.id(), b"old news")
        .await?;
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
    second.shutdown().await?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
