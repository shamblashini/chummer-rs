//! Per-player invites: claims, re-issued links, revocation, expiry, joins
//! by mail, and the relay mailbox only taking mail from current keys. The
//! first half drives the authority directly; the second goes through an
//! in-process relay.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_net::campaign::{DenyReason, CAMPAIGN_ALPN};
use chummer_net::invite::{CampaignId, InviteLink, Role};
use chummer_net::mailbox::{MailboxClient, MailboxError, Registration};
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, EndpointId, NetError, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use chummer_sync::invites::InviteState;
use chummer_sync::msg::ClaimProof;
use chummer_sync::{Authority, AuthorityHost, CharacterId, Event, PlayerConfig, PlayerSession, SyncMode};
use iroh::protocol::Router;

const T0: u64 = 1_800_000_000;
const NOW: u64 = 1_700_000_000;
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

fn node() -> EndpointId {
    SecretKey::generate().public()
}

fn authority() -> (Authority, SecretKey) {
    let gm = SecretKey::generate();
    (Authority::new(CampaignId([5; 16]), gm.public(), "GM"), gm)
}

// ----- the authority alone -----

#[test]
fn first_join_claims_the_invite_and_binds_it_to_that_node() {
    let (mut a, _) = authority();
    let anna = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let (dev1, dev2) = (node(), node());
    let c = a.campaign();
    // No key, a wrong key, another campaign.
    assert_eq!(a.admit(dev1, c, None, NOW), Err(DenyReason::NotInvited));
    assert_eq!(a.admit(dev1, c, Some(node()), NOW), Err(DenyReason::NotInvited));
    assert_eq!(a.admit(dev1, CampaignId([6; 16]), Some(anna.key()), NOW), Err(DenyReason::NoSuchCampaign));
    // The first device claims it.
    let ok = a.admit(dev1, c, Some(anna.key()), NOW + 1).unwrap();
    assert_eq!((ok.role, ok.label.as_str(), ok.claimed), (Role::Player, "Anna", true));
    assert_eq!(a.invite(&anna.id).unwrap().state(NOW + 2), InviteState::Claimed { node: dev1, at: NOW + 1 });
    assert_eq!(a.members()[&dev1].invite, Some(anna.id));
    // Again from the same device: fine, not a new claim.
    assert!(!a.admit(dev1, c, Some(anna.key()), NOW + 5).unwrap().claimed);
    assert_eq!(a.members()[&dev1].last_seen, Some(NOW + 5));
    // The member must prove the key every time.
    assert_eq!(a.admit(dev1, c, None, NOW), Err(DenyReason::BadProof));
    // The forwarded link on another device.
    assert_eq!(a.admit(dev2, c, Some(anna.key()), NOW), Err(DenyReason::Claimed));
    assert!(a.role(&dev2).is_none());
    // The membership names the label.
    assert_eq!(a.membership(&dev1).unwrap().label, "Anna");
}

#[test]
fn reissue_supersedes_the_old_link_and_moves_the_characters() {
    let (mut a, _) = authority();
    let anna = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let (dev1, dev2) = (node(), node());
    let c = a.campaign();
    a.admit(dev1, c, Some(anna.key()), NOW).unwrap();
    let ch = CharacterId::new("anna-pc");
    a.add_character(ch.clone(), Some(dev1), munin(5)).unwrap();
    a.take_owner_changes();

    // A new device: the GM issues a new link.
    assert_eq!(a.reissue_invite(&anna.id, None, NOW + 10).unwrap(), Some(dev1), "the old device is hung up on");
    let new_key = a.invite(&anna.id).unwrap().key();
    assert_ne!(new_key, anna.key());
    assert!(a.role(&dev1).is_none() && a.is_retired(&dev1));
    // The old link is refused everywhere, with a reason that says why.
    assert_eq!(a.admit(dev1, c, Some(anna.key()), NOW), Err(DenyReason::Superseded));
    assert_eq!(a.admit(dev2, c, Some(anna.key()), NOW), Err(DenyReason::Superseded));
    assert!(!a.mail_keys(NOW + 11).contains(&anna.key()));
    assert!(a.mail_keys(NOW + 11).contains(&new_key));
    // The new device claims the new link and gets Anna's character.
    assert!(a.admit(dev2, c, Some(new_key), NOW + 20).unwrap().claimed);
    assert_eq!(a.owner(&ch), Some(dev2));
    assert_eq!(a.take_owner_changes().into_iter().collect::<Vec<_>>(), std::slice::from_ref(&ch));
    assert_eq!(a.visible(&dev2), [ch]);
    // The old device, even with the new link, is too late.
    assert_eq!(a.admit(dev1, c, Some(new_key), NOW), Err(DenyReason::Claimed));
}

