//! Bugs found by the randomised sync tests (`chaos.rs`), each reduced to
//! the smallest exchange that shows it.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use chummer_core::career::ManualExpense;
use chummer_core::character::Character;
use chummer_core::command::{self, Command};
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Role};
use chummer_net::{EndpointId, SecretKey};
use chummer_sync::msg::{ClientMessage, ServerMessage};
use chummer_sync::{Authority, CharacterId, Event, Replica};

fn engine() -> &'static Arc<Engine> {
    static E: OnceLock<Arc<Engine>> = OnceLock::new();
    E.get_or_init(|| Arc::new(Engine::load().expect("game data")))
}

fn munin() -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
    let mut ch = Character::load(&p).unwrap();
    ch.karma = 20;
    ch
}

fn gain(amount: f64) -> Command {
    Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount, reason: "test".into(), ..Default::default() } }
}

fn id(n: u8) -> EndpointId {
    SecretKey::from_bytes(&[n; 32]).public()
}

fn setup() -> (Authority, EndpointId, CharacterId, Replica) {
    let p = id(2);
    let mut auth = Authority::new(CampaignId([1; 16]), id(1), "GM");
    auth.add_member(p, Role::Player, "Alice");
    let c = CharacterId::new("c");
    auth.add_character(c.clone(), Some(p), munin()).unwrap();
    let mut r = Replica::new();
    rejoin(&mut auth, p, &mut r);
    (auth, p, c, r)
}

fn rejoin(auth: &mut Authority, p: EndpointId, r: &mut Replica) -> Vec<Event> {
    let ClientMessage::Join { have, .. } = r.join_message("Alice") else { unreachable!() };
    let (membership, pushes) = auth.join(p, "Alice", &have).unwrap();
    r.handle(engine(), ServerMessage::Joined { membership, pushes })
}

fn resyncs(ev: &[Event]) -> usize {
    ev.iter().filter(|e| matches!(e, Event::NeedResync(_))).count()
}

/// A copy that drifted asks for a snapshot. When that request or its
/// answer is lost (the connection dropped), every later push, ack or
/// join answer used to be ignored without asking again, so a live session
/// stayed stuck (and its outbox never drained) until it fell back to the
/// mailbox.
#[test]
fn a_lost_resync_is_asked_for_again() {
    let (mut auth, p, c, mut r) = setup();
    // The GM changes the character; the push arrives damaged.
    let gm = auth.apply_local(engine(), &c, gain(3.0)).unwrap();
    assert_eq!(gm.notify, [p]);
    let mut push = auth.push_for(&p, &c).unwrap();
    push.hash = [0; 32];
    let ev = r.handle(engine(), ServerMessage::Push(push));
    assert_eq!(resyncs(&ev), 1);
    assert!(r.needs_resync(&c));
    // The resync answer is lost. The player edits and sends; the ack ...
    r.edit(engine(), &c, gain(1.0)).unwrap();
    let s = auth.submit(engine(), p, r.batch(&c).unwrap()).unwrap();
    let ev = r.handle(engine(), ServerMessage::Ack(s.ack));
    assert_eq!(resyncs(&ev), 1, "the ack asks for the snapshot again: {ev:?}");
    // ... and a rejoin both ask again.
    let ev = rejoin(&mut auth, p, &mut r);
    assert_eq!(resyncs(&ev), 1, "the join answer asks again: {ev:?}");
    let Some(Event::NeedResync(req)) = ev.into_iter().find(|e| matches!(e, Event::NeedResync(_))) else { unreachable!() };
    let snap = auth.resync(p, &req).unwrap();
    r.handle(engine(), ServerMessage::Push(snap));
    assert!(!r.needs_resync(&c));
    assert_eq!(r.version(&c), auth.version(&c));
    assert_eq!(r.confirmed_hash(&c), auth.hash(&c));
    assert!(r.outbox(&c).is_empty(), "the accepted edit is confirmed");
    assert_eq!(command::state_hash(r.character(&c).unwrap()), auth.hash(&c).unwrap());
}

/// The authority saves every couple of seconds, so a crash can take it
/// back to before changes it already acknowledged. Its copies then claim
/// versions it no longer has, and they used to ignore its snapshot (an
/// older version) and stay wrong until the authority happened to reach
/// their version again; edits made meanwhile vanished from the player's
/// view.
#[test]
fn copies_follow_an_authority_that_went_back_to_its_last_save() {
    let (mut auth, p, c, mut r) = setup();
    let saved = auth.to_bytes();
    for _ in 0..3 {
        r.edit(engine(), &c, gain(5.0)).unwrap();
    }
    let s = auth.submit(engine(), p, r.batch(&c).unwrap()).unwrap();
    r.handle(engine(), ServerMessage::Ack(s.ack));
    assert_eq!(r.version(&c), Some(3));
    // Crash: back to the save, without the three gains.
    let mut auth = Authority::from_bytes(&saved).unwrap();
    assert_eq!(auth.version(&c), Some(0));
    let karma = auth.character(&c).unwrap().karma;

    // On reconnecting, the copy follows the authority.
    rejoin(&mut auth, p, &mut r);
    assert!(r.version(&c).unwrap() > 3, "the authority moved past the versions it lost");
    assert_eq!(r.version(&c), auth.version(&c));
    assert_eq!(r.confirmed_hash(&c), auth.hash(&c));
    assert_eq!(r.character(&c).unwrap().karma, karma);

    // An edit made on the old copy before rejoining (a submit with a
    // base the authority does not have) lands and shows too.
    let (mut auth2, p2, c2, mut r2) = setup();
    let saved = auth2.to_bytes();
    r2.edit(engine(), &c2, gain(5.0)).unwrap();
    let s = auth2.submit(engine(), p2, r2.batch(&c2).unwrap()).unwrap();
    r2.handle(engine(), ServerMessage::Ack(s.ack));
    let mut auth2 = Authority::from_bytes(&saved).unwrap();
    r2.edit(engine(), &c2, gain(2.0)).unwrap();
    let s = auth2.submit(engine(), p2, r2.batch(&c2).unwrap()).unwrap();
    assert_eq!(s.ack.accepted.len(), 1);
    r2.handle(engine(), ServerMessage::Ack(s.ack));
    assert_eq!(r2.version(&c2), auth2.version(&c2));
    assert_eq!(r2.confirmed_hash(&c2), auth2.hash(&c2));
    assert!(r2.outbox(&c2).is_empty());
    assert_eq!(r2.character(&c2).unwrap().karma, 22);
}

