//! Dice rolls at the table: players' rolls reaching the authority (live
//! and by mail, once each), who is sent which, and the bounded roll log
//! across saves. Messages passed by hand, as in `sync.rs`.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use chummer_core::campaign::RollSettings;
use chummer_core::character::Character;
use chummer_core::dice::RollRecord;
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Role};
use chummer_net::{EndpointId, SecretKey};
use chummer_sync::mail::{self, Inbox};
use chummer_sync::msg::{ClientMessage, MailMessage, ServerMessage, TableRoll};
use chummer_sync::{Authority, CharacterId, Event, Replica};

fn engine() -> &'static Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data")))
}

fn munin() -> Character {
    Character::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5")).unwrap()
}

fn id() -> EndpointId {
    SecretKey::generate().public()
}

/// A roll as the Play screen makes it.
fn pistols(dice: Vec<u8>) -> RollRecord {
    RollRecord { at: 1_000, label: "Pistols · Ares Predator V".into(), pool: dice.len() as u32, limit: Some(5), dice, ..Default::default() }
}

struct Table {
    auth: Authority,
    p1: EndpointId,
    p2: EndpointId,
    c1: CharacterId,
    npc: CharacterId,
    r1: Replica,
    r2: Replica,
}

/// A GM with an NPC, two players with a character each, both joined.
fn table() -> Table {
    let (gm, p1, p2) = (id(), id(), id());
    let mut auth = Authority::new(CampaignId::random(), gm, "GM");
    auth.add_member(p1, Role::Player, "Anna");
    auth.add_member(p2, Role::Player, "Bob");
    let (c1, c2, npc) = (CharacterId::new("anna-munin"), CharacterId::new("bob-munin"), CharacterId::new("npc"));
    auth.add_character(c1.clone(), Some(p1), munin()).unwrap();
    auth.add_character(c2.clone(), Some(p2), munin()).unwrap();
    auth.add_character(npc.clone(), None, munin()).unwrap();
    let (mut r1, mut r2) = (Replica::new(), Replica::new());
    for (p, r, name) in [(p1, &mut r1, "Anna"), (p2, &mut r2, "Bob")] {
        let ClientMessage::Join { have, .. } = r.join_message(name, None) else { unreachable!() };
        let (membership, pushes) = auth.join(p, name, &have).unwrap();
        r.handle(engine(), ServerMessage::Joined { membership, pushes });
        for m in auth.outgoing_for(&p) {
            auth.mark_sent(p, &m);
        }
    }
    Table { auth, p1, p2, c1, npc, r1, r2 }
}

/// The rolls `peer` is sent next (marked sent).
fn sent_rolls(auth: &mut Authority, peer: EndpointId) -> Vec<TableRoll> {
    let mut out = Vec::new();
    for m in auth.outgoing_for(&peer) {
        auth.mark_sent(peer, &m);
        if let ServerMessage::Rolls(r) = m {
            out.extend(r);
        }
    }
    out
}

/// The ids a [`ServerMessage::RollsTaken`] names.
fn taken(m: &ServerMessage) -> Vec<chummer_sync::msg::RollId> {
    let ServerMessage::RollsTaken(ids) = m else { panic!("not an answer to rolls: {m:?}") };
    ids.clone()
}

/// The authority's answer to `msg` from `peer`, as `host.rs` makes it.
fn answer(auth: &mut Authority, peer: EndpointId, msg: ClientMessage) -> ServerMessage {
    let ClientMessage::Rolls(rolls) = msg else { panic!("rolls") };
    ServerMessage::RollsTaken(auth.submit_rolls(peer, rolls).unwrap().0)
}