#[test]
fn revoke_cuts_the_member_and_leaves_the_others() {
    let (mut a, _) = authority();
    let anna = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let bert = a.create_invite(Role::Player, "Bert", None, None, NOW).clone();
    let (da, db) = (node(), node());
    let c = a.campaign();
    a.admit(da, c, Some(anna.key()), NOW).unwrap();
    a.admit(db, c, Some(bert.key()), NOW).unwrap();
    let (ca, cb) = (CharacterId::new("a"), CharacterId::new("b"));
    a.add_character(ca.clone(), Some(da), munin(5)).unwrap();
    a.add_character(cb.clone(), Some(db), munin(5)).unwrap();
    assert_eq!(a.mail_keys(NOW), { let mut k = vec![anna.key(), bert.key()]; k.sort(); k });

    assert_eq!(a.revoke_invite(&anna.id, NOW + 1).unwrap(), Some(da));
    assert_eq!(a.invite(&anna.id).unwrap().state(NOW + 2), InviteState::Revoked { at: NOW + 1 });
    assert_eq!(a.admit(da, c, Some(anna.key()), NOW + 2), Err(DenyReason::Revoked));
    assert_eq!(a.admit(node(), c, Some(anna.key()), NOW + 2), Err(DenyReason::Revoked));
    assert_eq!(a.mail_keys(NOW + 2), [bert.key()]);
    assert!(a.membership(&da).is_none());
    assert!(a.submit(&engine(), da, chummer_sync::msg::SubmitBatch { character: ca.clone(), base_version: 0, base_hash: [0; 32], ops: Vec::new() }).is_err());
    // Bert goes on as before.
    assert!(!a.admit(db, c, Some(bert.key()), NOW + 2).unwrap().claimed);
    assert_eq!(a.visible(&db), [cb]);
    assert!(a.membership(&db).unwrap().members.iter().all(|m| m.id != da), "Anna is gone from the member list");
    // Revoked stays listed; removing deletes it.
    assert!(a.invites().contains_key(&anna.id));
    a.remove_invite(&anna.id);
    assert!(!a.invites().contains_key(&anna.id));
    assert_eq!(a.admit(da, c, Some(anna.key()), NOW + 3), Err(DenyReason::Revoked), "still refused as a retired node");
}

#[test]
fn unclaimed_invites_expire() {
    let (mut a, _) = authority();
    let i = a.create_invite(Role::Player, "Late", None, Some(NOW + 100), NOW).clone();
    assert!(a.mail_keys(NOW + 99).contains(&i.key()), "an unclaimed invite may mail (a first join by mail)");
    assert!(a.mail_keys(NOW + 100).is_empty());
    assert_eq!(a.admit(node(), a.campaign(), Some(i.key()), NOW + 100), Err(DenyReason::Expired));
    // Claimed before it expires: it stays.
    let j = a.create_invite(Role::Player, "Early", None, Some(NOW + 100), NOW).clone();
    let d = node();
    a.admit(d, a.campaign(), Some(j.key()), NOW + 50).unwrap();
    assert!(a.admit(d, a.campaign(), Some(j.key()), NOW + 5000).is_ok());
    assert!(a.mail_keys(NOW + 5000).contains(&j.key()));
}

#[test]
fn mailed_claims_need_a_proof_for_this_node() {
    let (mut a, gm) = authority();
    let i = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let (d, other) = (node(), node());
    let c = a.campaign();
    // A proof made for another node (copied out of someone's mail) does
    // not work, nor one for another campaign.
    let theirs = ClaimProof::new(&i.secret.key(), &c, &gm.public(), &other);
    assert_eq!(a.admit_by_mail(d, Some(&theirs), NOW), Err(DenyReason::BadProof));
    let wrong = ClaimProof::new(&i.secret.key(), &CampaignId([9; 16]), &gm.public(), &d);
    assert_eq!(a.admit_by_mail(d, Some(&wrong), NOW), Err(DenyReason::BadProof));
    assert_eq!(a.admit_by_mail(d, None, NOW), Err(DenyReason::NotInvited));
    let mine = ClaimProof::new(&i.secret.key(), &c, &gm.public(), &d);
    assert!(a.admit_by_mail(d, Some(&mine), NOW).unwrap().claimed);
    // A member's later mail needs no proof (the sealed mail proves the node).
    assert!(a.admit_by_mail(d, None, NOW).is_ok());
}

