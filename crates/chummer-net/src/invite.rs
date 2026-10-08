//! Campaign ids, member keys and invite links.
//!
//! An invite link is
//! `chummer-rs://join/<gm-node-id>?campaign=<id>&member=<secret>&gm=<key>`,
//! with an optional `&relay=<url>` hint when the GM uses a relay that is not
//! in the player's default list.
//!
//! - `campaign`: 128 random bits, 32 lower-case hex characters.
//! - `member`: the invite's member key, an ed25519 secret (64 hex
//!   characters). Each invite has its own; possessing it is the permission
//!   to join (live: the campaign protocol's hello proves it; by mail: the
//!   mailed join carries a proof) and to put mail into the GM's relay
//!   mailbox. The first node that joins with it claims the invite; after
//!   that it only admits that node.
//! - `gm`: the public half of the GM's campaign key
//!   ([`derive_campaign_key`]). The player's app registers it with its own
//!   relay mailbox, so the GM can mail it before the first live contact.
//!
//! Links without `member` are for members the GM added by node id; they
//! prove themselves by their node key.

use std::fmt;
use std::str::FromStr;

use iroh::{EndpointId, PublicKey, RelayUrl, SecretKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// URL scheme of invite links.
pub const SCHEME: &str = "chummer-rs";

/// 128 bits from the OS random source (for ids that must not collide
/// between machines, such as the sync layer's operation ids).
pub fn random_id() -> [u8; 16] {
    random16()
}

fn random16() -> [u8; 16] {
    use crypto_box::aead::rand_core::RngCore;
    let mut b = [0u8; 16];
    crypto_box::aead::OsRng.fill_bytes(&mut b);
    b
}

macro_rules! id16 {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; 16]);

        impl $name {
            /// A new random value (128 bits from the OS random source).
            pub fn random() -> Self {
                Self(random16())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&crate::hex::encode(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self)
            }
        }

        impl FromStr for $name {
            type Err = InviteError;
            fn from_str(s: &str) -> Result<Self, InviteError> {
                crate::hex::decode::<16>(s)
                    .map(Self)
                    .ok_or(InviteError::BadId(stringify!($name)))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s: std::borrow::Cow<'de, str> = Deserialize::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

id16!(
    /// Identifies a campaign on its host.
    CampaignId
);
id16!(
    /// Identifies one invite (a member slot) in a campaign. It stays the
    /// same when the GM issues the member a new link.
    InviteId
);

/// What a member of a campaign may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Sees and edits every character in the campaign.
    Gm,
    /// Sees and edits only their own characters.
    Player,
}

/// The secret half of an invite's member key, as carried in the link.
/// Its `Debug` form does not show the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct MemberSecret(pub [u8; 32]);

impl MemberSecret {
    /// A new random member key.
    pub fn random() -> MemberSecret {
        MemberSecret(SecretKey::generate().to_bytes())
    }

    pub fn key(&self) -> SecretKey {
        SecretKey::from_bytes(&self.0)
    }

    pub fn public(&self) -> PublicKey {
        self.key().public()
    }
}

impl fmt::Debug for MemberSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemberSecret(public {})", self.public().fmt_short())
    }
}

impl fmt::Display for MemberSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::hex::encode(&self.0))
    }
}

impl FromStr for MemberSecret {
    type Err = InviteError;
    fn from_str(s: &str) -> Result<Self, InviteError> {
        crate::hex::decode::<32>(s.trim()).map(MemberSecret).ok_or(InviteError::BadId("member key"))
    }
}