#[test]
fn a_player_roll_reaches_the_gm_once_live_and_by_mail() {
    let mut t = table();
    let rep = t.r1.roll(&t.c1, pistols(vec![6, 5, 5, 1, 2, 3, 6, 6])).unwrap();
    assert_eq!(t.r1.roll_outbox().len(), 1);

    // Live: sent, answered, and gone from the outbox.
    let live = t.r1.rolls_message().expect("a roll to send");
    let reply = answer(&mut t.auth, t.p1, live.clone());
    assert_eq!(taken(&reply), [rep.id]);
    // The answer was lost: the roll is sent again, and logged once.
    let again = answer(&mut t.auth, t.p1, live);
    assert_eq!(taken(&again), [rep.id], "answered the same way");
    assert_eq!(t.auth.rolls().len(), 1, "logged once");
    assert_eq!(t.r1.handle(engine(), reply), [Event::Rolls]);
    assert!(t.r1.roll_outbox().is_empty());
    assert_eq!(t.r1.table_rolls().len(), 1, "the player keeps their own roll");

    let logged = &t.auth.rolls()[0];
    assert_eq!((logged.author, logged.author_name.as_str(), logged.author_role), (t.p1, "Anna", Role::Player));
    assert_eq!(logged.who, munin().display_name(), "the character's name as the GM has it");
    assert_eq!(logged.character.as_ref(), Some(&t.c1));
    assert_eq!(logged.roll.label, "Pistols · Ares Predator V");
    assert_eq!(logged.roll.outcome().hits, 5, "worked out from the dice, capped by the limit");

    // By mail: chunked, reassembled, delivered twice (a replayed mailbox),
    // logged once.
    let rep2 = t.r1.roll(&t.c1, RollRecord { label: "Soak".into(), pool: 3, dice: vec![1, 1, 2], ..Default::default() }).unwrap();
    let unmailed = t.r1.unmailed_rolls();
    assert_eq!(unmailed.len(), 1);
    t.r1.mark_rolls_mailed(&[rep2.id]);
    assert!(t.r1.unmailed_rolls().is_empty(), "mailed once");
    let mail = MailMessage::Client(ClientMessage::Rolls(unmailed));
    for _ in 0..2 {
        let mut inbox = Inbox::default();
        let mut got = None;
        for b in mail::split(&mail, mail::MIN_BLOB_LIMIT) {
            got = inbox.accept(t.p1, &b).unwrap();
        }
        let Some(MailMessage::Client(m)) = got else { panic!("reassembled") };
        let reply = answer(&mut t.auth, t.p1, m);
        assert_eq!(taken(&reply), [rep2.id]);
        t.r1.handle(engine(), reply);
    }
    assert_eq!(t.auth.rolls().len(), 2);
    assert!(t.r1.roll_outbox().is_empty());
    assert_eq!(t.r1.table_rolls().len(), 2, "no roll of ours twice");

    // Rolls are a log: no character changed, nothing to push.
    assert_eq!(t.auth.version(&t.c1), Some(0));
    assert!(t.auth.feed().is_empty());

    // A roll whose dice do not fit its pool, or for a character that is
    // not theirs, is answered (so not sent forever) but not logged.
    let mut bad = t.r1.roll(&t.c1, pistols(vec![6, 6])).unwrap();
    bad.roll.pool = 5;
    let mut theirs = t.r1.roll(&t.c1, pistols(vec![4])).unwrap();
    theirs.character = Some(t.npc.clone());
    let (taken, _) = t.auth.submit_rolls(t.p1, vec![bad.clone(), theirs.clone()]).unwrap();
    assert_eq!(taken, [bad.id, theirs.id]);
    assert_eq!(t.auth.rolls().len(), 2);
}

#[test]
fn gm_rolls_reach_players_only_when_open() {
    let mut t = table();
    t.auth.add_gm_roll(Some(t.npc.clone()), "Ganger 2", false, pistols(vec![6, 2]));
    assert!(sent_rolls(&mut t.auth, t.p1).is_empty(), "the GM's rolls are private by default");
    assert!(!t.auth.has_outgoing(&t.p1));

    // One rolled openly.
    let (open, notify) = t.auth.add_gm_roll(Some(t.npc.clone()), "Ganger 2", true, pistols(vec![5, 5, 1]));
    assert!(notify.contains(&t.p1) && notify.contains(&t.p2));
    assert!(t.auth.has_outgoing(&t.p1));
    let got = sent_rolls(&mut t.auth, t.p1);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].id, got[0].who.as_str(), got[0].open), (open.id, "Ganger 2", true));
    assert_eq!(got[0].character, None, "the NPC's id is not shown to players");
    assert!(sent_rolls(&mut t.auth, t.p1).is_empty(), "sent once");
    assert!(!t.auth.has_outgoing(&t.p1));
    // The player takes it into the table's rolls.
    let ev = t.r1.handle(engine(), ServerMessage::Rolls(got.clone()));
    assert_eq!(ev, [Event::Rolls]);
    t.r1.handle(engine(), ServerMessage::Rolls(got));
    assert_eq!(t.r1.table_rolls().len(), 1, "a roll delivered twice is shown once");
    assert_eq!(sent_rolls(&mut t.auth, t.p2).len(), 1);
}

