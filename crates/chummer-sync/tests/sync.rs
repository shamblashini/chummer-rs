//! The authority and replicas in one process, messages passed by hand.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command};
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Role};
use chummer_net::{EndpointId, SecretKey};
use chummer_sync::mail::{self, Inbox};
use chummer_sync::msg::{self, ClientMessage, MailMessage, PushBody, ServerMessage};
use chummer_sync::{Authority, CharacterId, Event, Replica};

fn engine() -> &'static Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data")))
}

fn munin(karma: i32) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
    let mut ch = Character::load(&p).unwrap();
    ch.karma = karma;
    ch
}

fn id() -> EndpointId {
    SecretKey::generate().public()
}

fn gain(amount: f64, reason: &str) -> Command {
    Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount, reason: reason.into(), ..Default::default() } }
}

fn spend(amount: f64, reason: &str) -> Command {
    Command::ManualExpense { karma: true, gain: false, expense: ManualExpense { amount, reason: reason.into(), ..Default::default() } }
}

/// A skill Munin can raise, and its karma cost.
fn raisable(ch: &Character) -> (String, i32) {
    ch.skills
        .iter()
        .filter(|s| s.base + s.karma > 0 && s.base + s.karma < 6)
        .find_map(|s| chummer_core::career::karma::skill_upgrade_karma_cost(engine(), ch, &s.guid).map(|c| (s.guid.clone(), c)))
        .expect("a skill to raise")
}

struct Campaign {
    auth: Authority,
    gm: EndpointId,
    p1: EndpointId,
    p2: EndpointId,
    c1: CharacterId,
    c2: CharacterId,
}

/// A GM, two players, one character each.
fn campaign(karma: i32) -> Campaign {
    let gm = id();
    let (p1, p2) = (id(), id());
    let mut auth = Authority::new(CampaignId::random(), gm, "GM");
    auth.add_member(p1, Role::Player, "Alice");
    auth.add_member(p2, Role::Player, "Bob");
    let (c1, c2) = (CharacterId::new("munin-1"), CharacterId::new("munin-2"));
    auth.add_character(c1.clone(), Some(p1), munin(karma)).unwrap();
    auth.add_character(c2.clone(), Some(p2), munin(karma)).unwrap();
    Campaign { auth, gm, p1, p2, c1, c2 }
}

fn joined(auth: &mut Authority, peer: EndpointId, r: &mut Replica, name: &str) -> Vec<Event> {
    let ClientMessage::Join { name: _, have } = r.join_message(name) else { unreachable!() };
    let (membership, pushes) = auth.join(peer, name, &have).unwrap();
    r.handle(engine(), ServerMessage::Joined { membership, pushes })
}

fn submit(auth: &mut Authority, peer: EndpointId, r: &mut Replica, c: &CharacterId) -> (chummer_sync::Submitted, Vec<Event>) {
    let batch = r.batch(c).expect("something to send");
    let s = auth.submit(engine(), peer, batch).unwrap();
    let ev = r.handle(engine(), ServerMessage::Ack(s.ack.clone()));
    (s, ev)
}

fn assert_in_sync(auth: &Authority, r: &Replica, c: &CharacterId) {
    assert_eq!(r.version(c), auth.version(c), "version of {c}");
    assert_eq!(r.confirmed_hash(c), auth.hash(c), "hash of {c}");
    assert_eq!(command::state_hash(r.confirmed(c).unwrap()), auth.hash(c).unwrap(), "recomputed hash of {c}");
    assert!(r.outbox(c).is_empty());
    assert_eq!(command::state_hash(r.character(c).unwrap()), auth.hash(c).unwrap(), "shown state of {c}");
}