impl Serialize for MemberSecret {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MemberSecret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s: std::borrow::Cow<'de, str> = Deserialize::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The GM's campaign key of `generation`: what the GM's app signs mailbox
/// puts to players with. Derived from the GM's node key (BLAKE3
/// `derive_key`), so it needs no storage and `chummer-authority` can make
/// links without the running host; a new generation is a new key
/// (rotation).
pub fn derive_campaign_key(gm: &SecretKey, campaign: CampaignId, generation: u32) -> SecretKey {
    let mut material = Vec::with_capacity(32 + 16 + 4);
    material.extend_from_slice(&gm.to_bytes());
    material.extend_from_slice(&campaign.0);
    material.extend_from_slice(&generation.to_be_bytes());
    SecretKey::from_bytes(&blake3::derive_key("chummer-rs campaign key v1", &material))
}

const CLAIM_DOMAIN: &[u8] = b"chummer-rs/mail-claim/2\0";

/// What a member key signs to join by mail (when the GM is offline): the
/// campaign, the GM's node and the joining node. The mail is sealed to
/// the GM and signed by the joining node, so only the GM reads the proof
/// and it is no good to another node.
pub fn claim_message(campaign: &CampaignId, host: &EndpointId, node: &EndpointId) -> Vec<u8> {
    let mut m = Vec::with_capacity(CLAIM_DOMAIN.len() + 16 + 64);
    m.extend_from_slice(CLAIM_DOMAIN);
    m.extend_from_slice(&campaign.0);
    m.extend_from_slice(host.as_bytes());
    m.extend_from_slice(node.as_bytes());
    m
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("not a chummer-rs invite link")]
    NotALink,
    #[error("the invite link has no host id")]
    MissingHost,
    #[error("the invite link's host id is not valid")]
    BadHost,
    #[error("the invite link has no campaign id")]
    MissingCampaign,
    #[error("not a valid {0}")]
    BadId(&'static str),
    #[error("the invite link's relay is not a valid URL")]
    BadRelay,
    #[error("this is an invite link of an older chummer-rs version; ask your GM for a new one")]
    OldLink,
}

/// A parsed `chummer-rs://join/...` link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteLink {
    /// The GM's node id: the host to dial.
    pub host: EndpointId,
    pub campaign: CampaignId,
    /// The invite's member key. Absent for members the GM added by node id.
    pub member: Option<MemberSecret>,
    /// The GM's campaign key (public), for the player's mailbox.
    pub gm_key: Option<PublicKey>,
    /// The GM's home relay, when it may not be in the player's relay list.
    pub relay: Option<RelayUrl>,
}

impl fmt::Display for InviteLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}://join/{}?campaign={}", self.host, self.campaign)?;
        if let Some(m) = &self.member {
            write!(f, "&member={m}")?;
        }
        if let Some(k) = &self.gm_key {
            write!(f, "&gm={k}")?;
        }
        if let Some(r) = &self.relay {
            let enc: String = url::form_urlencoded::byte_serialize(r.as_str().as_bytes()).collect();
            write!(f, "&relay={enc}")?;
        }
        Ok(())
    }
}

impl FromStr for InviteLink {
    type Err = InviteError;

