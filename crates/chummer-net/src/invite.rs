//! Campaign ids, invite tokens and invite links.
//!
//! An invite link is
//! `chummer-rs://join/<gm-node-id>?campaign=<id>&invite=<token>`, with an
//! optional `&relay=<url>` hint when the GM uses a relay that is not in the
//! player's default list. Ids and tokens are 128 random bits written as 32
//! lower-case hex characters; the node id is iroh's 64-character hex form.
//!
//! The host keeps an [`InviteStore`]: which tokens are valid and the role
//! each one grants. A token stays valid until the GM revokes it, so one link
//! can be posted to a whole group.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use iroh::{EndpointId, RelayUrl};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// URL scheme of invite links.
pub const SCHEME: &str = "chummer-rs";

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
    /// A secret that lets a new player join a campaign.
    InviteToken
);

/// What a member of a campaign may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Sees and edits every character in the campaign.
    Gm,
    /// Sees and edits only their own characters.
    Player,
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
}

/// A parsed `chummer-rs://join/...` link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteLink {
    /// The GM's node id: the host to dial.
    pub host: EndpointId,
    pub campaign: CampaignId,
    /// Absent for links that only point at a campaign (members rejoining).
    pub invite: Option<InviteToken>,
    /// The GM's home relay, when it may not be in the player's relay list.
    pub relay: Option<RelayUrl>,
}

impl fmt::Display for InviteLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{SCHEME}://join/{}?campaign={}",
            self.host, self.campaign
        )?;
        if let Some(t) = &self.invite {
            write!(f, "&invite={t}")?;
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
        let host = host
            .parse::<EndpointId>()
            .map_err(|_| InviteError::BadHost)?;
        let (mut campaign, mut invite, mut relay) = (None, None, None);
        for (k, v) in url.query_pairs() {
            match &*k {
                "campaign" => campaign = Some(v.parse::<CampaignId>()?),
                "invite" => invite = Some(v.parse::<InviteToken>()?),
                "relay" => relay = Some(v.parse::<RelayUrl>().map_err(|_| InviteError::BadRelay)?),
                _ => {} // unknown keys: ignored, for forward compatibility
            }
        }
        Ok(InviteLink {
            host,
            campaign: campaign.ok_or(InviteError::MissingCampaign)?,
            invite,
            relay,
        })
    }
}

/// One invite the host has handed out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub role: Role,
    /// Free text for the GM ("Thursday group").
    pub label: String,
    /// Unix seconds.
    pub created: u64,
}

/// The host's valid invite tokens and the role each one grants.
///
/// Serialisable (JSON, TOML or postcard) so the host can keep it with the
/// campaign.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteStore {
    invites: BTreeMap<InviteToken, Invite>,
}

impl InviteStore {
    /// Makes a new random token granting `role`.
    pub fn create(&mut self, role: Role, label: impl Into<String>, now_unix: u64) -> InviteToken {
        let token = InviteToken::random();
        self.invites.insert(
            token,
            Invite {
                role,
                label: label.into(),
                created: now_unix,
            },
        );
        token
    }

    /// The role `token` grants, if it is valid.
    pub fn redeem(&self, token: &InviteToken) -> Option<Role> {
        self.invites.get(token).map(|i| i.role)
    }

    /// Invalidates `token`. Returns whether it existed.
    pub fn revoke(&mut self, token: &InviteToken) -> bool {
        self.invites.remove(token).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&InviteToken, &Invite)> {
        self.invites.iter()
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
        let link = InviteLink {
            host: host(),
            campaign: CampaignId([0x11; 16]),
            invite: Some(InviteToken([0xab; 16])),
            relay: None,
        };
        let s = link.to_string();
        assert_eq!(
            s,
            format!(
                "chummer-rs://join/{}?campaign={}&invite={}",
                host(),
                "11".repeat(16),
                "ab".repeat(16)
            )
        );
        assert_eq!(s.parse::<InviteLink>().unwrap(), link);
    }

    #[test]
    fn relay_hint_round_trips() {
        let link = InviteLink {
            host: host(),
            campaign: CampaignId::random(),
            invite: None,
            relay: Some("https://relay.example.org:8443/".parse().unwrap()),
        };
        let s = link.to_string();
        assert!(
            s.contains("relay=https%3A%2F%2Frelay.example.org%3A8443%2F"),
            "{s}"
        );
        assert_eq!(s.parse::<InviteLink>().unwrap(), link);
    }

    #[test]
    fn rejects_bad_links() {
        let id = host();
        let c = "11".repeat(16);
        assert_eq!(
            "https://x/join".parse::<InviteLink>(),
            Err(InviteError::NotALink)
        );
        assert_eq!(
            "chummer-rs://open/abc".parse::<InviteLink>(),
            Err(InviteError::NotALink)
        );
        assert_eq!(
            format!("chummer-rs://join/?campaign={c}").parse::<InviteLink>(),
            Err(InviteError::MissingHost)
        );
        assert_eq!(
            format!("chummer-rs://join/nothex?campaign={c}").parse::<InviteLink>(),
            Err(InviteError::BadHost)
        );
        assert_eq!(
            format!("chummer-rs://join/{id}").parse::<InviteLink>(),
            Err(InviteError::MissingCampaign)
        );
        assert_eq!(
            format!("chummer-rs://join/{id}?campaign=12").parse::<InviteLink>(),
            Err(InviteError::BadId("CampaignId"))
        );
        assert_eq!(
            format!("chummer-rs://join/{id}?campaign={c}&invite=zz").parse::<InviteLink>(),
            Err(InviteError::BadId("InviteToken"))
        );
        // Unknown keys are ignored.
        assert!(format!("chummer-rs://join/{id}?campaign={c}&future=1")
            .parse::<InviteLink>()
            .is_ok());
    }

    #[test]
    fn tokens_are_random() {
        assert_ne!(InviteToken::random(), InviteToken::random());
    }

    #[test]
    fn store_grants_roles_until_revoked() {
        let mut store = InviteStore::default();
        let p = store.create(Role::Player, "group", 100);
        let g = store.create(Role::Gm, "co-gm", 100);
        assert_eq!(store.redeem(&p), Some(Role::Player));
        assert_eq!(store.redeem(&g), Some(Role::Gm));
        assert_eq!(store.redeem(&InviteToken::random()), None);
        // Survives a JSON-like round trip via postcard and keeps working.
        let bytes = postcard::to_stdvec(&store).unwrap();
        let mut back: InviteStore = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, store);
        assert!(back.revoke(&p));
        assert!(!back.revoke(&p));
        assert_eq!(back.redeem(&p), None);
        assert_eq!(back.iter().count(), 1);
    }
}
