//! A campaign shares the settings presets its characters use: the
//! player's app resolves them as the GM's app does, apart from the
//! player's own presets.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use chummer_core::character::Character;
use chummer_core::command::Command;
use chummer_core::engine::Engine;
use chummer_core::settings::{self, STANDARD_ID};
use chummer_net::invite::{CampaignId, InviteLink, Role};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::{bind, dial_addr};
use chummer_net::{Endpoint, EndpointId, SecretKey};
use chummer_relay::{CertMode, Config, ManualClock, RelayNode};
use chummer_sync::msg::{ClientMessage, ServerMessage};
use chummer_sync::{Authority, AuthorityHost, CharacterId, Event, PlayerConfig, PlayerSession, Replica, SyncMode};
use iroh::protocol::Router;

const T0: u64 = 1_800_000_000;
const WAIT: Duration = Duration::from_secs(30);
const HOUSE: &str = "House.xml";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chummer-sync-presets-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Writes `House.xml` into `dir`: Standard with attributes costing
/// `karma` per point.
fn write_house(dir: &Path, karma: i32) {
    static STANDARD: OnceLock<chummer_core::xml::Element> = OnceLock::new();
    let standard = STANDARD.get_or_init(|| Engine::load().expect("game data").settings().find(STANDARD_ID).unwrap().raw.clone());
    let mut raw = standard.clone();
    raw.set_child_text("name", "House");
    raw.child_or_insert("karmacost").set_child_text("karmaattribute", karma.to_string());
    std::fs::write(dir.join(HOUSE), settings::export_string(&raw)).unwrap();
}

/// An attribute Munin can raise.
fn raisable(engine: &Engine, ch: &Character) -> (String, i32) {
    ["BOD", "AGI", "REA", "STR", "CHA", "INT", "LOG", "WIL"]
        .iter()
        .find_map(|a| chummer_core::career::karma::attribute_upgrade_karma_cost(engine, ch, a).map(|k| (a.to_string(), k)))
        .expect("an attribute to raise")
}

/// An engine reading user presets from `dir` only.
fn engine_in(dir: &Path) -> Arc<Engine> {
    Arc::new(Engine::load().expect("game data").with_settings_dir(Some(dir.to_owned())).unwrap())
}

fn munin(settings: &str) -> Character {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin_Career.chum5");
    let mut ch = Character::load(&p).unwrap();
    ch.karma = 200;
    ch.set_field("settings", settings);
    ch
}

fn attribute_karma(engine: &Engine, ch: &Character) -> i32 {
    engine.settings().resolve_for(ch).unwrap().karma("karmaattribute", 5)
}

fn join(auth: &mut Authority, peer: EndpointId, r: &mut Replica, engine: &Engine) -> Vec<Event> {
    let ClientMessage::Join { have, builtins, .. } = r.join_message(engine, "Alice", None) else { unreachable!() };
    auth.note_builtins(&peer, builtins);
    let (membership, pushes) = auth.join(peer, "Alice", &have).unwrap();
    r.handle(engine, ServerMessage::Joined { membership, pushes })
}

/// What the authority has for `peer`, handled by the replica (as a
/// mailbox round or a live sweep delivers it).
fn deliver(auth: &mut Authority, peer: EndpointId, r: &mut Replica, engine: &Engine) -> Vec<Event> {
    let mut ev = Vec::new();
    for m in auth.outgoing_for(&peer) {
        auth.mark_sent(peer, &m);
        ev.extend(r.handle(engine, m));
    }
    ev
}

