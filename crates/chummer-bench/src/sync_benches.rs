//! Online sync in one process: the authority's cost per change, a
//! replica catching up, and the mail path's encode and decode. No
//! network (as in chummer-sync/tests/sync.rs).

use std::time::Instant;

use anyhow::{ensure, Context, Result};
use chummer_bench::{ms, Record, Runner};
use chummer_core::career::ManualExpense;
use chummer_core::command::{self, Command};
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Role};
use chummer_net::{seal, EndpointId, SecretKey};
use chummer_sync::mail::{self, Inbox};
use chummer_sync::msg::{ClientMessage, MailMessage, ServerMessage};
use chummer_sync::{Authority, CharacterId, Replica};

use crate::{engine, load, slug, BIG, HEAVY};

fn gain(i: usize) -> Command {
    Command::ManualExpense { karma: true, gain: true, expense: ManualExpense { amount: 1.0, reason: format!("Run {i}"), ..Default::default() } }
}

fn note(i: usize) -> Command {
    Command::SetField { key: "background".into(), value: format!("{i} {}", "x".repeat(300)) }
}

/// A GM, one player and the heavy character, owned by the player.
struct Table {
    auth: Authority,
    gm: SecretKey,
    player: SecretKey,
    p1: EndpointId,
    c1: CharacterId,
}

fn table() -> Result<Table> {
    let (gm, player) = (SecretKey::generate(), SecretKey::generate());
    let p1 = player.public();
    let mut auth = Authority::new(CampaignId::random(), gm.public(), "GM");
    auth.add_member(p1, Role::Player, "Alice");
    let c1 = CharacterId::new("heavy");
    auth.add_character(c1.clone(), Some(p1), load(HEAVY)?).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(Table { auth, gm, player, p1, c1 })
}

/// A replica that joined `t` (it has the character's current version).
fn joined(engine: &Engine, t: &mut Table) -> Replica {
    let mut r = Replica::new();
    let ClientMessage::Join { have, .. } = r.join_message(engine, "Alice", None) else { unreachable!() };
    let (membership, pushes) = t.auth.join(t.p1, "Alice", &have).expect("join");
    r.handle(engine, ServerMessage::Joined { membership, pushes });
    r
}

pub fn run(r: &mut Runner) -> Result<()> {
    if !r.wants("sync.") {
        return Ok(());
    }
    let engine = engine()?;
    for name in [HEAVY, BIG] {
        let s = slug(name);
        if !(r.wants("sync.state_hash.") || r.wants("sync.snapshot.") || r.wants("sync.restore.")) {
            break;
        }
        let ch = load(name)?;
        r.bench(&format!("sync.state_hash.{s}"), 15, || {
            let _ = command::state_hash(&ch);
        });
        r.bench(&format!("sync.snapshot.{s}"), 10, || {
            let _ = command::snapshot(&ch);
        });
        let snap = command::snapshot(&ch);
        r.bench(&format!("sync.restore.{s}"), 10, || {
            let _ = command::restore(&snap).expect("restore");
        });
    }
    let s = slug(HEAVY);
    round_trips(r, &engine, &s)?;
    catch_up(r, &engine, &s)?;
    mail_path(r, &engine, &s)?;
    Ok(())
}

/// A player's edit: made on the replica, submitted, acknowledged.
fn round_trips(r: &mut Runner, engine: &Engine, s: &str) -> Result<()> {
    if !(r.wants("sync.replica.") || r.wants("sync.authority.submit")) {
        return Ok(());
    }
    let mut t = table()?;
    let mut rep = joined(engine, &mut t);
    let n = r.samples(20);
    let (mut edit, mut submit, mut ack) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..=n {
        let t0 = Instant::now();
        rep.edit(engine, &t.c1, note(i)).map_err(|e| anyhow::anyhow!(e.reason))?;
        let e = ms(t0);
        let batch = rep.batch(&t.c1).context("a batch")?;
        let t0 = Instant::now();
        let sub = t.auth.submit(engine, t.p1, batch).map_err(anyhow::Error::msg)?;
        let a = ms(t0);
        let t0 = Instant::now();
        rep.handle(engine, ServerMessage::Ack(sub.ack));
        let k = ms(t0);
        // The first round is the warm-up.
        if i > 0 {
            edit.push(e);
            submit.push(a);
            ack.push(k);
        }
    }
    ensure!(rep.confirmed_hash(&t.c1) == t.auth.hash(&t.c1), "the replica follows the authority");
    r.push(Record::from_samples(&format!("sync.replica.edit.{s}"), "ms", edit).with_note("a player's edit on its replica (applied, queued)"));
    r.push(Record::from_samples(&format!("sync.authority.submit.{s}"), "ms", submit).with_note("the authority runs a one-command batch: apply, state hash, log"));
    r.push(Record::from_samples(&format!("sync.replica.ack.{s}"), "ms", ack).with_note("the replica takes the authority's answer"));
    Ok(())
}

