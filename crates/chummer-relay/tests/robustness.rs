//! The mailbox under bad conditions, in process: damaged or unreadable
//! database files, many writers at once, big pages, a clock that jumps,
//! hostile frames, and a relay restarting under a connected client.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chummer_net::mailbox::{MailboxClient, MailboxError, MailboxRequest, MailboxResponse, MAILBOX_ALPN, MAX_FETCH_BYTES};
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, EndpointId, SecretKey};
use chummer_relay::service::MailboxService;
use chummer_relay::store::{Limits, Store};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};

const T0: u64 = 1_800_000_000;
const DAY: u64 = 24 * 60 * 60;
const WAIT: Duration = Duration::from_secs(20);

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-relay-robust-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn id() -> EndpointId {
    SecretKey::generate().public()
}

/// The key every test mailbox takes mail from.
fn key() -> chummer_net::PublicKey {
    SecretKey::from_bytes(&[42; 32]).public()
}

/// A put to `rcpt` (which takes mail signed by [`key`]).
fn put(s: &Store, sender: EndpointId, rcpt: EndpointId, blob: Vec<u8>, now: u64) -> Result<u64, MailboxError> {
    s.register(rcpt, [1; 16], &[key()], now)?;
    s.put(sender, rcpt, key(), chummer_net::invite::random_id(), blob, now)
}

/// Lets `me` take mail from its own node key (what a test client signs
/// with) through a mailbox connection.
async fn open_own_mailbox(mb: &MailboxClient, me: EndpointId) -> Result<()> {
    mb.register([1; 16], vec![me]).await?;
    Ok(())
}

/// Opens `path`, which must not panic; returns whether it opened.
fn opens(path: &Path) -> bool {
    match catch_unwind(AssertUnwindSafe(|| Store::open(path, Limits::default()))) {
        Ok(r) => r.is_ok(),
        Err(_) => panic!("opening {} panicked", path.display()),
    }
}