#[test]
fn members_by_node_id_mail_with_their_node_key() {
    let (mut a, _) = authority();
    let p = node();
    a.add_member(p, Role::Player, "Old friend");
    assert_eq!(a.mail_keys(NOW), [p]);
    assert!(a.admit(p, a.campaign(), None, NOW).is_ok());
    assert!(a.remove_member(&p));
    assert!(a.mail_keys(NOW).is_empty());
    assert_eq!(a.admit(p, a.campaign(), None, NOW), Err(DenyReason::Revoked));
}

/// What the GM registers with the relay: a claimed invite's key only from
/// the claiming device, an unclaimed one from any device (a first join by
/// mail), a member added by node id only from that node.
#[test]
fn claimed_invite_keys_are_bound_to_their_device() {
    let (mut a, _) = authority();
    let anna = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let p = node();
    a.add_member(p, Role::Player, "Old friend");
    let mut want = vec![Registration::any(anna.key()), Registration::bound(p, p)];
    want.sort();
    assert_eq!(a.mail_registrations(NOW), want);
    let dev = node();
    a.admit(dev, a.campaign(), Some(anna.key()), NOW).unwrap();
    let mut want = vec![Registration::bound(anna.key(), dev), Registration::bound(p, p)];
    want.sort();
    assert_eq!(a.mail_registrations(NOW), want);
    // A new link: a new key, unclaimed again (any device until claimed).
    a.reissue_invite(&anna.id, None, NOW + 1).unwrap();
    let key = a.invite(&anna.id).unwrap().key();
    assert!(a.mail_registrations(NOW + 1).contains(&Registration::any(key)));
    assert!(!a.mail_keys(NOW + 1).contains(&anna.key()));
}

#[test]
fn invites_and_retired_nodes_survive_a_save() {
    let (mut a, _) = authority();
    let anna = a.create_invite(Role::Player, "Anna", Some(CharacterId::new("x")), Some(NOW + 9), NOW).clone();
    let bert = a.create_invite(Role::Player, "Bert", None, None, NOW).clone();
    let (da, db) = (node(), node());
    a.admit(da, a.campaign(), Some(anna.key()), NOW).unwrap();
    a.admit(db, a.campaign(), Some(bert.key()), NOW).unwrap();
    a.revoke_invite(&bert.id, NOW).unwrap();
    a.rotate_campaign_key();
    let back = Authority::from_bytes(&a.to_bytes()).unwrap();
    assert_eq!(back.invites(), a.invites());
    assert!(back.is_retired(&db));
    assert_eq!(back.members()[&da].invite, Some(anna.id));
    assert_eq!(back.key_generation(), 1);
}

#[test]
fn assigned_characters_follow_the_claim_into_the_campaign_file() {
    use chummer_core::campaign::{Campaign, Member, MemberKind};
    use chummer_sync::hosted;
    let (mut a, _) = authority();
    let mut campaign = Campaign::new("Seattle");
    let pc = campaign.add(Member::embedded(MemberKind::Player, &munin(1)));
    hosted::reconcile(&mut a, &campaign, None, |_| None);
    let id = hosted::character_id(pc);
    let i = a.create_invite(Role::Player, "Anna", Some(id.clone()), None, NOW).clone();
    let d = node();
    a.admit(d, a.campaign(), Some(i.key()), NOW).unwrap();
    assert_eq!(a.owner(&id), Some(d));
    // The file does not know yet; reconciling it must not undo the claim.
    campaign.member_mut(pc).unwrap().owner = Some(hosted::GM_OWNER.into());
    hosted::reconcile(&mut a, &campaign, None, |_| None);
    assert_eq!(a.owner(&id), Some(d));
    assert!(hosted::adopt_owner_changes(&mut a, &mut campaign));
    assert_eq!(campaign.member(pc).unwrap().owner, Some(d.to_string()));
    assert!(!a.has_owner_changes());
    // A file naming a revoked node does not bring it back either.
    a.revoke_invite(&i.id, NOW).unwrap();
    hosted::reconcile(&mut a, &campaign, None, |_| None);
    assert!(a.role(&d).is_none());
}