/// A replica that was away while the GM made changes reconnects.
fn catch_up(r: &mut Runner, engine: &Engine, s: &str) -> Result<()> {
    if !(r.wants("sync.catchup.") || r.wants("sync.authority.apply_local")) {
        return Ok(());
    }
    // Below the authority's log window, the backlog comes as entries the
    // replica replays; above it, as a snapshot.
    let window = chummer_sync::authority::LOG_WINDOW;
    let (entries, beyond) = if r.quick { (30, window + 40) } else { (200, 500) };
    let mut t = table()?;
    let away = joined(engine, &mut t).to_bytes();
    let mut apply = Vec::new();
    let mut i = 0;
    for (label, backlog) in [("entries", entries), ("snapshot", beyond)] {
        while i < backlog {
            let t0 = Instant::now();
            t.auth.apply_local(engine, &t.c1, gain(i)).map_err(|e| anyhow::anyhow!(e.reason))?;
            apply.push(ms(t0));
            i += 1;
        }
        let name = format!("sync.catchup.{label}.{s}");
        let mut ok = true;
        r.bench_manual(&name, 7, || {
            let mut rep = Replica::from_bytes(engine, &away).expect("replica");
            let ClientMessage::Join { have, .. } = rep.join_message(engine, "Alice", None) else { unreachable!() };
            let t0 = Instant::now();
            let (membership, pushes) = t.auth.join(t.p1, "Alice", &have).expect("join");
            rep.handle(engine, ServerMessage::Joined { membership, pushes });
            let took = ms(t0);
            ok &= rep.confirmed_hash(&t.c1) == t.auth.hash(&t.c1);
            // Entries are replayed one by one: per change, so that quick
            // runs (a shorter backlog) have the same budget.
            if label == "entries" { took / backlog as f64 } else { took }
        });
        ensure!(ok, "{name}: the replica caught up");
        if let Some(b) = r.report.benches.last_mut().filter(|b| b.name == name) {
            b.note = Some(if label == "entries" {
                format!("per change, {backlog} changes behind (below the log window of {window}): the authority's push and the replica replaying it")
            } else {
                format!("{backlog} changes behind (beyond the log window of {window}, so a snapshot): the authority's push and the replica restoring it")
            });
        }
    }
    r.push(Record::from_samples(&format!("sync.authority.apply_local.{s}"), "ms", apply).with_note("a GM edit at the authority: apply, state hash, log (every change of the catch-up backlogs)"));
    Ok(())
}

/// Sealing a batch of offline edits for the GM's mailbox, and opening it
/// there; the same for a snapshot push to a player.
fn mail_path(r: &mut Runner, engine: &Engine, s: &str) -> Result<()> {
    if !r.wants("sync.mail.") {
        return Ok(());
    }
    let mut t = table()?;
    let mut rep = joined(engine, &mut t);
    for i in 0..20 {
        rep.edit(engine, &t.c1, note(i)).map_err(|e| anyhow::anyhow!(e.reason))?;
    }
    let batch = rep.batch(&t.c1).context("a batch")?;
    let limit = mail::DEFAULT_BLOB_LIMIT;
    let encode_batch = || -> Vec<Vec<u8>> {
        mail::split_batch(batch.clone(), limit).into_iter().flat_map(|b| mail::split(&MailMessage::Client(ClientMessage::Submit(b)), limit)).map(|p| seal::seal(&t.player, &t.gm.public(), &p).expect("seal")).collect()
    };
    // A new member's first push is the character's snapshot.
    let newcomer = SecretKey::generate();
    t.auth.add_member(newcomer.public(), Role::Player, "Bob");
    t.auth.set_owner(&t.c1, Some(newcomer.public()));
    let push = t.auth.outgoing_for(&newcomer.public()).into_iter().find(|m| matches!(m, ServerMessage::Push(p) if p.is_snapshot())).context("a snapshot push")?;
    let snapshot = MailMessage::Server(push);
    let encode_snapshot = || -> Vec<Vec<u8>> { mail::split(&snapshot, limit).into_iter().map(|p| seal::seal(&t.gm, &newcomer.public(), &p).expect("seal")).collect() };
    fn decode(me: &SecretKey, blobs: &[Vec<u8>]) -> usize {
        let mut inbox = Inbox::default();
        let mut got = 0;
        for b in blobs {
            let o = seal::open(me, b).expect("open");
            if inbox.accept(o.sender, &o.payload).expect("decode").is_some() {
                got += 1;
            }
        }
        got
    }
    let blobs = encode_batch();
    let bytes: usize = blobs.iter().map(Vec::len).sum();
    r.bench(&format!("sync.mail.encode_batch20.{s}"), 15, || {
        let _ = encode_batch();
    });
    let mut got = 0;
    r.bench(&format!("sync.mail.decode_batch20.{s}"), 15, || got = decode(&t.gm, &blobs));
    ensure!(got >= 1, "the batch decodes");
    for b in r.report.benches.iter_mut().filter(|b| b.name.starts_with("sync.mail.") && b.name.contains("batch20")) {
        b.note = Some(format!("20 offline edits: split, postcard, seal ({} blob(s), {bytes} bytes); decode: open, reassemble", blobs.len()));
    }
    let blobs = encode_snapshot();
    let bytes: usize = blobs.iter().map(Vec::len).sum();
    r.bench(&format!("sync.mail.encode_snapshot.{s}"), 10, || {
        let _ = encode_snapshot();
    });
    r.bench(&format!("sync.mail.decode_snapshot.{s}"), 10, || got = decode(&newcomer, &blobs));
    ensure!(got == 1, "the snapshot decodes");
    for b in r.report.benches.iter_mut().filter(|b| b.name.starts_with("sync.mail.") && b.name.contains("snapshot")) {
        b.note = Some(format!("a snapshot push by mail: {} blob(s), {bytes} bytes", blobs.len()));
    }
    Ok(())
}
