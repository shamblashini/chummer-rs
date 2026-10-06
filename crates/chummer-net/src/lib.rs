//! Networking for chummer-rs online campaigns.
//!
//! This crate knows nothing about characters. Every payload is opaque bytes;
//! the sync layer (commands, versions, snapshots) defines what is inside.
//! See `docs/online-design.md` (sections 4, 6 and 7) and `docs/relay.md`.
//!
//! Parts:
//! - [`identity`]: the persistent node key (`node.key` in the config dir).
//!   Its public half, the iroh [`EndpointId`], is the user's identity.
//! - [`config`] and [`node`]: relay list and endpoint setup. Peers are found
//!   by [`EndpointId`] through the configured relays only
//!   ([`node::RelayLookup`]); no third-party DNS or pkarr servers.
//! - [`campaign`]: the `chummer-rs/campaign/1` protocol between a player's
//!   app and the GM's app (hello, submit/ack, server push, ping).
//! - [`invite`]: campaign ids, invite tokens and `chummer-rs://join/...` links.
//! - [`mailbox`]: the `chummer-rs/mailbox/1` protocol to the relay's
//!   store-and-forward mailbox, and its client.
//! - [`seal`]: end-to-end sealing of mailbox blobs to a recipient's
//!   [`EndpointId`], signed by the sender.
//! - [`frame`]: length-prefixed postcard framing used by all protocols.

pub mod campaign;
pub mod config;
pub mod frame;
pub mod identity;
pub mod invite;
pub mod mailbox;
pub mod node;
pub mod seal;

mod error;
mod hex;

pub use error::NetError;
pub use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey};