#[test]
fn concurrent_gm_and_player_edits_both_apply() {
    let mut k = campaign(50);
    let mut r1 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");
    assert_eq!(r1.characters().collect::<Vec<_>>(), [&k.c1]);
    assert_in_sync(&k.auth, &r1, &k.c1);

    // The player raises a skill offline...
    let (skill, cost) = raisable(r1.character(&k.c1).unwrap());
    r1.edit(engine(), &k.c1, Command::RaiseSkill { skill: skill.clone() }).unwrap();
    assert_eq!(r1.character(&k.c1).unwrap().karma, 50 - cost);
    assert_eq!(r1.outbox(&k.c1).len(), 1);

    // ...while the GM gives karma.
    let gm = k.auth.apply_local(engine(), &k.c1, gain(100.0, "Good run")).unwrap();
    assert!(gm.accepted.changed);
    assert_eq!(gm.notify, [k.p1], "the owner hears about GM edits");

    // The player's command was made on version 0; the authority is at 1.
    let (s, ev) = submit(&mut k.auth, k.p1, &mut r1, &k.c1);
    assert_eq!(s.ack.accepted.len(), 1);
    assert!(s.ack.accepted[0].rebased);
    assert!(s.ack.rejected.is_empty());
    assert!(s.notify.is_empty(), "nobody else sees this character");
    assert!(ev.contains(&Event::Updated(k.c1.clone())));
    assert_eq!(k.auth.version(&k.c1), Some(2));
    assert_eq!(k.auth.character(&k.c1).unwrap().karma, 150 - cost);
    assert_in_sync(&k.auth, &r1, &k.c1);
    assert_eq!(r1.character(&k.c1).unwrap().karma, 150 - cost);

    // Both are in the feed, with their authors.
    let feed: Vec<String> = k.auth.feed().iter().map(|f| f.to_string()).collect();
    assert_eq!(feed[0], "GM: Gained 100 karma: Good run");
    assert!(feed[1].starts_with("Alice: "), "{feed:?}");
    let mine: Vec<String> = r1.feed().iter().map(|f| f.to_string()).collect();
    assert_eq!(mine, feed, "the player's feed shows the same");
    // The log records who did it.
    let log = k.auth.log(&k.c1);
    assert_eq!(log[0].author, k.gm);
    assert_eq!(log[1].author, k.p1);
    assert_eq!(log[1].env.author, k.p1.to_string(), "the author is the proven sender, not what the client wrote");
}

#[test]
fn command_refused_after_rebase_is_reported() {
    let mut k = campaign(0);
    let (skill, cost) = raisable(k.auth.character(&k.c1).unwrap());
    // Exactly enough karma for the raise.
    k.auth.apply_local(engine(), &k.c1, gain(cost as f64, "Payout")).unwrap();
    let mut r1 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");
    r1.edit(engine(), &k.c1, Command::RaiseSkill { skill }).expect("affordable locally");

    // The GM spends it first.
    k.auth.apply_local(engine(), &k.c1, spend(cost as f64, "Bribe")).unwrap();

    let (s, ev) = submit(&mut k.auth, k.p1, &mut r1, &k.c1);
    assert!(s.ack.accepted.is_empty());
    assert_eq!(s.ack.rejected.len(), 1);
    let reason = s.ack.rejected[0].reason.clone();
    assert!(!reason.is_empty());
    let refused: Vec<_> = ev.iter().filter_map(|e| if let Event::Refused(r) = e { Some(r.clone()) } else { None }).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].reason, reason);
    assert!(refused[0].description.contains("(") || !refused[0].description.is_empty());
    assert_eq!(r1.refused(), refused.as_slice());
    // The refused raise is gone; the player sees the GM's state.
    assert_in_sync(&k.auth, &r1, &k.c1);
    assert_eq!(r1.character(&k.c1).unwrap().karma, 0);
    // The feed shows the refusal.
    let last = k.auth.feed().back().unwrap();
    assert_eq!(last.rejected.as_deref(), Some(reason.as_str()));
    assert_eq!(last.author_name, "Alice");
    assert_eq!(r1.dismiss_refused().len(), 1);
    assert!(r1.refused().is_empty());
}

#[test]
fn drift_is_detected_and_resynced_from_a_snapshot() {
    let mut k = campaign(20);
    let mut r1 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");

    // A push whose hash does not match what the replica computes.
    k.auth.apply_local(engine(), &k.c1, gain(5.0, "Tip")).unwrap();
    let mut push = k.auth.push_for(&k.p1, &k.c1).unwrap();
    assert!(matches!(push.body, PushBody::Entries(ref e) if e.len() == 1));
    push.hash = [7; 32];
    let ev = r1.handle(engine(), ServerMessage::Push(push));
    let req = ev.iter().find_map(|e| if let Event::NeedResync(r) = e { Some(r.clone()) } else { None }).expect("drift noticed");
    assert!(r1.needs_resync(&k.c1));
    assert_eq!(r1.resync_requests(), std::slice::from_ref(&req));

    // Edits made meanwhile survive the resync.
    r1.edit(engine(), &k.c1, Command::SetField { key: "alias".into(), value: "Raven".into() }).unwrap();

    let snap = k.auth.resync(k.p1, &req).unwrap();
    assert!(snap.is_snapshot());
    let ev = r1.handle(engine(), ServerMessage::Push(snap));
    assert!(ev.contains(&Event::Updated(k.c1.clone())), "{ev:?}");
    assert!(!r1.needs_resync(&k.c1));
    assert_eq!(r1.confirmed_hash(&k.c1), k.auth.hash(&k.c1));
    assert_eq!(r1.outbox(&k.c1).len(), 1);
    assert_eq!(r1.character(&k.c1).unwrap().field("alias"), "Raven");

    // A submit from a wrong base hash gets a snapshot back.
    let mut batch = r1.batch(&k.c1).unwrap();
    batch.base_hash = [1; 32];
    let s = k.auth.submit(engine(), k.p1, batch).unwrap();
    assert!(s.ack.update.is_snapshot());
    assert_eq!(s.ack.accepted.len(), 1);
    r1.handle(engine(), ServerMessage::Ack(s.ack));
    assert_in_sync(&k.auth, &r1, &k.c1);
    assert_eq!(k.auth.character(&k.c1).unwrap().field("alias"), "Raven");
}