#[test]
fn a_member_gets_the_presets_of_their_characters() {
    let (gm_dir, p_dir) = (tmp("gm"), tmp("player"));
    write_house(&gm_dir, 7);
    // The player has a preset of their own under the same file name.
    write_house(&p_dir, 4);
    let own_before = std::fs::read(p_dir.join(HOUSE)).unwrap();
    let (gm_engine, p_engine) = (engine_in(&gm_dir), engine_in(&p_dir));

    let (gm, p) = (SecretKey::generate().public(), SecretKey::generate().public());
    let mut auth = Authority::new(CampaignId::random(), gm, "GM");
    auth.set_name("Seattle Nights");
    auth.set_presets(gm_engine.settings());
    auth.add_member(p, Role::Player, "Alice");
    let (c, std_char, npc) = (CharacterId::new("house"), CharacterId::new("standard"), CharacterId::new("npc"));
    auth.add_character(c.clone(), Some(p), munin(HOUSE)).unwrap();
    auth.add_character(std_char.clone(), Some(p), munin(STANDARD_ID)).unwrap();
    // The GM's own character's preset is not the player's business.
    auth.add_character(npc, None, munin("Street-Level.xml")).unwrap();

    let mut r = Replica::new();
    let ev = join(&mut auth, p, &mut r, &p_engine);
    assert!(ev.contains(&Event::Presets), "{ev:?}");
    let m = r.membership().unwrap().clone();
    assert_eq!(m.presets.len(), 2, "{:?}", m.presets.iter().map(|p| &p.key).collect::<Vec<_>>());
    let house = m.presets.iter().find(|p| p.key == HOUSE).unwrap();
    assert!(house.xml.as_ref().is_some_and(|x| x.len() < 64 * 1024), "the GM's preset travels with its XML, a few KiB");
    let standard = m.presets.iter().find(|p| p.key == STANDARD_ID).unwrap();
    assert!(standard.xml.is_none(), "a built-in the player has with the same content is not sent");

    // The character resolves against the campaign's copy.
    let scope = r.presets_scope().unwrap();
    let ch = r.character(&c).unwrap().clone();
    assert_eq!(ch.settings_scope.as_deref(), Some(scope.as_str()));
    let lib = p_engine.settings();
    assert_eq!(lib.missing_for(&ch), None, "no Cannot Find Settings File");
    let found = lib.find_for(&ch).unwrap();
    let src = found.campaign.as_ref().expect("the campaign's copy");
    assert_eq!(src.campaign, "Seattle Nights");
    assert_eq!(found.karma("karmaattribute", 5), 7);
    assert_eq!(attribute_karma(&p_engine, &ch), 7);
    assert!(!found.is_user(), "read-only for the player");
    assert_eq!(lib.find_for(r.character(&std_char).unwrap()).unwrap().key(), STANDARD_ID);
    assert!(lib.find_for(r.character(&std_char).unwrap()).unwrap().is_builtin());
    // The player's own preset of the same name is untouched, and the
    // player's other characters keep using it.
    let own = lib.find(HOUSE).unwrap();
    assert!(own.is_user());
    assert_eq!(own.karma("karmaattribute", 5), 4);
    assert_eq!(std::fs::read(p_dir.join(HOUSE)).unwrap(), own_before);
    assert_eq!(std::fs::read_dir(&p_dir).unwrap().count(), 1, "nothing written into the player's settings folder");
    let mut offline = ch.clone();
    offline.settings_scope = None;
    assert_eq!(attribute_karma(&p_engine, &offline), 4);

    // Edits cost what they cost at the GM's, so the hashes agree.
    let (attr, cost) = raisable(&p_engine, &ch);
    assert_eq!(Some(cost), chummer_core::career::karma::attribute_upgrade_karma_cost(&gm_engine, auth.character(&c).unwrap(), &attr), "the GM's price");
    assert_ne!(Some(cost), chummer_core::career::karma::attribute_upgrade_karma_cost(&p_engine, &offline, &attr), "not the player's own price");
    let before = r.character(&c).unwrap().karma;
    r.edit(&p_engine, &c, Command::RaiseAttribute { attribute: attr }).unwrap();
    assert_eq!(before - r.character(&c).unwrap().karma, cost);
    let s = auth.submit(&gm_engine, p, r.batch(&c).unwrap()).unwrap();
    let ev = r.handle(&p_engine, ServerMessage::Ack(s.ack));
    assert!(!ev.iter().any(|e| matches!(e, Event::NeedResync(_))), "{ev:?}");
    assert_eq!(r.confirmed_hash(&c), auth.hash(&c));
    let (attr, _) = raisable(&gm_engine, auth.character(&c).unwrap());
    auth.apply_local(&gm_engine, &c, Command::RaiseAttribute { attribute: attr }).unwrap();
    let ev = deliver(&mut auth, p, &mut r, &p_engine);
    assert!(ev.contains(&Event::Updated(c.clone())) && !ev.iter().any(|e| matches!(e, Event::NeedResync(_))), "{ev:?}");
    assert_eq!(r.confirmed_hash(&c), auth.hash(&c));

    // Nothing to send while nothing changed.
    assert!(!auth.membership_stale(&p));

    // The GM edits and saves the preset: the member gets the new version.
    write_house(&gm_dir, 9);
    gm_engine.reload_settings().unwrap();
    auth.set_presets(gm_engine.settings());
    assert!(auth.membership_stale(&p));
    let ev = deliver(&mut auth, p, &mut r, &p_engine);
    assert!(ev.contains(&Event::Presets), "{ev:?}");
    assert_eq!(attribute_karma(&p_engine, r.character(&c).unwrap()), 9);
    assert_eq!(p_engine.settings().find(HOUSE).unwrap().karma("karmaattribute", 5), 4, "the player's own is still theirs");
    assert!(!auth.membership_stale(&p));

    // The GM switches a character to another preset: the membership follows.
    auth.apply_local(&gm_engine, &std_char, Command::SwitchSettings { key: HOUSE.into() }).unwrap();
    let ev = deliver(&mut auth, p, &mut r, &p_engine);
    assert!(!ev.iter().any(|e| matches!(e, Event::NeedResync(_))), "{ev:?}");
    assert_eq!(r.membership().unwrap().presets.len(), 1, "Standard is no longer used");
    assert_eq!(r.confirmed_hash(&std_char), auth.hash(&std_char));

    // Saved and loaded again, the replica puts its presets back.
    let bytes = r.to_bytes();
    assert!(p_engine.remove_campaign_presets(&scope));
    let r2 = Replica::from_bytes(&p_engine, &bytes).unwrap();
    assert_eq!(attribute_karma(&p_engine, r2.character(&c).unwrap()), 9);
    // The authority's file keeps what each member was sent.
    let mut back = Authority::from_bytes(&auth.to_bytes()).unwrap();
    back.set_presets(gm_engine.settings());
    assert!(!back.membership_stale(&p));

    // Leaving the campaign takes them out; the player's own stays.
    assert!(r.forget_presets(&p_engine));
    let lib = p_engine.settings();
    assert_eq!(lib.campaign_presets(&scope).count(), 0);
    assert_eq!(lib.missing_for(r.character(&c).unwrap()).as_deref(), Some("House"));
    assert_eq!(lib.find(HOUSE).unwrap().karma("karmaattribute", 5), 4);
    assert_eq!(std::fs::read(p_dir.join(HOUSE)).unwrap(), own_before);

    // Rejoining brings them back; the GM taking the characters away (or
    // removing the player) removes them again.
    join(&mut auth, p, &mut r, &p_engine);
    assert_eq!(p_engine.settings().campaign_presets(&scope).count(), 1);
    auth.set_owner(&c, None);
    auth.set_owner(&std_char, None);
    let ev = deliver(&mut auth, p, &mut r, &p_engine);
    assert!(ev.contains(&Event::Presets), "{ev:?}");
    assert_eq!(p_engine.settings().campaign_presets(&scope).count(), 0);
    let ev = r.handle(&p_engine, ServerMessage::Denied(chummer_net::campaign::DenyReason::Revoked));
    assert!(!ev.contains(&Event::Presets), "nothing left to remove");
}

