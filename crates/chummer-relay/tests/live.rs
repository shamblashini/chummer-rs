//! Checks a deployed relay over the internet: two nodes come online on it,
//! one leaves the other mail in its mailbox, and the other collects it.
//! Ignored by default; run with the relay entry:
//!
//! ```text
//! CHUMMER_LIVE_RELAY='https://relay.example.org#<mailbox node id>' \
//!   cargo test -p chummer-relay --test live -- --ignored
//! ```

use std::time::Duration;

use anyhow::{Context, Result};
use chummer_net::config::{NetConfig, RelayEntry};
use chummer_net::mailbox::MailboxClient;
use chummer_net::node::{bind, dial_addr};
use chummer_net::SecretKey;

const WAIT: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a deployed relay (CHUMMER_LIVE_RELAY)"]
async fn a_deployed_relay_carries_mail() -> Result<()> {
    let entry: RelayEntry = std::env::var("CHUMMER_LIVE_RELAY")
        .context("set CHUMMER_LIVE_RELAY to the relay entry")?
        .parse()?;
    let mailbox = entry.mailbox.context("the entry needs #<mailbox node id>")?;
    let cfg = NetConfig::with_relays([entry]);

    let alice_key = SecretKey::generate();
    let bob_key = SecretKey::generate();
    let alice_ep = bind(alice_key.clone(), &cfg, vec![]).await?;
    let bob_ep = bind(bob_key.clone(), &cfg, vec![]).await?;
    tokio::time::timeout(WAIT, alice_ep.online()).await.context("alice did not reach the relay")?;
    tokio::time::timeout(WAIT, bob_ep.online()).await.context("bob did not reach the relay")?;

    let alice = tokio::time::timeout(WAIT, MailboxClient::connect(&alice_ep, dial_addr(mailbox, None))).await.context("alice: mailbox timed out")??;
    let bob = tokio::time::timeout(WAIT, MailboxClient::connect(&bob_ep, dial_addr(mailbox, None))).await.context("bob: mailbox timed out")??;
    let cap = SecretKey::generate();
    bob.register(rand_scope(), vec![cap.public()]).await?;
    let id = alice.put_sealed(&alice_key, &cap, bob_ep.id(), b"live relay check").await?;
    let (mail, _) = bob.fetch_opened(&bob_key, 100).await?;
    let got = mail.iter().find(|(m, _)| m.id == id).context("the mail did not arrive")?;
    assert_eq!(got.1.as_ref().expect("opens").payload, b"live relay check");
    assert_eq!(bob.ack(vec![id]).await?, 1);
    println!("relayed and mailed through {}", cfg.relays[0].url);

    for c in [&alice, &bob] {
        c.close();
    }
    alice_ep.close().await;
    bob_ep.close().await;
    Ok(())
}

fn rand_scope() -> [u8; 16] {
    SecretKey::generate().public().as_bytes()[..16].try_into().unwrap()
}
