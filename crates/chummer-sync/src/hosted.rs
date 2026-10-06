//! A GM's campaign file served as an online campaign: the glue between
//! [`chummer_core::campaign::Campaign`] (what the GM opens and edits) and
//! the [`Authority`] (versions, logs, members, invites).
//!
//! # Files
//!
//! The GM opens one file, `<name>.chummercampaign`. Next to it, the
//! authority's state is kept in `<name>.authority` (the sidecar,
//! [`authority_path`]): the characters with their versions and command
//! logs, members, invites, what each member was sent and the mailbox
//! state. The sidecar is made the first time the campaign is hosted and
//! found again by name; the GM never opens it directly. `<name>.invites`
//! ([`invites_path`]) holds invites made by `chummer-authority invite`
//! while a host is running; the host takes them in.
//!
//! # Which copy wins
//!
//! Once a campaign has a sidecar, every change to its characters goes
//! through the authority (the GM's too, as [`AuthorityHost::gm_edit`]),
//! hosted or not. So the authority's characters are the newest:
//! [`reconcile`] keeps them and only adds members that are new in the
//! campaign file, drops removed ones and copies owners across;
//! [`write_back`] copies the characters into the campaign file (embedded
//! members) or their own files (linked members) when the GM saves.
//!
//! # Ids
//!
//! A member's [`MemberId`] (32 hex digits) is its [`CharacterId`]. A member
//! with an `owner` (a player's node id) is that player's character; the
//! rest (NPCs, critters, spirits) belong to the GM and players never see
//! them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chummer_core::campaign::{Campaign, Member, MemberId};
use chummer_core::character::Character;
use chummer_core::command;
use chummer_core::engine::Engine;
use chummer_net::invite::{CampaignId, Invite, InviteLink, InviteToken, Role};
use chummer_net::{EndpointId, SecretKey};

use crate::authority::Authority;
use crate::host::AuthorityHost;
use crate::msg::CharacterId;
use crate::node::Node;
use crate::persist::PersistError;

/// Extension of the authority sidecar.
pub const AUTHORITY_EXTENSION: &str = "authority";
/// Extension of the invites file.
pub const INVITES_EXTENSION: &str = "invites";

/// `<name>.authority` next to `<name>.chummercampaign`.
pub fn authority_path(campaign: &Path) -> PathBuf {
    campaign.with_extension(AUTHORITY_EXTENSION)
}

/// `<name>.invites` next to `<name>.chummercampaign`.
pub fn invites_path(campaign: &Path) -> PathBuf {
    campaign.with_extension(INVITES_EXTENSION)
}

/// Whether the campaign at `campaign` has been hosted (has a sidecar).
pub fn is_online(campaign: &Path) -> bool {
    authority_path(campaign).exists()
}

pub fn character_id(m: MemberId) -> CharacterId {
    CharacterId::new(m.to_string())
}

pub fn member_id(c: &CharacterId) -> Option<MemberId> {
    c.0.parse().ok()
}

/// The player who owns `m`, if it is a player character.
pub fn owner_of(m: &Member) -> Option<EndpointId> {
    m.owner.as_deref().and_then(|o| o.trim().parse().ok())
}

/// What the campaign file says about `m`'s owner: `None` when it says
/// nothing (the authority's owner stands), `Some(None)` for "the GM's"
/// ([`GM_OWNER`] or any text that is not a node id), `Some(Some(id))` for
/// a player.
pub fn explicit_owner(m: &Member) -> Option<Option<EndpointId>> {
    m.owner.as_deref().map(|o| o.trim().parse().ok())
}

/// The `owner` text that gives a character back to the GM.
pub const GM_OWNER: &str = "gm";

pub fn campaign_id(c: &Campaign) -> CampaignId {
    CampaignId(c.id.0)
}