/// Two players of one campaign on one engine (tests do that) keep their
/// presets apart.
#[test]
fn presets_are_scoped_per_member() {
    let gm_dir = tmp("gm-two");
    write_house(&gm_dir, 6);
    let gm_engine = engine_in(&gm_dir);
    let p_engine = engine_in(&tmp("player-two"));
    let (gm, p1, p2) = (SecretKey::generate().public(), SecretKey::generate().public(), SecretKey::generate().public());
    let mut auth = Authority::new(CampaignId::random(), gm, "GM");
    auth.set_presets(gm_engine.settings());
    auth.add_member(p1, Role::Player, "Alice");
    auth.add_member(p2, Role::Player, "Bob");
    auth.add_character(CharacterId::new("a"), Some(p1), munin(HOUSE)).unwrap();
    auth.add_character(CharacterId::new("b"), Some(p2), munin(STANDARD_ID)).unwrap();
    let (mut r1, mut r2) = (Replica::new(), Replica::new());
    join(&mut auth, p1, &mut r1, &p_engine);
    join(&mut auth, p2, &mut r2, &p_engine);
    let lib = p_engine.settings();
    assert_eq!(lib.campaign_presets(&r1.presets_scope().unwrap()).count(), 1);
    assert_eq!(lib.campaign_presets(&r2.presets_scope().unwrap()).count(), 0);
    assert_eq!(attribute_karma(&p_engine, r1.character(&CharacterId::new("a")).unwrap()), 6);
}