fn chunk(message: [u8; 16], index: u32, total: u32, data: Vec<u8>) -> Vec<u8> {
    chummer_sync::msg::encode(&chummer_sync::mail::Chunk { message, index, total, data })
}

/// Chunks of mail are kept until their message is complete, and the
/// partial messages are saved with the replica or the authority. A member
/// (or anyone the GM's mail is from) could send chunks of a message that
/// never completes, with any `total`, and every one was kept: memory and
/// the saved file grew without bound (only the number of partial
/// messages was limited).
#[test]
fn partial_mail_is_bounded() {
    use chummer_sync::mail::Inbox;
    let sender = id(9);
    let mut inbox = Inbox::default();
    // One message that claims 4 billion chunks, 300 of 256 KiB sent.
    for i in 0..300 {
        assert!(inbox.accept(sender, &chunk([1; 16], i, u32::MAX, vec![i as u8; 256 * 1024])).unwrap().is_none());
    }
    // Many messages of many big chunks each.
    for m in 0..40u8 {
        for i in 0..20 {
            let _ = inbox.accept(sender, &chunk([m + 2; 16], i, 1000, vec![m; 256 * 1024]));
        }
    }
    let kept = postcard::to_stdvec(&inbox).unwrap().len();
    assert!(kept <= chummer_sync::mail::MAX_PARTIAL_BYTES + 64 * 1024, "{kept} bytes of partial mail kept");
    // A normal multi-chunk message still goes through afterwards.
    let m = chummer_sync::msg::MailMessage::Server(ServerMessage::Error("x".repeat(20_000)));
    let parts = chummer_sync::mail::split(&m, chummer_sync::mail::MIN_BLOB_LIMIT);
    assert!(parts.len() > 1);
    let mut done = None;
    for p in parts {
        done = inbox.accept(sender, &p).unwrap();
    }
    assert!(matches!(done, Some(chummer_sync::msg::MailMessage::Server(ServerMessage::Error(_)))));
}

/// The authority used to acknowledge changes before saving them (it saves
/// every couple of seconds), so a crash lost changes players had been
/// told were accepted (found by the Docker gm-crash scenario). Now each
/// is journaled before the answer goes out and replayed on start.
#[tokio::test(flavor = "multi_thread")]
async fn acknowledged_changes_survive_an_authority_crash() {
    use chummer_net::campaign::CampaignHandler;
    use chummer_sync::msg;
    use chummer_sync::AuthorityHost;
    let dir = std::env::temp_dir().join(format!("chummer-sync-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let crash = dir.join("crash");
    std::fs::create_dir_all(&crash).unwrap();
    let path = dir.join("campaign.authority");
    let (auth, p, c, mut r) = setup();
    let gm_key = SecretKey::from_bytes(&[1; 32]);
    let host = AuthorityHost::new(auth, engine().clone(), gm_key.clone(), Some(path.clone()));
    host.save().unwrap();
    let handler = host.protocol();
    let handler = handler.handler();
    for round in 0..3 {
        r.edit(engine(), &c, gain(1.0 + round as f64)).unwrap();
        let bytes = handler.submit(p, Role::Player, msg::encode(&ClientMessage::Submit(r.batch(&c).unwrap()))).await.unwrap();
        r.handle(engine(), msg::decode(&bytes).unwrap());
    }
    host.gm_edit(&c, gain(10.0)).unwrap();
    let want = {
        let a = host.authority();
        (a.version(&c), a.hash(&c))
    };
    assert_eq!(want.0, Some(4));
    // "kill -9": copy the files as they are now, without saving.
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        if f.path().is_file() {
            std::fs::copy(f.path(), crash.join(f.file_name())).unwrap();
        }
    }
    let side = crash.join("campaign.authority");
    let back = AuthorityHost::new(Authority::load(&side).unwrap(), engine().clone(), gm_key.clone(), Some(side.clone()));
    {
        let a = back.authority();
        assert_eq!((a.version(&c), a.hash(&c)), want, "every acknowledged change is back");
        assert_eq!(a.character(&c).unwrap().karma, 36);
    }
    // The GM's own sequence numbers go on (no op id reused).
    back.gm_edit(&c, gain(1.0)).unwrap();
    assert_eq!(back.authority().version(&c), Some(5));
    // After a save the journal is empty, and loading needs nothing from it.
    back.save().unwrap();
    assert!(chummer_sync::journal::Journal::read(&side).is_empty());
    let again = Authority::load(&side).unwrap();
    assert_eq!(again.version(&c), Some(5));
    std::fs::remove_dir_all(&dir).unwrap();
}