#[test]
fn rotated_campaign_key_is_used_once_a_member_has_it() {
    let (mut a, _) = authority();
    let i = a.create_invite(Role::Player, "Anna", None, None, NOW).clone();
    let d = node();
    a.admit(d, a.campaign(), Some(i.key()), NOW).unwrap();
    a.mark_membership_sent(d);
    assert_eq!(a.mail_key_generation(&d), 0);
    assert_eq!(a.rotate_campaign_key(), 1);
    assert_eq!(a.mail_key_generation(&d), 0, "until the member was told the new key");
    assert!(a.membership_stale(&d));
    a.mark_membership_sent(d);
    assert_eq!(a.mail_key_generation(&d), 1);
}

#[test]
fn invites_file_ops_apply_once() {
    use chummer_sync::hosted;
    use chummer_sync::invites::{Invite, InviteOp};
    let dir = std::env::temp_dir().join(format!("chummer-sync-invites-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("c.invites");
    let (mut a, _) = authority();
    let inv = Invite::new(Role::Player, "Anna", None, None, NOW);
    hosted::append_invite_op(&file, &InviteOp::Create { invite: inv.clone() }).unwrap();
    hosted::append_invite_op(&file, &InviteOp::Revoke { id: inv.id }).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600, "it holds member keys");
    }
    assert_eq!(hosted::pending_invite_ops(&file).len(), 2);
    assert_eq!(hosted::merge_invites(&mut a, &file), 2);
    // Not done yet (not saved): taking them again gives the same, and
    // applying them again changes nothing.
    assert_eq!(hosted::merge_invites(&mut a, &file), 0);
    hosted::invite_ops_done(&file);
    assert!(hosted::pending_invite_ops(&file).is_empty());
    assert_eq!(a.invite(&inv.id).unwrap().state(NOW).clone(), InviteState::Revoked { at: a.invite(&inv.id).unwrap().revoked.unwrap() });
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- through a relay -----

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-sync-inv-{}-{name}", std::process::id()));
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

async fn until(what: impl Fn() -> bool) -> Result<()> {
    tokio::time::timeout(WAIT, async {
        while !what() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?;
    Ok(())
}

/// A player device: its key, endpoint and session on `link`.
struct Device {
    ep: Endpoint,
    session: PlayerSession,
}

async fn device(relay: &RelayNode, name: &str, link: &InviteLink) -> Result<Device> {
    let key = SecretKey::generate();
    let ep = endpoint(relay, key.clone()).await?;
    let mut cfg = PlayerConfig::new(name, link.clone());
    cfg.mailbox = relay.mailbox_id();
    cfg.connect_timeout = Duration::from_secs(5);
    let session = PlayerSession::new(ep.clone(), key, engine(), cfg)?;
    Ok(Device { ep, session })
}

struct Gm {
    ep: Endpoint,
    host: AuthorityHost,
    mailbox: MailboxClient,
    router: Option<Router>,
}

async fn gm(relay: &RelayNode, live: bool) -> Result<Gm> {
    let key = SecretKey::generate();
    let ep = endpoint(relay, key.clone()).await?;
    let host = AuthorityHost::new(Authority::new(CampaignId::random(), ep.id(), "GM"), engine(), key, None);
    let router = live.then(|| Router::builder(ep.clone()).accept(CAMPAIGN_ALPN, host.protocol()).spawn());
    let mailbox = MailboxClient::connect(&ep, dial_addr(relay.mailbox_id().unwrap(), None)).await?;
    Ok(Gm { ep, host, mailbox, router })
}

/// Revoking a member hangs up on them, refuses their joins and their mail
/// at the relay; the other member is unaffected.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_cuts_live_connection_and_mail_others_unaffected() -> Result<()> {
    let relay = relay("revoke").await?;
    let g = gm(&relay, true).await?;
    let (anna_inv, anna_link) = g.host.create_invite(Role::Player, "Anna", None, None, None);
    let (_, bert_link) = g.host.create_invite(Role::Player, "Bert", None, None, None);
    g.host.sync_mail(&g.mailbox).await?;
    let anna = device(&relay, "Anna", &anna_link).await?;
    let bert = device(&relay, "Bert", &bert_link).await?;
    assert_eq!(anna.session.sync().await, SyncMode::Online);
    assert_eq!(bert.session.sync().await, SyncMode::Online);
    let (ca, cb) = (CharacterId::new("anna"), CharacterId::new("bert"));
    g.host.authority().add_character(ca.clone(), Some(anna.ep.id()), munin(10))?;
    g.host.authority().add_character(cb.clone(), Some(bert.ep.id()), munin(10))?;
    g.host.changed();
    until(|| anna.session.replica().version(&ca).is_some() && bert.session.replica().version(&cb).is_some()).await?;

    // Anna's mail gets in while she is a member.
    anna.session.send_mail().await?;

    g.host.revoke_invite(&anna_inv.id).map_err(anyhow::Error::msg)?;
    until(|| !anna.session.is_online()).await?;
    assert!(matches!(anna.session.connect().await, Err(NetError::Denied(DenyReason::Revoked))));
    assert_eq!(anna.session.denied(), Some(DenyReason::Revoked));
    // The next mailbox round registers the GM's keys without hers.
    let r = g.host.sync_mail(&g.mailbox).await?;
    assert_eq!(r.status.as_ref().map(|s| s.keys), Some(1));
    anna.session.edit_now(&ca, gain(1.0, "after the revoke"))?;
    match anna.session.send_mail().await {
        Err(NetError::Mailbox(MailboxError::NotAllowed)) => {}
        other => panic!("Anna's mail should be refused, got {other:?}"),
    }
    // Bert goes on, live and by mail.
    bert.session.edit(&cb, gain(3.0, "still here")).await?;
    assert_eq!(g.host.authority().character(&cb).unwrap().karma, 13);
    assert!(bert.session.is_online());
    bert.session.send_mail().await?;
    assert_eq!(g.host.authority().character(&ca).unwrap().karma, 10, "nothing of Anna's after the revoke");

    anna.session.close();
    bert.session.close();
    if let Some(r) = g.router {
        r.shutdown().await?;
    }
    for ep in [anna.ep, bert.ep, g.ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

/// A re-issued link moves the member to a new device: the old one is hung
/// up on and told its link was replaced; the new one gets the characters.
#[tokio::test(flavor = "multi_thread")]
async fn reissued_link_supersedes_the_old_one() -> Result<()> {
    let relay = relay("reissue").await?;
    let g = gm(&relay, true).await?;
    let (inv, link) = g.host.create_invite(Role::Player, "Anna", None, None, None);
    let old = device(&relay, "Anna", &link).await?;
    assert_eq!(old.session.sync().await, SyncMode::Online);
    let c = CharacterId::new("anna");
    g.host.authority().add_character(c.clone(), Some(old.ep.id()), munin(10))?;
    g.host.changed();
    until(|| old.session.replica().version(&c).is_some()).await?;

    let new_link = g.host.reissue_invite(&inv.id, None, None).map_err(anyhow::Error::msg)?;
    assert_ne!(new_link.member, link.member);
    until(|| !old.session.is_online()).await?;
    assert!(matches!(old.session.connect().await, Err(NetError::Denied(DenyReason::Superseded))));

    let new = device(&relay, "Anna", &new_link).await?;
    assert_eq!(new.session.sync().await, SyncMode::Online);
    until(|| new.session.replica().version(&c).is_some()).await?;
    assert_eq!(g.host.authority().owner(&c), Some(new.ep.id()));
    assert_eq!(new.session.label().as_deref(), Some("Anna"));
    new.session.edit(&c, gain(2.0, "new laptop")).await?;
    assert_eq!(g.host.authority().character(&c).unwrap().karma, 12);
    // The old link is no good on yet another device either.
    let third = device(&relay, "Mallory", &link).await?;
    assert!(matches!(third.session.connect().await, Err(NetError::Denied(DenyReason::Superseded))));

    for s in [&old.session, &new.session, &third.session] {
        s.close();
    }
    if let Some(r) = g.router {
        r.shutdown().await?;
    }
    for ep in [old.ep, new.ep, third.ep, g.ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

/// The GM is never online live: a player joins by mail with a fresh
/// invite (and is given the assigned character); the relay then takes
/// the invite's key only from that device, so a second device with the
/// same link is refused there and told that it was claimed.
#[tokio::test(flavor = "multi_thread")]
async fn first_join_by_mail_and_a_leaked_link() -> Result<()> {
    let relay = relay("mailjoin").await?;
    let g = gm(&relay, false).await?;
    let c = CharacterId::new("pc");
    g.host.authority().add_character(c.clone(), None, munin(10))?;
    let (inv, link) = g.host.create_invite(Role::Player, "Anna", Some(c.clone()), Some(u64::MAX / 2), None);
    let link: InviteLink = link.to_string().parse()?;
    assert!(link.gm_key.is_some());
    // The GM's mailbox takes the unclaimed invite's key.
    g.host.sync_mail(&g.mailbox).await?;

    let anna = device(&relay, "Anna", &link).await?;
    assert_eq!(anna.session.sync().await, SyncMode::Mailbox, "registers the GM's key, mails the claim");
    let r = g.host.sync_mail(&g.mailbox).await?;
    assert_eq!(r.handled, 1, "{r:?}");
    assert!(matches!(g.host.authority().invite(&inv.id).unwrap().state(0), InviteState::Claimed { .. }));
    assert_eq!(g.host.authority().owner(&c), Some(anna.ep.id()));
    assert_eq!(anna.session.sync().await, SyncMode::Mailbox);
    assert_eq!(anna.session.replica().version(&c), Some(0), "the assigned character came by mail");
    assert_eq!(anna.session.label().as_deref(), Some("Anna"));
    anna.session.edit(&c, gain(5.0, "by post")).await?;
    anna.session.sync().await;
    g.host.sync_mail(&g.mailbox).await?;
    assert_eq!(g.host.authority().character(&c).unwrap().karma, 15);

    // The claim bound Anna's key to her device at the relay.
    let regs = g.host.authority().mail_registrations(NOW);
    assert!(regs.contains(&Registration::bound(link.member.as_ref().unwrap().key().public(), anna.ep.id())), "{regs:?}");

    // The link leaks to another device, which tries by mail: the relay
    // refuses the key from that device, and the thief is told "claimed"
    // without the GM seeing anything.
    let thief = device(&relay, "Mallory", &link).await?;
    thief.session.sync().await;
    assert_eq!(thief.session.denied(), Some(DenyReason::Claimed));
    let r = g.host.sync_mail(&g.mailbox).await?;
    assert_eq!((r.fetched, r.refused), (0, 0), "{r:?}");
    assert!(r.status.as_ref().is_some_and(|s| s.refused_today >= 1), "{r:?}");
    thief.session.sync().await;
    assert_eq!(thief.session.denied(), Some(DenyReason::Claimed));
    assert!(thief.session.events().iter().any(|e| matches!(e, Event::Denied(DenyReason::Claimed))));
    assert!(thief.session.replica().characters().next().is_none());
    assert_eq!(g.host.authority().members().len(), 2, "GM and Anna");

    anna.session.close();
    thief.session.close();
    for ep in [anna.ep, thief.ep, g.ep] {
        ep.close().await;
    }
    relay.shutdown().await
}

/// A new GM campaign key reaches an offline member through normal sync:
/// mail stays signed with the old key until the member was told the new
/// one, and its app registers both.
#[tokio::test(flavor = "multi_thread")]
async fn rotated_campaign_key_reaches_an_offline_member() -> Result<()> {
    let relay = relay("rotate").await?;
    let g = gm(&relay, false).await?;
    let (_, link) = g.host.create_invite(Role::Player, "Anna", None, None, None);
    g.host.sync_mail(&g.mailbox).await?;
    let anna = device(&relay, "Anna", &link).await?;
    anna.session.sync().await;
    g.host.sync_mail(&g.mailbox).await?;
    anna.session.sync().await;
    let c = CharacterId::new("pc");
    g.host.authority().add_character(c.clone(), Some(anna.ep.id()), munin(1))?;
    g.host.changed();
    assert_eq!(g.host.rotate_campaign_key(), 1);
    assert_eq!(g.host.authority().gm_keys().len(), 2);
    for _ in 0..3 {
        g.host.sync_mail(&g.mailbox).await?;
        anna.session.sync().await;
    }
    assert_eq!(anna.session.replica().membership().unwrap().gm_keys, g.host.authority().gm_keys());
    assert_eq!(g.host.authority().mail_key_generation(&anna.ep.id()), 1);
    g.host.gm_edit(&c, gain(4.0, "after the rotation")).map_err(|e| anyhow::anyhow!(e.reason))?;
    g.host.sync_mail(&g.mailbox).await?;
    anna.session.sync().await;
    assert_eq!(anna.session.replica().character(&c).unwrap().karma, 5);

    anna.session.close();
    for ep in [anna.ep, g.ep] {
        ep.close().await;
    }
    relay.shutdown().await
}