#[derive(Debug, thiserror::Error)]
pub enum HostedError {
    #[error("could not read the campaign's online state: {0}")]
    Persist(#[from] PersistError),
    #[error("{0} belongs to another campaign")]
    WrongCampaign(PathBuf),
    #[error("this campaign is hosted with another node key ({0}); copy that machine's node.key to host it here")]
    OtherGm(String),
    #[error("save the campaign to a file before hosting it")]
    NoFile,
    #[error("could not write {0}: {1}")]
    Io(PathBuf, std::io::Error),
}

/// What [`reconcile`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Members that became characters of the authority.
    pub added: Vec<MemberId>,
    /// Characters dropped because their member is gone.
    pub removed: usize,
    /// Characters whose owner changed.
    pub owners: usize,
    /// Members whose character could not be loaded.
    pub failed: Vec<(MemberId, String)>,
}

/// Brings the authority in line with the campaign file. New members are
/// added with their character from `current` (the GM's open copy) or, if
/// that gives none, from the file; existing characters keep the
/// authority's state. The campaign name follows the file, and so does an
/// owner the file names ([`explicit_owner`]); a member without one keeps
/// the authority's owner ([`write_back`] then writes it into the file). A
/// member's owner who is not a member of the authority yet is added as a
/// player (so a GM who knows a player's node id needs no invite).
pub fn reconcile(auth: &mut Authority, campaign: &Campaign, base: Option<&Path>, mut current: impl FnMut(MemberId) -> Option<Character>) -> Reconciled {
    let mut out = Reconciled::default();
    auth.set_name(&campaign.name);
    for m in &campaign.members {
        let id = character_id(m.id);
        let explicit = explicit_owner(m);
        let owner = explicit.flatten();
        if let Some(p) = owner {
            if auth.role(&p).is_none() {
                auth.add_member(p, Role::Player, m.player.clone());
            }
        }
        if auth.character(&id).is_none() {
            let ch = match current(m.id) {
                Some(ch) => Ok(ch),
                None => m.load_character(base).map_err(|e| e.to_string()),
            };
            match ch.and_then(|ch| auth.add_character(id, owner, ch).map_err(|e| e.to_string())) {
                Ok(_) => out.added.push(m.id),
                Err(e) => out.failed.push((m.id, e)),
            }
        } else if explicit.is_some() && auth.owner(&id) != owner {
            auth.set_owner(&id, owner);
            out.owners += 1;
        }
    }
    let gone: Vec<CharacterId> = auth.characters().filter(|c| member_id(c).is_none_or(|m| campaign.member(m).is_none())).cloned().collect();
    for c in gone {
        auth.remove_character(&c);
        out.removed += 1;
    }
    out
}

/// Copies the authority's characters into the campaign: embedded members
/// store theirs, linked members are saved to their own file when it
/// differs. The roster names follow.
pub fn write_back(auth: &Authority, campaign: &mut Campaign, base: Option<&Path>, engine: &Engine) -> Result<(), String> {
    for m in &mut campaign.members {
        let id = character_id(m.id);
        let Some(ch) = auth.character(&id) else { continue };
        m.owner = auth.owner(&id).map(|o| o.to_string());
        match m.linked_path(base) {
            Some(p) => {
                let same = Character::load(&p).is_ok_and(|on_disk| command::state_hash(&on_disk) == command::state_hash(ch));
                if !same {
                    let mut copy = ch.clone();
                    engine.save(&mut copy, &p).map_err(|e| format!("could not save {}: {e}", p.display()))?;
                }
                m.name = ch.display_name();
            }
            None => m.store_character(ch),
        }
    }
    Ok(())
}

/// Takes in the tokens of `invites_path` that `auth` does not know yet.
/// Lines are `<token> <gm|player> <label>`; others are skipped. Returns
/// how many were new.
pub fn merge_invites(auth: &mut Authority, file: &Path) -> usize {
    let Ok(text) = std::fs::read_to_string(file) else { return 0 };
    let mut n = 0;
    for line in text.lines() {
        let mut parts = line.trim().splitn(3, ' ');
        let (Some(token), Some(role)) = (parts.next(), parts.next()) else { continue };
        let Ok(token) = token.parse::<InviteToken>() else { continue };
        let role = match role {
            "gm" => Role::Gm,
            "player" => Role::Player,
            _ => continue,
        };
        let label = parts.next().unwrap_or("").to_owned();
        let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if auth.invites_mut().insert(token, Invite { role, label, created }) {
            n += 1;
        }
    }
    n
}

