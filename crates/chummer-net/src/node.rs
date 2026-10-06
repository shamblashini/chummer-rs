//! Endpoint setup and peer lookup.
//!
//! # Discovery choice
//!
//! Peers are dialled by [`EndpointId`] alone. To turn an id into an address
//! iroh asks its address-lookup services; the stock ones (pkarr publishing
//! plus DNS, `presets::N0`) depend on number 0's `dns.iroh.link` servers, and
//! the iroh relay server does not host a pkarr endpoint itself. We do not
//! want third-party infrastructure beyond our relay, so we never use them.
//!
//! Instead [`RelayLookup`] answers every lookup with "try this id on each
//! configured relay". A relay forwards packets to any endpoint connected to
//! it, so as long as the peer's home relay is one of ours (the default for
//! everyone using the project relay) the first packets go through the relay
//! and iroh then hole-punches a direct path. Invite links may carry a
//! `relay=` hint for GMs on a private relay; [`dial_addr`] adds it.
//!
//! Nothing is published, so there is nothing for an outsider to enumerate.
//! The cost is that a peer whose home relay is in neither list cannot be
//! found; a relay-side directory can be added later without changing the
//! protocols.

use std::sync::Arc;

use iroh::address_lookup::{self, AddressLookup, EndpointInfo, Item};
use iroh::endpoint::{presets, Builder, RelayMode};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey};
use iroh_relay::tls::CaTlsConfig;
use iroh_relay::RelayMap;
use n0_future::{boxed::BoxStream, stream, StreamExt};

use crate::config::NetConfig;
use crate::NetError;

/// Answers lookups with the configured relay URLs.
#[derive(Debug, Clone)]
pub struct RelayLookup {
    relays: Arc<Vec<RelayUrl>>,
}

impl RelayLookup {
    pub const PROVENANCE: &'static str = "chummer-relay-list";

    pub fn new(relays: impl IntoIterator<Item = RelayUrl>) -> Self {
        RelayLookup {
            relays: Arc::new(relays.into_iter().collect()),
        }
    }
}

impl AddressLookup for RelayLookup {
    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<BoxStream<Result<Item, address_lookup::Error>>> {
        if self.relays.is_empty() {
            return None;
        }
        let mut info = EndpointInfo::new(endpoint_id);
        for url in self.relays.iter() {
            info = info.with_relay_url(url.clone());
        }
        let item = Item::new(info, Self::PROVENANCE, None);
        Some(stream::iter(Some(Ok(item))).boxed())
    }
}

/// The relay map for `cfg`.
pub fn relay_map(cfg: &NetConfig) -> RelayMap {
    cfg.relays.iter().map(|r| r.iroh_config()).collect()
}

/// The TLS roots used to check relay certificates.
pub fn ca_tls_config(cfg: &NetConfig) -> CaTlsConfig {
    CaTlsConfig::default().with_extra_roots(
        cfg.extra_ca_roots
            .iter()
            .map(|der| rustls::pki_types::CertificateDer::from(der.clone())),
    )
}

/// An endpoint builder set up for chummer-rs: our relays only, our lookup
/// only, no third-party services. Add ALPNs and call `bind()`.
pub fn builder(secret: SecretKey, cfg: &NetConfig) -> Result<Builder, NetError> {
    let urls = cfg.relays.iter().map(|r| r.url.clone());
    let mut b = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .relay_mode(if cfg.relays.is_empty() {
            RelayMode::Disabled
        } else {
            RelayMode::Custom(relay_map(cfg))
        })
        .ca_tls_config(ca_tls_config(cfg))
        .address_lookup(RelayLookup::new(urls));
    if let Some(port) = cfg.port {
        let v6 = iroh::endpoint::BindOpts::default().set_is_required(false);
        b = b
            .clear_ip_transports()
            .bind_addr((std::net::Ipv4Addr::UNSPECIFIED, port))
            .map_err(|e| NetError::Bind(e.to_string()))?
            .bind_addr_with_opts((std::net::Ipv6Addr::UNSPECIFIED, port), v6)
            .map_err(|e| NetError::Bind(e.to_string()))?;
    }
    Ok(b)
}

/// Binds an endpoint that accepts `alpns`.
pub async fn bind(
    secret: SecretKey,
    cfg: &NetConfig,
    alpns: Vec<Vec<u8>>,
) -> Result<Endpoint, NetError> {
    builder(secret, cfg)?
        .alpns(alpns)
        .bind()
        .await
        .map_err(|e| NetError::Bind(e.to_string()))
}

/// The address to dial for `id`, with an optional relay hint (from an
/// invite link). The configured relays are added by [`RelayLookup`].
pub fn dial_addr(id: EndpointId, relay_hint: Option<&RelayUrl>) -> EndpointAddr {
    let addr = EndpointAddr::new(id);
    match relay_hint {
        Some(url) => addr.with_relay_url(url.clone()),
        None => addr,
    }
}