#[test]
fn damaged_database_files_are_errors_not_panics() {
    let d = dir("damaged");
    // A real database with mail in it, to damage.
    let good = d.join("good.redb");
    {
        let s = Store::open(&good, Limits::default()).unwrap();
        for i in 0..50u8 {
            put(&s, id(), id(), vec![i; 2000], T0).unwrap();
        }
    }
    let bytes = std::fs::read(&good).unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("zeros", vec![0; 64 * 1024]),
        ("garbage", (0..100_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect()),
        ("empty", Vec::new()),
        ("half", bytes[..bytes.len() / 2].to_vec()),
        ("header only", bytes[..512.min(bytes.len())].to_vec()),
        ("flipped", {
            let mut b = bytes.clone();
            for i in (0..b.len()).step_by(997) {
                b[i] ^= 0x5a;
            }
            b
        }),
    ];
    for (name, data) in cases {
        let p = d.join(format!("{name}.redb"));
        std::fs::write(&p, data).unwrap();
        // Either an error or a usable store; and a usable store works.
        if opens(&p) {
            let s = Store::open(&p, Limits::default()).unwrap();
            let r = catch_unwind(AssertUnwindSafe(|| {
                let b = id();
                let _ = put(&s, id(), b, vec![1], T0);
                let _ = s.fetch(b, 10, T0);
                let _ = s.purge(T0 + 40 * DAY);
            }));
            assert!(r.is_ok(), "using the {name} database panicked");
        }
    }
    // A directory where the file should be.
    let p = d.join("a-dir.redb");
    std::fs::create_dir_all(&p).unwrap();
    assert!(!opens(&p));
    std::fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
#[test]
fn unreadable_database_is_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("perm");
    let p = d.join("mailbox.redb");
    drop(Store::open(&p, Limits::default()).unwrap());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o400)).unwrap();
    // Root ignores file modes; only check when they apply.
    if std::fs::OpenOptions::new().write(true).open(&p).is_err() {
        assert!(!opens(&p), "a read-only database opened for writing");
    }
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn many_writers_at_once_get_unique_ids() {
    let d = dir("threads");
    let limits = Limits { max_messages_per_recipient: 10_000, max_messages_per_key: 10_000, ..Limits::default() };
    let s = Arc::new(Store::open(&d.join("m.redb"), limits).unwrap());
    let bob = id();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let s = s.clone();
            std::thread::spawn(move || {
                let me = id();
                (0..40).map(|i| put(&s, me, bob, vec![i; 100], T0).unwrap()).collect::<Vec<u64>>()
            })
        })
        .collect();
    let mut ids: Vec<u64> = threads.into_iter().flat_map(|t| t.join().unwrap()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 320);
    let (items, more) = s.fetch(bob, 1000, T0).unwrap();
    assert!(!more);
    assert_eq!(items.len(), 320);
    assert_eq!(s.ack(bob, &ids).unwrap(), 320);
    assert!(s.is_empty().unwrap());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn big_mail_is_fetched_in_pages_exactly_once() {
    let d = dir("pages");
    let limits = Limits { max_blob_bytes: 1 << 20, ..Limits::default() };
    let s = Store::open(&d.join("m.redb"), limits).unwrap();
    let bob = id();
    for i in 0..7u8 {
        put(&s, id(), bob, vec![i; 900 * 1024], T0).unwrap();
    }
    let mut got = Vec::new();
    loop {
        let (items, more) = s.fetch(bob, 100, T0).unwrap();
        let bytes: usize = items.iter().map(|i| i.blob.len()).sum();
        assert!(items.len() == 1 || bytes <= MAX_FETCH_BYTES, "{bytes} bytes in one page");
        assert!(!items.is_empty());
        got.extend(items.iter().map(|i| i.blob[0]));
        s.ack(bob, &items.iter().map(|i| i.id).collect::<Vec<_>>()).unwrap();
        if !more {
            break;
        }
    }
    assert_eq!(got, (0..7).collect::<Vec<u8>>(), "every message once, oldest first");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn clock_jumps_neither_lose_nor_resurrect_mail() {
    let d = dir("clock");
    let limits = Limits { expiry_secs: 10 * DAY, max_messages_per_sender_per_day: 3, ..Limits::default() };
    let s = Store::open(&d.join("m.redb"), limits).unwrap();
    let (a, b) = (id(), id());
    put(&s, a, b, vec![1], T0 + DAY).unwrap();
    // The clock goes back a day (NTP step): the mail is still there and
    // does not count as expired.
    assert_eq!(s.fetch(b, 10, T0).unwrap().0.len(), 1);
    assert_eq!(s.purge(T0).unwrap(), 0);
    // Back in yesterday the daily quota is yesterday's.
    for _ in 0..3 {
        put(&s, a, id(), vec![0], T0).unwrap();
    }
    assert!(matches!(put(&s, a, id(), vec![0], T0), Err(MailboxError::SenderQuota { .. })));
    // Then far forward: expired, not delivered, purged once.
    assert!(s.fetch(b, 10, T0 + 400 * DAY).unwrap().0.is_empty());
    assert_eq!(s.purge(T0 + 400 * DAY).unwrap(), 4);
    assert_eq!(s.purge(T0 + 400 * DAY).unwrap(), 0);
    // And back again: purged mail stays gone.
    assert!(s.fetch(b, 10, T0 + DAY).unwrap().0.is_empty());
    assert!(s.is_empty().unwrap());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn service_refuses_huge_acks() {
    let d = dir("svc");
    let store = Arc::new(Store::open(&d.join("m.redb"), Limits::default()).unwrap());
    let svc = MailboxService::new(store, Arc::new(ManualClock::new(T0)));
    let r = svc.handle(id(), MailboxRequest::Ack { ids: (0..10_001).collect() });
    assert!(matches!(r, MailboxResponse::Error(MailboxError::BadRequest(_))), "{r:?}");
    let r = svc.handle(id(), MailboxRequest::Fetch { limit: u32::MAX });
    assert!(matches!(r, MailboxResponse::Mail { .. }), "{r:?}");
    let r = svc.handle(id(), MailboxRequest::Fetch { limit: 0 });
    assert!(matches!(r, MailboxResponse::Mail { .. }), "{r:?}");
    std::fs::remove_dir_all(&d).unwrap();
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn config(data_dir: &Path, https: u16, http: u16, qad: u16) -> Config {
    let mut cfg = Config {
        hostname: "127.0.0.1".into(),
        data_dir: data_dir.to_owned(),
        http_bind: format!("127.0.0.1:{http}").parse().unwrap(),
        https_bind: format!("127.0.0.1:{https}").parse().unwrap(),
        qad_bind: format!("127.0.0.1:{qad}").parse().unwrap(),
        mailbox_port: 0,
        ..Config::default()
    };
    cfg.tls.cert_mode = CertMode::SelfSigned;
    cfg
}

async fn client(relay: &RelayNode) -> Result<(SecretKey, Endpoint)> {
    let key = SecretKey::generate();
    let ep = bind(key.clone(), &relay.client_config(), vec![]).await?;
    tokio::time::timeout(WAIT, ep.online()).await?;
    Ok((key, ep))
}

/// Broken frames on raw mailbox streams: the relay answers or drops each
/// stream and keeps serving the connection and everyone else.
#[tokio::test(flavor = "multi_thread")]
async fn hostile_frames_do_not_hurt_the_mailbox() -> Result<()> {
    let d = dir("frames");
    let relay = RelayNode::spawn(config(&d, 0, 0, 0), Arc::new(ManualClock::new(T0))).await?;
    relay.wait_online(WAIT).await?;
    let mailbox = relay.mailbox_id().unwrap();
    let (key, ep) = client(&relay).await?;
    let conn = ep.connect(dial_addr(mailbox, None), MAILBOX_ALPN).await?;
    let frames: Vec<Vec<u8>> = vec![
        u32::MAX.to_be_bytes().to_vec(),
        vec![0, 0, 0, 3, 9, 9, 9],
        vec![0, 0, 0, 10, 1, 2],
        vec![0, 0, 0, 0],
        vec![0, 0, 0, 5, 1, 0xff, 0xff, 0xff, 0xff],
        vec![0, 0],
        // A Put whose blob length says 4 GiB.
        vec![0, 0, 0, 40, 1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x0f],
    ];
    for f in frames {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&f).await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(1 << 20)).await;
    }
    // A stream opened and never written to does not block the others.
    let (_idle_send, _idle_recv) = conn.open_bi().await?;
    let mb = MailboxClient::connect(&ep, dial_addr(mailbox, None)).await?;
    let me = ep.id();
    open_own_mailbox(&mb, me).await?;
    tokio::time::timeout(WAIT, mb.put_sealed(&key, &key, me, b"still works")).await??;
    let (items, _) = tokio::time::timeout(WAIT, mb.fetch_opened(&key, 10)).await??;
    assert_eq!(items.len(), 1);
    ep.close().await;
    relay.shutdown().await?;
    std::fs::remove_dir_all(&d)?;
    Ok(())
}

/// The relay restarts (same ports, same data) while a client holds a
/// mailbox connection: the old connection fails promptly instead of
/// hanging, a new one works, and stored mail survived.
#[tokio::test(flavor = "multi_thread")]
async fn relay_restart_keeps_mail_and_old_connections_fail_fast() -> Result<()> {
    let d = dir("restart");
    let (https, http, qad) = (free_port(), free_port(), free_udp_port());
    let clock = Arc::new(ManualClock::new(T0));
    let relay = RelayNode::spawn(config(&d, https, http, qad), clock.clone()).await?;
    relay.wait_online(WAIT).await?;
    let mailbox = relay.mailbox_id().unwrap();
    let cfg = relay.client_config();
    let (key, ep) = client(&relay).await?;
    let me = ep.id();
    let mb = MailboxClient::connect(&ep, dial_addr(mailbox, None)).await?;
    open_own_mailbox(&mb, me).await?;
    let first = mb.put_sealed(&key, &key, me, b"before the restart").await?;

    relay.shutdown().await?;
    let relay = RelayNode::spawn(config(&d, https, http, qad), clock).await?;
    relay.wait_online(WAIT).await?;
    assert_eq!(relay.mailbox_id(), Some(mailbox));
    assert_eq!(relay.client_config().relays, cfg.relays);

    // The old connection is dead: an error within the timeout, no hang.
    let old = tokio::time::timeout(Duration::from_secs(60), mb.fetch(10)).await;
    assert!(matches!(old, Ok(Err(_))), "old connection: {old:?}");
    // The client's endpoint reconnects to the relay by itself.
    let mb = tokio::time::timeout(Duration::from_secs(60), MailboxClient::connect(&ep, dial_addr(mailbox, None))).await??;
    let (items, _) = mb.fetch_opened(&key, 10).await?;
    assert_eq!(items.iter().map(|(i, _)| i.id).collect::<Vec<_>>(), [first]);
    assert_eq!(items[0].1.as_ref().unwrap().payload, b"before the restart");
    // The registration survived the restart too: mail still gets in, and
    // a stranger's still does not.
    mb.put_sealed(&key, &key, me, b"after the restart").await?;
    let stranger = SecretKey::generate();
    assert!(matches!(mb.put_sealed(&key, &stranger, me, b"spam").await, Err(chummer_net::NetError::Mailbox(MailboxError::NotAllowed))));
    ep.close().await;
    relay.shutdown().await?;
    std::fs::remove_dir_all(&d)?;
    Ok(())
}

/// A damaged mailbox database used to stop the relay from starting at
/// all (a crash loop under systemd or Docker). Now it is moved aside and
/// an empty one is made; a permission problem is still an error.
#[test]
fn damaged_database_is_moved_aside_on_start() {
    let d = dir("recover");
    let p = d.join("mailbox.redb");
    {
        let s = Store::open(&p, Limits::default()).unwrap();
        put(&s, id(), id(), vec![1; 100], T0).unwrap();
    }
    let mut b = std::fs::read(&p).unwrap();
    for i in (0..b.len()).step_by(509) {
        b[i] ^= 0xa5;
    }
    std::fs::write(&p, &b).unwrap();
    assert!(Store::open(&p, Limits::default()).is_err(), "the damage is detected");
    let (s, moved) = Store::open_or_recover(&p, Limits::default(), T0).unwrap();
    let moved = moved.expect("moved aside");
    assert_eq!(std::fs::read(&moved).unwrap(), b, "the damaged file is kept");
    assert!(s.is_empty().unwrap());
    let bob = id();
    put(&s, id(), bob, vec![2], T0).unwrap();
    assert_eq!(s.fetch(bob, 10, T0).unwrap().0.len(), 1);
    drop(s);
    // A healthy database is opened as it is.
    let (s, moved) = Store::open_or_recover(&p, Limits::default(), T0).unwrap();
    assert!(moved.is_none());
    assert_eq!(s.len().unwrap(), 1);
    drop(s);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&p).is_err() {
            assert!(Store::open_or_recover(&p, Limits::default(), T0).is_err(), "an unreadable file is not moved aside");
            assert!(p.exists());
        }
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// Storage in memory whose writes can be made to fail like a full disk.
#[derive(Debug, Clone, Default)]
struct Flaky {
    data: Arc<std::sync::Mutex<Vec<u8>>>,
    full: Arc<std::sync::atomic::AtomicBool>,
}

impl Flaky {
    fn check(&self) -> std::io::Result<()> {
        if self.full.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(28)); // ENOSPC
        }
        Ok(())
    }
}

