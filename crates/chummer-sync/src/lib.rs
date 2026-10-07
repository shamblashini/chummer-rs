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
pub use host::{AuthorityHost, HostEvent, MailReport};
pub use hosted::HostedCampaign;
pub use node::Node;
pub use msg::{CharacterId, FeedEntry};
pub use player::{PlayerConfig, PlayerSession, SyncMode};
pub use replica::{Event, Refused, Replica};

#[cfg(test)]
mod tests {
    fn send_sync<T: Send + Sync>() {}

    #[test]
    fn shared_state_is_thread_safe() {
        send_sync::<chummer_core::character::Character>();
        send_sync::<chummer_core::engine::Engine>();
        send_sync::<crate::Authority>();
        send_sync::<crate::Replica>();
    }
}
