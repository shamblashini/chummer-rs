//! Online campaign sync for chummer-rs.
//!
//! The GM's app is the authority for a campaign; players keep local copies
//! of their own characters and send commands. See `docs/online-design.md`.
//!
//! - [`msg`]: the versioned messages inside chummer-net's opaque payloads.
//! - [`authority`]: [`Authority`], the GM's state of every character:
//!   applies and rebases submissions, de-duplicates by operation id, keeps
//!   the log window, the activity feed and who was sent what. Synchronous
//!   and transport-agnostic.
//! - [`replica`]: [`Replica`], a player's copies with an offline outbox,
//!   applied optimistically and rebuilt on every answer; detects drift by
//!   hash and resyncs from a snapshot. Saved to disk.
//! - [`mail`]: play-by-post through the relay mailbox: splitting into
//!   blobs, sealing, reassembly.
//! - [`host`]: [`AuthorityHost`], the authority on chummer-net (campaign
//!   protocol handler, live pushes, mailbox rounds).
//! - [`player`]: [`PlayerSession`], a player's connection with mailbox
//!   fallback.
//! - [`node`]: [`Node`], an app's one endpoint: dials out for player
//!   sessions and serves the hosted campaign.
//! - [`hosted`]: [`HostedCampaign`], a GM's `.chummercampaign` file with
//!   its authority sidecar (the glue the GUI and `chummer-authority` share).
//! - [`joined`]: the campaigns a player has joined, as the app lists them.
//! - [`journal`]: the authority's journal of acknowledged changes, so a
//!   crash between saves loses none of them.

pub mod authority;
pub mod feed;
pub mod host;
pub mod hosted;
pub mod invites;
pub mod joined;
pub mod journal;
pub mod lockwatch;
pub mod mail;
pub mod msg;
pub mod node;
pub mod persist;
pub mod player;
pub mod replica;

pub use authority::{Authority, LocalApplied, Member, Reverted, Submitted};
pub use host::{AuthorityHost, HostEvent, HostNotice, MailReport};
pub use hosted::HostedCampaign;
pub use node::Node;
pub use msg::{CharacterId, FeedEntry};
pub use player::{PlayerConfig, PlayerSession, SyncMode};
pub use replica::{Event, Refused, Replica};

// The live handshake compares the campaign protocol version, so it must
// change whenever the sync format does: then an app with other sync
// messages is refused at the handshake, with a message saying whose app
// is older, instead of failing on every message after it. They are kept
// equal; when you change one, change the other to the same number.
const _: () = assert!(
    chummer_net::campaign::PROTOCOL_VERSION == msg::SYNC_VERSION as u32,
    "msg::SYNC_VERSION and chummer_net::campaign::PROTOCOL_VERSION differ: bump both together (to the same number)"
);
const _: () = assert!(msg::SYNC_VERSION != msg::NOTICE_BYTE, "sync version 0 is reserved for version notices");

#[cfg(test)]
mod tests {
    fn send_sync<T: Send + Sync>() {}

    /// The handshake's protocol version follows the sync format (also
    /// checked at compile time, above): a sync change is a handshake
    /// change, and peers that differ are told so when they connect.
    #[test]
    fn protocol_version_follows_the_sync_format() {
        assert_eq!(chummer_net::campaign::PROTOCOL_VERSION, crate::msg::SYNC_VERSION as u32);
        const { assert!(chummer_net::campaign::FIRST_WITH_APP_VERSION <= chummer_net::campaign::PROTOCOL_VERSION) };
    }

    /// The version notice is readable whatever the sync version, and is
    /// not a sync message.
    #[test]
    fn version_notice_round_trips() {
        use crate::msg::{self, VersionNotice};
        let n = VersionNotice::mine();
        let bytes = n.encode();
        assert_eq!(bytes[0], 0);
        assert_eq!(VersionNotice::decode(&bytes), Some(n.clone()));
        assert!(matches!(msg::decode::<msg::MailMessage>(&bytes), Err(msg::DecodeError::Version(0))));
        assert_eq!(VersionNotice::decode(&msg::encode(&1u8)), None);
        // A later app's notice, with a field this one does not know.
        let mut later = vec![0u8];
        later.extend_from_slice(br#"{"app_version":"9.0.0","sync_version":9,"protocol_version":9,"os":"x"}"#);
        let n = VersionNotice::decode(&later).unwrap();
        assert_eq!((n.app_version.as_str(), n.sync_version), ("9.0.0", 9));
        let mine = chummer_net::campaign::APP_VERSION;
        assert_eq!(n.refusal().to_string(), format!("Your chummer-rs is older than the GM's (version 9.0.0; yours is {mine}). Update chummer-rs."));
    }

    #[test]
    fn shared_state_is_thread_safe() {
        send_sync::<chummer_core::character::Character>();
        send_sync::<chummer_core::engine::Engine>();
        send_sync::<crate::Authority>();
        send_sync::<crate::Replica>();
    }
}