    fn from_str(s: &str) -> Result<Self, InviteError> {
        let url = url::Url::parse(s.trim()).map_err(|_| InviteError::NotALink)?;
        if url.scheme() != SCHEME || url.host_str() != Some("join") {
            return Err(InviteError::NotALink);
        }
        let host = url.path().trim_matches('/');
        if host.is_empty() {
            return Err(InviteError::MissingHost);
        }
        let host = host.parse::<EndpointId>().map_err(|_| InviteError::BadHost)?;
        let (mut campaign, mut member, mut gm_key, mut relay) = (None, None, None, None);
        for (k, v) in url.query_pairs() {
            match &*k {
                "campaign" => campaign = Some(v.parse::<CampaignId>()?),
                "member" => member = Some(v.parse::<MemberSecret>()?),
                "gm" => gm_key = Some(v.parse::<PublicKey>().map_err(|_| InviteError::BadId("GM key"))?),
                "relay" => relay = Some(v.parse::<RelayUrl>().map_err(|_| InviteError::BadRelay)?),
                // Shared tokens of the first protocol: no longer valid.
                "invite" => return Err(InviteError::OldLink),
                _ => {} // unknown keys: ignored, for forward compatibility
            }
        }
        Ok(InviteLink { host, campaign: campaign.ok_or(InviteError::MissingCampaign)?, member, gm_key, relay })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> EndpointId {
        iroh::SecretKey::from_bytes(&[7; 32]).public()
    }

    #[test]
    fn format_and_parse() {
        let gm = SecretKey::from_bytes(&[9; 32]);
        let link = InviteLink { host: host(), campaign: CampaignId([0x11; 16]), member: Some(MemberSecret([0xab; 32])), gm_key: Some(gm.public()), relay: None };
        let s = link.to_string();
        assert_eq!(s, format!("chummer-rs://join/{}?campaign={}&member={}&gm={}", host(), "11".repeat(16), "ab".repeat(32), gm.public()));
        assert_eq!(s.parse::<InviteLink>().unwrap(), link);
        assert!(!format!("{link:?}").contains(&"ab".repeat(32)), "Debug hides the member secret");
    }

    #[test]
    fn relay_hint_round_trips() {
        let link = InviteLink { host: host(), campaign: CampaignId::random(), member: None, gm_key: None, relay: Some("https://relay.example.org:8443/".parse().unwrap()) };
        let s = link.to_string();
        assert!(s.contains("relay=https%3A%2F%2Frelay.example.org%3A8443%2F"), "{s}");
        assert_eq!(s.parse::<InviteLink>().unwrap(), link);
    }

    #[test]
    fn rejects_bad_links() {
        let id = host();
        let c = "11".repeat(16);
        assert_eq!("https://x/join".parse::<InviteLink>(), Err(InviteError::NotALink));
        assert_eq!("chummer-rs://open/abc".parse::<InviteLink>(), Err(InviteError::NotALink));
        assert_eq!(format!("chummer-rs://join/?campaign={c}").parse::<InviteLink>(), Err(InviteError::MissingHost));
        assert_eq!(format!("chummer-rs://join/nothex?campaign={c}").parse::<InviteLink>(), Err(InviteError::BadHost));
        assert_eq!(format!("chummer-rs://join/{id}").parse::<InviteLink>(), Err(InviteError::MissingCampaign));
        assert_eq!(format!("chummer-rs://join/{id}?campaign=12").parse::<InviteLink>(), Err(InviteError::BadId("CampaignId")));
        assert_eq!(format!("chummer-rs://join/{id}?campaign={c}&member=zz").parse::<InviteLink>(), Err(InviteError::BadId("member key")));
        assert_eq!(format!("chummer-rs://join/{id}?campaign={c}&invite={}", "ab".repeat(16)).parse::<InviteLink>(), Err(InviteError::OldLink));
        // Unknown keys are ignored.
        assert!(format!("chummer-rs://join/{id}?campaign={c}&future=1").parse::<InviteLink>().is_ok());
    }

    #[test]
    fn ids_and_member_keys_are_random() {
        assert_ne!(InviteId::random(), InviteId::random());
        assert_ne!(MemberSecret::random(), MemberSecret::random());
    }

    #[test]
    fn campaign_keys_are_per_campaign_and_generation() {
        let gm = SecretKey::from_bytes(&[3; 32]);
        let (a, b) = (CampaignId([1; 16]), CampaignId([2; 16]));
        let k = derive_campaign_key(&gm, a, 0).public();
        assert_eq!(k, derive_campaign_key(&gm, a, 0).public(), "deterministic");
        assert_ne!(k, derive_campaign_key(&gm, b, 0).public());
        assert_ne!(k, derive_campaign_key(&gm, a, 1).public());
        assert_ne!(k, gm.public());
        assert_ne!(k, derive_campaign_key(&SecretKey::from_bytes(&[4; 32]), a, 0).public());
    }
}