// ----- over the network -----

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

fn campaign_karma(s: &PlayerSession, c: &CharacterId) -> i32 {
    let ch = s.replica().character(c).unwrap().clone();
    attribute_karma(s.engine(), &ch)
}

/// By mail first (the GM's app serves nothing), then live: the preset
/// arrives, and the GM's edits to it follow both ways.
#[tokio::test(flavor = "multi_thread")]
async fn presets_by_mail_and_live() -> Result<()> {
    let relay = relay("presets").await?;
    let mailbox_id = relay.mailbox_id().expect("mailbox");
    let (gm_dir, p_dir) = (tmp("net-gm"), tmp("net-player"));
    write_house(&gm_dir, 7);
    write_house(&p_dir, 4);
    let (gm_engine, p_engine) = (engine_in(&gm_dir), engine_in(&p_dir));

    let gm_key = SecretKey::generate();
    let gm_ep = endpoint(&relay, gm_key.clone()).await?;
    let p_key = SecretKey::generate();
    let p_ep = endpoint(&relay, p_key.clone()).await?;
    let mut auth = Authority::new(CampaignId::random(), gm_ep.id(), "GM");
    auth.set_name("Seattle Nights");
    auth.add_member(p_ep.id(), Role::Player, "Alice");
    let c = CharacterId::new("alice");
    auth.add_character(c.clone(), Some(p_ep.id()), munin(HOUSE))?;
    let host = AuthorityHost::new(auth, gm_engine.clone(), gm_key, Some(tmp("net-gm-file").join("campaign.authority")));
    let gm_mail = MailboxClient::connect(&gm_ep, dial_addr(mailbox_id, None)).await?;
    host.sync_mail(&gm_mail).await?;

    let (campaign, gm_pub) = {
        let a = host.authority();
        (a.campaign(), a.gm_keys()[0])
    };
    let link = InviteLink { host: gm_ep.id(), campaign, member: None, gm_key: Some(gm_pub), relay: None };
    let mut cfg = PlayerConfig::new("Alice", link);
    cfg.mailbox = Some(mailbox_id);
    cfg.path = Some(tmp("net-player-file").join("campaign.replica"));
    cfg.connect_timeout = Duration::from_secs(3);
    let player = PlayerSession::new(p_ep.clone(), p_key, p_engine.clone(), cfg)?;

    // By mail.
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    host.sync_mail(&gm_mail).await?;
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    assert_eq!(campaign_karma(&player, &c), 7, "the GM's preset came by mail");
    assert_eq!(player.replica().confirmed_hash(&c), host.authority().hash(&c));
    assert_eq!(p_engine.settings().find(HOUSE).unwrap().karma("karmaattribute", 5), 4);

    // The GM saves a change to the preset; the next mailbox round has it.
    write_house(&gm_dir, 8);
    gm_engine.reload_settings().unwrap();
    host.settings_changed();
    let r = host.sync_mail(&gm_mail).await?;
    assert!(r.sent >= 1, "{r:?}");
    assert_eq!(player.sync().await, SyncMode::Mailbox);
    assert_eq!(campaign_karma(&player, &c), 8, "the edit came by mail");

    // Live.
    let router = Router::builder(gm_ep.clone()).accept(chummer_net::campaign::CAMPAIGN_ALPN, host.protocol()).spawn();
    assert_eq!(player.connect().await?, Role::Player);
    player.events();
    write_house(&gm_dir, 9);
    gm_engine.reload_settings().unwrap();
    host.settings_changed();
    wait_for(&player, |e| *e == Event::Presets).await?;
    assert_eq!(campaign_karma(&player, &c), 9, "the edit came live");
    let own = p_engine.settings().find(HOUSE).unwrap().clone();
    assert!(own.is_user() && own.karma("karmaattribute", 5) == 4, "the player's own preset is untouched");

    // Leaving takes them out.
    assert!(player.forget_presets());
    assert_eq!(p_engine.settings().missing_for(player.replica().character(&c).unwrap()).as_deref(), Some("House"));

    player.disconnect();
    gm_mail.close();
    router.shutdown().await?;
    for ep in [gm_ep, p_ep] {
        ep.close().await;
    }
    relay.shutdown().await
}