#[test]
fn replayed_commands_run_once() {
    let mut k = campaign(0);
    let mut r1 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");
    r1.edit(engine(), &k.c1, gain(10.0, "Run")).unwrap();
    r1.edit(engine(), &k.c1, gain(1.0, "Bonus")).unwrap();
    let batch = r1.batch(&k.c1).unwrap();

    let first = k.auth.submit(engine(), k.p1, batch.clone()).unwrap();
    assert_eq!(k.auth.version(&k.c1), Some(2));
    let second = k.auth.submit(engine(), k.p1, batch.clone()).unwrap();
    assert_eq!(k.auth.version(&k.c1), Some(2), "a resubmitted batch changes nothing");
    assert_eq!(first.ack.accepted, second.ack.accepted, "and gets the same answer");
    assert_eq!(k.auth.character(&k.c1).unwrap().karma, 11);

    // Still after the authority was saved and loaded again.
    let mut back = Authority::from_bytes(&k.auth.to_bytes()).unwrap();
    assert_eq!(back.hash(&k.c1), k.auth.hash(&k.c1));
    let third = back.submit(engine(), k.p1, batch).unwrap();
    assert_eq!(back.version(&k.c1), Some(2));
    assert_eq!(third.ack.accepted, first.ack.accepted);
    assert_eq!(back.feed().len(), k.auth.feed().len());

    // The replica takes either answer, and duplicates, in any order.
    r1.handle(engine(), ServerMessage::Ack(second.ack));
    r1.handle(engine(), ServerMessage::Ack(first.ack));
    r1.handle(engine(), ServerMessage::Ack(third.ack));
    assert_in_sync(&k.auth, &r1, &k.c1);
}

#[test]
fn replica_keeps_offline_work_across_restarts() {
    let mut k = campaign(30);
    let mut r1 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");
    let (skill, cost) = raisable(r1.character(&k.c1).unwrap());
    r1.edit(engine(), &k.c1, Command::RaiseSkill { skill }).unwrap();
    r1.edit(engine(), &k.c1, Command::SetField { key: "alias".into(), value: "Offline".into() }).unwrap();
    let shown = command::state_hash(r1.character(&k.c1).unwrap());

    let dir = std::env::temp_dir().join(format!("chummer-sync-replica-{}", std::process::id()));
    let path = dir.join("campaign.replica");
    r1.save(&path).unwrap();
    drop(r1);
    let mut r1 = Replica::load(engine(), &path).unwrap();
    assert_eq!(r1.outbox(&k.c1).len(), 2);
    assert_eq!(command::state_hash(r1.character(&k.c1).unwrap()), shown);
    assert_eq!(r1.version(&k.c1), Some(0));
    assert!(r1.membership().is_some());

    // A new edit after the restart gets a fresh op id.
    r1.edit(engine(), &k.c1, gain(2.0, "Later")).unwrap();
    let ids: std::collections::BTreeSet<_> = r1.outbox(&k.c1).iter().map(|p| p.op.id).collect();
    assert_eq!(ids.len(), 3);

    let (s, _) = submit(&mut k.auth, k.p1, &mut r1, &k.c1);
    assert_eq!(s.ack.accepted.len(), 3);
    assert_in_sync(&k.auth, &r1, &k.c1);
    assert_eq!(k.auth.character(&k.c1).unwrap().karma, 30 - cost + 2);
    std::fs::remove_dir_all(dir).unwrap();

    // A damaged file is an error, not a panic.
    assert!(Replica::from_bytes(engine(), b"CRSR\0\x01junk").is_err());
    assert!(Replica::from_bytes(engine(), b"nope").is_err());
}

