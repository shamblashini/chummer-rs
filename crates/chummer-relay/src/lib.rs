//! chummer-rs relay: an iroh relay server plus the campaign mailbox.
//!
//! - The relay ([`iroh_relay::server`]) connects peers that cannot reach
//!   each other directly and helps them hole-punch. It only forwards
//!   end-to-end encrypted QUIC packets.
//! - The mailbox is an iroh endpoint next to it that serves
//!   `chummer-rs/mailbox/3` ([`chummer_net::mailbox`]): sealed blobs for
//!   offline peers, kept in a redb database with size, count, quota and
//!   expiry limits ([`store::Limits`]).
//!
//! See `docs/relay.md` for deployment.

pub mod config;
pub mod log;
pub mod server;
pub mod service;
pub mod store;

pub use config::{CertMode, Config};
pub use server::RelayNode;
pub use service::{Clock, ManualClock, SystemClock};