impl redb::StorageBackend for Flaky {
    fn len(&self) -> Result<u64, std::io::Error> {
        Ok(self.data.lock().unwrap().len() as u64)
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), std::io::Error> {
        let d = self.data.lock().unwrap();
        let o = offset as usize;
        out.copy_from_slice(&d[o..o + out.len()]);
        Ok(())
    }
    fn set_len(&self, len: u64) -> Result<(), std::io::Error> {
        let mut d = self.data.lock().unwrap();
        if len as usize > d.len() {
            self.check()?;
        }
        d.resize(len as usize, 0);
        Ok(())
    }
    fn sync_data(&self) -> Result<(), std::io::Error> {
        self.check()
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), std::io::Error> {
        self.check()?;
        let mut d = self.data.lock().unwrap();
        let o = offset as usize;
        d[o..o + data.len()].copy_from_slice(data);
        Ok(())
    }
}

/// After an I/O error (the disk filled up) redb refuses every later call
/// ("Previous I/O error occurred. Please close and re-open the
/// database"), so the mailbox stayed broken after the disk was freed,
/// until the relay was restarted (found by the Docker disk-full
/// scenario). The store now reopens the database.
#[test]
fn mailbox_works_again_after_the_disk_was_full() {
    let flaky = Flaky::default();
    let backend = flaky.clone();
    let opener: chummer_relay::store::Opener = Box::new(move || redb::Database::builder().create_with_backend(backend.clone()));
    let s = Store::open_with(opener, Limits::default()).unwrap();
    let bob = id();
    let first = put(&s, id(), bob, vec![1; 1000], T0).unwrap();
    flaky.full.store(true, std::sync::atomic::Ordering::SeqCst);
    for _ in 0..3 {
        assert!(matches!(put(&s, id(), bob, vec![2; 50_000], T0), Err(MailboxError::Internal(_))));
    }
    // The disk has room again.
    flaky.full.store(false, std::sync::atomic::Ordering::SeqCst);
    let second = put(&s, id(), bob, vec![3; 1000], T0).expect("works again without a restart");
    let (items, _) = s.fetch(bob, 10, T0).unwrap();
    assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [first, second]);
    assert_eq!(s.ack(bob, &[first, second]).unwrap(), 2);
}