#[test]
fn players_see_each_others_rolls_by_the_campaign_setting() {
    let mut t = table();
    assert!(t.auth.roll_settings().players_see_each_other, "on by default");
    let r = t.r1.roll(&t.c1, pistols(vec![6, 4])).unwrap();
    let (_, notify) = t.auth.submit_rolls(t.p1, vec![r.clone()]).unwrap();
    assert!(notify.contains(&t.p2));
    assert!(sent_rolls(&mut t.auth, t.p1).is_empty(), "nobody is sent their own roll back");
    let got = sent_rolls(&mut t.auth, t.p2);
    assert_eq!(got.iter().map(|x| x.id).collect::<Vec<_>>(), [r.id]);
    assert_eq!(got[0].character, None, "Bob does not see Anna's character");
    t.r2.handle(engine(), ServerMessage::Rolls(got));
    assert_eq!(t.r2.table_rolls()[0].author_name, "Anna");

    t.auth.set_roll_settings(RollSettings { players_see_each_other: false, ..Default::default() });
    let r = t.r1.roll(&t.c1, pistols(vec![1, 1])).unwrap();
    t.auth.submit_rolls(t.p1, vec![r]).unwrap();
    assert!(sent_rolls(&mut t.auth, t.p2).is_empty(), "the GM turned it off");
    assert_eq!(t.auth.rolls().len(), 2, "the GM still has it");
}

#[test]
fn the_roll_log_is_bounded_and_survives_saves() {
    let mut t = table();
    for k in 0..250 {
        t.auth.add_gm_roll(None, &format!("Ganger {k}"), false, pistols(vec![3]));
    }
    let r = t.r1.roll(&t.c1, pistols(vec![6])).unwrap();
    let (ids, _) = t.auth.submit_rolls(t.p1, vec![r.clone()]).unwrap();
    t.r1.handle(engine(), ServerMessage::RollsTaken(ids));
    assert_eq!(t.auth.rolls().len(), chummer_sync::authority::ROLL_LOG);
    assert_eq!(t.auth.rolls().front().unwrap().who, "Ganger 51", "the oldest went");

    // The authority file keeps the log, the ids it took and the settings.
    t.auth.set_roll_settings(RollSettings { show_gm_rolls: true, players_see_each_other: false });
    let mut back = Authority::from_bytes(&t.auth.to_bytes()).unwrap();
    assert_eq!(back.rolls(), t.auth.rolls());
    assert_eq!(back.roll_settings(), t.auth.roll_settings());
    back.submit_rolls(t.p1, vec![r]).unwrap();
    assert_eq!(back.rolls().len(), 200);
    assert_eq!(back.rolls().back().unwrap().who, munin().display_name(), "the replayed roll is not logged twice");
    let (gm, _) = back.add_gm_roll(None, "Ganger 999", false, pistols(vec![2]));
    assert!(t.auth.rolls().iter().all(|x| x.id != gm.id), "the GM's roll ids go on");

    // The replica keeps rolls not answered yet and the table's rolls.
    let waiting = t.r1.roll(&t.c1, pistols(vec![5, 5])).unwrap();
    t.r1.handle(engine(), ServerMessage::Rolls(vec![gm]));
    let mut r1 = Replica::from_bytes(engine(), &t.r1.to_bytes()).unwrap();
    assert_eq!(r1.roll_outbox().iter().map(|p| p.report.id).collect::<Vec<_>>(), [waiting.id]);
    assert_eq!(r1.table_rolls(), t.r1.table_rolls());
    let next = r1.roll(&t.c1, pistols(vec![4])).unwrap();
    assert!(next.id.seq > waiting.id.seq, "no roll id reused");
}

/// The authority journals the rolls it answers (a player forgets a roll
/// once answered), and takes them back after a crash before its save.
#[test]
fn journaled_rolls_come_back_after_a_crash() {
    let dir = std::env::temp_dir().join(format!("chummer-sync-roll-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let side = dir.join("c.authority");
    let mut t = table();
    let saved = t.auth.to_bytes();
    let r = t.r1.roll(&t.c1, pistols(vec![6, 6, 2])).unwrap();
    t.auth.submit_rolls(t.p1, vec![r.clone()]).unwrap();
    t.auth.add_gm_roll(None, "Ganger", true, pistols(vec![3]));
    let rolled = t.auth.take_rolled();
    assert_eq!(rolled.len(), 2);
    chummer_sync::journal::Journal::new(&side).append(&[], &rolled).unwrap();

    let mut back = Authority::from_bytes(&saved).unwrap();
    let (entries, rolls) = chummer_sync::journal::Journal::read(&side);
    assert!(entries.is_empty());
    assert_eq!(rolls.iter().filter(|x| back.replay_roll(x)).count(), 2);
    assert_eq!(back.rolls(), t.auth.rolls());
    assert!(!back.replay_roll(&rolls[0]), "once");
    let (gm, _) = back.add_gm_roll(None, "Ganger", false, pistols(vec![1]));
    assert!(t.auth.rolls().iter().all(|x| x.id != gm.id), "the GM's roll ids go on after the replayed ones");
    std::fs::remove_dir_all(&dir).unwrap();
}
