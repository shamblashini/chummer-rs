//! Per-player invites: what the authority keeps about each link the GM
//! handed out.
//!
//! The GM makes one invite per player, labelled ("Anna"). Each has its
//! own member key ([`MemberSecret`], in the link). Possessing the key is
//! the permission to join and to put mail into the GM's relay mailbox.
//!
//! - **Claim.** The first node that proves the key (live hello or mailed
//!   join) claims the invite: it becomes a member, bound to that node id.
//!   The same link on another device is refused ([`DenyReason::Claimed`]).
//! - **Re-issue.** The GM can give the member a new link (a new device):
//!   a new key, the invite unclaimed again, the old node out of the
//!   campaign and its key no longer valid ([`DenyReason::Superseded`]).
//!   Whoever claims the new link takes over the old node's characters.
//! - **Revoke.** The invite stays listed as revoked; its key and node are
//!   refused from then on.
//! - **Expiry.** An unclaimed invite may expire; claimed ones do not.
//!
//! The authority keeps the member secrets (the GM's own file), so the GM
//! can copy an unclaimed link again.
//!
//! `chummer-authority invite ...` writes [`InviteOp`]s to the campaign's
//! `.invites` file for a running host to apply
//! ([`crate::hosted::merge_invites`]).

use chummer_net::campaign::DenyReason;
use chummer_net::invite::{InviteId, MemberSecret, Role};
use chummer_net::{EndpointId, PublicKey};
use serde::{Deserialize, Serialize};

use crate::msg::CharacterId;

/// Keys of earlier links kept per invite, to tell their holders the link
/// was replaced.
const SUPERSEDED_KEPT: usize = 8;

/// Which node claimed an invite, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub node: EndpointId,
    /// Unix seconds.
    pub at: u64,
}

/// One invite (a member slot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub id: InviteId,
    pub role: Role,
    /// The GM's name for the member ("Anna").
    pub label: String,
    /// The current link's member key.
    pub secret: MemberSecret,
    /// Unix seconds: when the invite was made, and when its current link
    /// was issued.
    pub created: u64,
    pub issued: u64,
    /// Unix seconds after which an unclaimed invite no longer works.
    pub expires: Option<u64>,
    /// A character to give the member when the invite is claimed.
    pub assign: Option<CharacterId>,
    pub claimed: Option<Claim>,
    /// Unix seconds.
    pub revoked: Option<u64>,
    /// The node of the link before a re-issue: its characters go to
    /// whoever claims this one.
    pub previous: Option<EndpointId>,
    /// Keys of earlier links (newest last).
    pub superseded: Vec<PublicKey>,
}

/// Where an invite stands, for lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteState {
    /// Waiting for its player; `expires` when it stops working.
    Unclaimed { expires: Option<u64> },
    Expired,
    Claimed { node: EndpointId, at: u64 },
    Revoked { at: u64 },
}

impl Invite {
    pub fn new(role: Role, label: &str, assign: Option<CharacterId>, expires: Option<u64>, now: u64) -> Invite {
        Invite { id: InviteId::random(), role, label: label.trim().to_owned(), secret: MemberSecret::random(), created: now, issued: now, expires, assign, claimed: None, revoked: None, previous: None, superseded: Vec::new() }
    }

    pub fn key(&self) -> PublicKey {
        self.secret.public()
    }

    pub fn state(&self, now: u64) -> InviteState {
        if let Some(at) = self.revoked {
            return InviteState::Revoked { at };
        }
        match &self.claimed {
            Some(c) => InviteState::Claimed { node: c.node, at: c.at },
            None if self.expires.is_some_and(|e| e <= now) => InviteState::Expired,
            None => InviteState::Unclaimed { expires: self.expires },
        }
    }

    /// Whether its key may put mail into the GM's mailbox and join.
    pub fn active(&self, now: u64) -> bool {
        matches!(self.state(now), InviteState::Unclaimed { .. } | InviteState::Claimed { .. })
    }

    /// Why `node`, proving this invite's current key, may not claim it
    /// (`None`: it may).
    pub fn refusal(&self, node: &EndpointId, now: u64) -> Option<DenyReason> {
        match self.state(now) {
            InviteState::Revoked { .. } => Some(DenyReason::Revoked),
            InviteState::Expired => Some(DenyReason::Expired),
            InviteState::Claimed { node: n, .. } if n != *node => Some(DenyReason::Claimed),
            _ => None,
        }
    }

    /// A new link for the same member: a new key; unclaimed again, with
    /// the old node remembered for its characters. Returns the node that
    /// held the old link.
    pub fn reissue(&mut self, expires: Option<u64>, now: u64) -> Option<EndpointId> {
        let old = std::mem::replace(&mut self.secret, MemberSecret::random());
        self.superseded.push(old.public());
        if self.superseded.len() > SUPERSEDED_KEPT {
            self.superseded.remove(0);
        }
        let node = self.claimed.take().map(|c| c.node);
        if node.is_some() {
            self.previous = node;
        }
        self.revoked = None;
        self.issued = now;
        self.expires = expires;
        node
    }
}

/// A change to the invites, as `chummer-authority invite ...` writes it
/// for a running host (one JSON object per line in `<name>.invites`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum InviteOp {
    /// A new invite, made by the CLI (it printed the link already).
    Create { invite: Invite },
    Revoke { id: InviteId },
    /// A new link for the invite, with the CLI's new key.
    Reissue { id: InviteId, secret: MemberSecret, expires: Option<u64> },
    Remove { id: InviteId },
    /// Move the GM's campaign key to `generation` (when it is behind).
    RotateKey { generation: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_reissue() {
        let node = chummer_net::SecretKey::from_bytes(&[1; 32]).public();
        let other = chummer_net::SecretKey::from_bytes(&[2; 32]).public();
        let mut i = Invite::new(Role::Player, " Anna ", None, Some(100), 10);
        assert_eq!(i.label, "Anna");
        assert_eq!(i.state(50), InviteState::Unclaimed { expires: Some(100) });
        assert_eq!(i.state(100), InviteState::Expired);
        assert_eq!(i.refusal(&node, 100), Some(DenyReason::Expired));
        assert!(!i.active(100));
        i.claimed = Some(Claim { node, at: 60 });
        assert_eq!(i.state(500), InviteState::Claimed { node, at: 60 }, "a claimed invite does not expire");
        assert_eq!(i.refusal(&node, 500), None);
        assert_eq!(i.refusal(&other, 500), Some(DenyReason::Claimed));
        let old = i.key();
        assert_eq!(i.reissue(None, 700), Some(node));
        assert_ne!(i.key(), old);
        assert_eq!(i.superseded, [old]);
        assert_eq!(i.previous, Some(node));
        assert_eq!(i.state(800), InviteState::Unclaimed { expires: None });
        i.revoked = Some(900);
        assert_eq!(i.refusal(&node, 901), Some(DenyReason::Revoked));
        assert!(!i.active(901));
        // A line of the invites file round-trips.
        let op = InviteOp::Revoke { id: i.id };
        let line = serde_json::to_string(&op).unwrap();
        assert!(line.contains("\"op\":\"revoke\""), "{line}");
        assert_eq!(serde_json::from_str::<InviteOp>(&line).unwrap(), op);
    }
}