/// Writes a new invite token to `file` (for a running host to take in).
pub fn append_invite(file: &Path, role: Role, label: &str) -> std::io::Result<InviteToken> {
    let token = InviteToken::random();
    let role = match role {
        Role::Gm => "gm",
        Role::Player => "player",
    };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    writeln!(f, "{token} {role} {}", label.replace(['\n', '\r'], " "))?;
    Ok(token)
}

/// The authority for `campaign`: loaded from its sidecar, or new.
pub fn open_authority(campaign: &Campaign, campaign_path: &Path, gm: EndpointId, gm_name: &str) -> Result<Authority, HostedError> {
    let side = authority_path(campaign_path);
    if !side.exists() {
        return Ok(Authority::new(campaign_id(campaign), gm, gm_name));
    }
    let a = Authority::load(&side)?;
    if a.campaign() != campaign_id(campaign) {
        return Err(HostedError::WrongCampaign(side));
    }
    if a.gm() != gm {
        return Err(HostedError::OtherGm(a.gm().fmt_short().to_string()));
    }
    Ok(a)
}

/// A campaign file with its authority, ready to serve. Cheap to clone.
#[derive(Debug, Clone)]
pub struct HostedCampaign {
    pub host: AuthorityHost,
    campaign_path: PathBuf,
}

impl HostedCampaign {
    /// Opens (or, the first time, makes) the authority of the campaign
    /// saved at `campaign_path`, reconciled with `campaign`; `current`
    /// gives the GM's open characters for members that are new to it.
    /// Must be called inside a tokio runtime.
    pub fn open(campaign: &Campaign, campaign_path: &Path, engine: Arc<Engine>, secret: SecretKey, gm_name: &str, current: impl FnMut(MemberId) -> Option<Character>) -> Result<(HostedCampaign, Reconciled), HostedError> {
        let mut auth = open_authority(campaign, campaign_path, secret.public(), gm_name)?;
        let base = campaign_path.parent();
        let rec = reconcile(&mut auth, campaign, base, current);
        merge_invites(&mut auth, &invites_path(campaign_path));
        let side = authority_path(campaign_path);
        let host = AuthorityHost::new(auth, engine, secret, Some(side.clone()));
        host.save().map_err(|e| HostedError::Io(side, e))?;
        Ok((HostedCampaign { host, campaign_path: campaign_path.to_owned() }, rec))
    }

    pub fn campaign_path(&self) -> &Path {
        &self.campaign_path
    }

    /// After the GM changed the roster or owners: [`reconcile`], then tell
    /// connected members.
    pub fn reconcile(&self, campaign: &Campaign, current: impl FnMut(MemberId) -> Option<Character>) -> Reconciled {
        let r = reconcile(&mut self.host.authority(), campaign, self.campaign_path.parent(), current);
        self.host.changed();
        r
    }

    /// [`write_back`] into `campaign`.
    pub fn write_back(&self, campaign: &mut Campaign) -> Result<(), String> {
        let engine = self.host.engine().clone();
        write_back(&self.host.authority(), campaign, self.campaign_path.parent(), &engine)
    }

    /// Takes in invites written by `chummer-authority invite`.
    pub fn merge_invites(&self) -> usize {
        let n = merge_invites(&mut self.host.authority(), &invites_path(&self.campaign_path));
        if n > 0 {
            let _ = self.host.save();
        }
        n
    }

    /// A new invite link. With a node, the link carries its relay when
    /// that is not the project's default relay.
    pub fn invite(&self, role: Role, label: &str, node: Option<&Node>) -> InviteLink {
        let link = self.host.invite(role, label, node.and_then(Node::relay_hint));
        let _ = self.host.save();
        link
    }

    pub fn character(&self, m: MemberId) -> Option<Character> {
        self.host.authority().character(&character_id(m)).cloned()
    }
}