#[test]
fn players_see_only_their_own_characters_and_the_gm_sees_all() {
    let mut k = campaign(10);
    let mut r1 = Replica::new();
    let mut r2 = Replica::new();
    joined(&mut k.auth, k.p1, &mut r1, "Alice");
    joined(&mut k.auth, k.p2, &mut r2, "Bob");
    assert_eq!(r1.characters().collect::<Vec<_>>(), [&k.c1]);
    assert_eq!(r2.characters().collect::<Vec<_>>(), [&k.c2]);
    let m1 = r1.membership().unwrap();
    assert_eq!(m1.role, Role::Player);
    assert_eq!(m1.characters.iter().map(|c| &c.id).collect::<Vec<_>>(), [&k.c1]);
    assert_eq!(k.auth.membership(&k.gm).unwrap().characters.len(), 2);
    assert_eq!(k.auth.visible(&k.gm).len(), 2);

    // Bob cannot submit for Alice's character, nor ask for it.
    r2.edit(engine(), &k.c2, gain(1.0, "x")).unwrap();
    let mut batch = r2.batch(&k.c2).unwrap();
    batch.character = k.c1.clone();
    assert!(k.auth.submit(engine(), k.p2, batch).unwrap_err().contains("not yours"));
    assert!(k.auth.resync(k.p2, &chummer_sync::msg::ResyncRequest { character: k.c1.clone(), have_version: 0 }).is_err());
    assert!(k.auth.push_for(&k.p2, &k.c1).is_none());
    assert_eq!(k.auth.version(&k.c1), Some(0));
    // A stranger cannot do anything.
    let stranger = id();
    assert!(k.auth.submit(engine(), stranger, r1.batch(&k.c2).unwrap_or_else(|| r2.batch(&k.c2).unwrap())).is_err());
    assert!(k.auth.join(stranger, "Eve", &[]).is_err());

    // GM edits reach only the owner.
    let r = k.auth.apply_local(engine(), &k.c2, gain(100.0, "Well done")).unwrap();
    assert_eq!(r.notify, [k.p2]);
    let push = k.auth.push_for(&k.p2, &k.c2).unwrap();
    r2.handle(engine(), ServerMessage::Push(push.clone()));
    k.auth.mark_sent(k.p2, &ServerMessage::Push(push));
    assert!(k.auth.push_for(&k.p2, &k.c2).is_none(), "delivered");
    assert_eq!(r2.version(&k.c2), Some(1));
    assert_eq!(r2.feed().back().unwrap().to_string(), "GM: Gained 100 karma: Well done");
    assert!(r1.feed().is_empty(), "Alice hears nothing about Bob's character");

    // Giving a character to someone else moves it.
    assert!(k.auth.set_owner(&k.c2, Some(k.p1)));
    let out = k.auth.outgoing_for(&k.p1);
    for m in out {
        k.auth.mark_sent(k.p1, &m);
        r1.handle(engine(), m);
    }
    assert_eq!(r1.characters().count(), 2);
    let out = k.auth.outgoing_for(&k.p2);
    for m in out {
        let ev = r2.handle(engine(), m);
        if ev.contains(&Event::Removed(k.c2.clone())) {
            assert_eq!(r2.characters().count(), 0);
        }
    }
    assert_eq!(r2.characters().count(), 0);
    assert_eq!(r1.confirmed_hash(&k.c2), k.auth.hash(&k.c2));
}

#[test]
fn mail_messages_split_and_reassemble() {
    let mut k = campaign(10);
    let p = k.p1;
    let msgs = k.auth.outgoing_for(&p);
    // Membership and a snapshot of the character.
    assert_eq!(msgs.len(), 2);
    let snap = MailMessage::Server(msgs[1].clone());
    let whole = msg::encode(&snap).len();
    let parts = mail::split(&snap, 8 * 1024);
    assert!(parts.len() > 1, "a {whole}-byte snapshot is chunked");
    assert!(parts.iter().all(|b| b.len() + mail::SEAL_OVERHEAD <= 8 * 1024 + 64));
    let mut inbox = Inbox::default();
    let mut got = None;
    for (i, b) in parts.iter().enumerate().rev() {
        let r = inbox.accept(k.gm, b).unwrap();
        if i > 0 {
            assert!(r.is_none());
            assert_eq!(inbox.pending(), 1);
        } else {
            got = r;
        }
    }
    let Some(MailMessage::Server(ServerMessage::Push(push))) = got else { panic!("reassembled") };
    let mut r1 = Replica::new();
    r1.handle(engine(), msgs[0].clone());
    r1.handle(engine(), ServerMessage::Push(push));
    assert_eq!(r1.confirmed_hash(&k.c1), k.auth.hash(&k.c1));

    // Outboxes split by command, not by bytes.
    for i in 0..40 {
        r1.edit(engine(), &k.c1, Command::SetField { key: "background".into(), value: format!("{i}{}", "x".repeat(300)) }).unwrap();
    }
    let batch = r1.batch(&k.c1).unwrap();
    let small = mail::split_batch(batch.clone(), mail::MIN_BLOB_LIMIT);
    assert!(small.len() > 1);
    assert_eq!(small.iter().map(|b| b.ops.len()).sum::<usize>(), 40);
    for b in &small {
        assert_eq!(mail::split(&MailMessage::Client(ClientMessage::Submit(b.clone())), mail::MIN_BLOB_LIMIT).len(), 1);
    }
    assert_eq!(mail::split_batch(batch, mail::DEFAULT_BLOB_LIMIT).len(), 1);
}
