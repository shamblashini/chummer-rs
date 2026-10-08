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
//! ([`invites_path`]) holds changes to the invites made by
//! `chummer-authority invite ...` ([`InviteOp`]s, one JSON object per
//! line; the file holds member keys, so it is readable by its owner only).
//! A host takes them in when it opens the campaign and every few seconds
//! while it runs ([`take_invite_ops`]).
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
use chummer_net::invite::{CampaignId, InviteId, InviteLink, Role};
use chummer_net::{EndpointId, SecretKey};

use crate::invites::{Invite, InviteOp};

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
/// the authority's owner ([`write_back`] then writes it into the file).
/// The authority's owner also stands when a claim changed it and the
/// file has not taken that yet ([`adopt_owner_changes`]), and when the
/// file names a node that was revoked or replaced. A member's owner who
/// is not a member of the authority yet is added as a player (so a GM
/// who knows a player's node id needs no invite), unless that node was
/// revoked or replaced.
pub fn reconcile(auth: &mut Authority, campaign: &Campaign, base: Option<&Path>, mut current: impl FnMut(MemberId) -> Option<Character>) -> Reconciled {
    let mut out = Reconciled::default();
    auth.set_name(&campaign.name);
    for m in &campaign.members {
        let id = character_id(m.id);
        let explicit = explicit_owner(m);
        let owner = explicit.flatten();
        if let Some(p) = owner {
            if auth.role(&p).is_none() && !auth.is_retired(&p) {
                auth.add_member(p, Role::Player, m.player.clone());
            }
        }
        let stale = auth.has_owner_change(&id) || owner.is_some_and(|p| auth.is_retired(&p));
        if auth.character(&id).is_none() {
            let ch = match current(m.id) {
                Some(ch) => Ok(ch),
                None => m.load_character(base).map_err(|e| e.to_string()),
            };
            match ch.and_then(|ch| auth.add_character(id, owner, ch).map_err(|e| e.to_string())) {
                Ok(_) => out.added.push(m.id),
                Err(e) => out.failed.push((m.id, e)),
            }
        } else if explicit.is_some() && !stale && auth.owner(&id) != owner {
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

/// Shows the authority's owners in the campaign: a member whose `owner`
/// the file leaves open (or marks [`GM_OWNER`]) gets the authority's.
/// Returns whether anything changed.
pub fn adopt_owners(auth: &Authority, campaign: &mut Campaign) -> bool {
    let mut changed = false;
    for m in &mut campaign.members {
        if explicit_owner(m).is_some_and(|o| o.is_some()) {
            continue;
        }
        let owner = auth.owner(&character_id(m.id)).map(|o| o.to_string());
        let normal = if owner.is_none() && m.owner.is_some() { m.owner.clone() } else { owner };
        if m.owner != normal {
            m.owner = normal;
            changed = true;
        }
    }
    changed
}

/// Copies the authority's characters into the campaign: embedded members
/// store theirs, linked members are saved to their own file when it
/// differs. The roster names follow.
pub fn write_back(auth: &Authority, campaign: &mut Campaign, base: Option<&Path>, engine: &Engine) -> Result<(), String> {
    let linked = write_back_embedded(auth, campaign, base);
    save_linked(engine, linked)
}

/// The quick half of [`write_back`]: owners, names and embedded
/// characters go into the campaign; the linked members' characters are
/// returned with their files, for [`save_linked`] (reading and writing
/// files: run it where waiting does no harm).
pub fn write_back_embedded(auth: &Authority, campaign: &mut Campaign, base: Option<&Path>) -> Vec<(std::path::PathBuf, Character)> {
    let mut linked = Vec::new();
    for m in &mut campaign.members {
        let id = character_id(m.id);
        let Some(ch) = auth.character(&id) else { continue };
        m.owner = auth.owner(&id).map(|o| o.to_string());
        match m.linked_path(base) {
            Some(p) => {
                m.name = ch.display_name();
                linked.push((p, ch.clone()));
            }
            None => m.store_character(ch),
        }
    }
    linked
}

/// The slow half of [`write_back`]: each linked character is saved to
/// its file when that differs.
pub fn save_linked(engine: &Engine, linked: Vec<(std::path::PathBuf, Character)>) -> Result<(), String> {
    for (p, ch) in linked {
        let same = Character::load(&p).is_ok_and(|on_disk| command::state_hash(&on_disk) == command::state_hash(&ch));
        if !same {
            let mut copy = ch;
            engine.save(&mut copy, &p).map_err(|e| format!("could not save {}: {e}", p.display()))?;
        }
    }
    Ok(())
}

/// Puts the authority's owners into the campaign file for the
/// characters whose owner a claim changed ([`Authority::take_owner_changes`]).
/// Returns whether anything changed (the file needs saving).
pub fn adopt_owner_changes(auth: &mut Authority, campaign: &mut Campaign) -> bool {
    let changes = auth.take_owner_changes();
    let mut changed = false;
    for m in &mut campaign.members {
        let id = character_id(m.id);
        if !changes.contains(&id) {
            continue;
        }
        let owner = Some(auth.owner(&id).map(|o| o.to_string()).unwrap_or_else(|| GM_OWNER.to_owned()));
        if m.owner != owner {
            m.owner = owner;
            changed = true;
        }
    }
    changed
}

/// `<name>.invites.merging`: the ops being applied.
fn merging_path(file: &Path) -> PathBuf {
    let mut p = file.as_os_str().to_owned();
    p.push(".merging");
    PathBuf::from(p)
}

fn parse_ops(text: &str) -> Vec<InviteOp> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).filter_map(|l| match serde_json::from_str(l) {
        Ok(op) => Some(op),
        Err(e) => {
            tracing::warn!("skipping a line of the invites file: {e}");
            None
        }
    }).collect()
}

/// Takes the changes `chummer-authority invite ...` wrote to `file`: the
/// file is moved aside (so lines written meanwhile go to a new one) and
/// read. Call [`invite_ops_done`] once they are applied and saved; until
/// then a crash leaves them to be applied again (they are idempotent).
pub fn take_invite_ops(file: &Path) -> Vec<InviteOp> {
    let merging = merging_path(file);
    if file.exists() && !merging.exists() {
        if let Err(e) = std::fs::rename(file, &merging) {
            tracing::warn!("could not take {}: {e}", file.display());
        }
    } else if file.exists() {
        // A merge was cut short: add the new lines to it.
        if let Ok(text) = std::fs::read_to_string(file) {
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&merging) {
                if f.write_all(text.as_bytes()).is_ok() {
                    let _ = std::fs::remove_file(file);
                }
            }
        }
    }
    std::fs::read_to_string(&merging).map(|t| parse_ops(&t)).unwrap_or_default()
}

/// The changes from [`take_invite_ops`] are applied and saved.
pub fn invite_ops_done(file: &Path) {
    let _ = std::fs::remove_file(merging_path(file));
}

/// Changes written to `file` that no host has taken yet (for listing).
pub fn pending_invite_ops(file: &Path) -> Vec<InviteOp> {
    let mut ops = std::fs::read_to_string(merging_path(file)).map(|t| parse_ops(&t)).unwrap_or_default();
    ops.extend(std::fs::read_to_string(file).map(|t| parse_ops(&t)).unwrap_or_default());
    ops
}

/// Appends a change to `file` for a running host (or the next one) to
/// apply. The file is readable by its owner only (it holds member keys).
pub fn append_invite_op(file: &Path, op: &InviteOp) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(file)?;
    let line = serde_json::to_string(op).map_err(std::io::Error::other)?;
    writeln!(f, "{line}")
}

/// Applies the pending invite changes of `file` to `auth` (a host being
/// opened). Returns how many changed something.
pub fn merge_invites(auth: &mut Authority, file: &Path) -> usize {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    take_invite_ops(file).into_iter().filter(|op| auth.apply_invite_op(op.clone(), now).0).count()
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
        merge_invites(&mut auth, &invites_path(campaign_path));
        let rec = reconcile(&mut auth, campaign, base, current);
        let side = authority_path(campaign_path);
        let host = AuthorityHost::new(auth, engine, secret, Some(side.clone()));
        host.save().map_err(|e| HostedError::Io(side, e))?;
        invite_ops_done(&invites_path(campaign_path));
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

    /// [`adopt_owners`] into `campaign`.
    pub fn adopt_owners(&self, campaign: &mut Campaign) -> bool {
        adopt_owners(&self.host.authority(), campaign)
    }

    /// [`write_back_embedded`] into `campaign`.
    pub fn write_back_embedded(&self, campaign: &mut Campaign) -> Vec<(std::path::PathBuf, Character)> {
        write_back_embedded(&self.host.authority(), campaign, self.campaign_path.parent())
    }

    /// [`write_back`] into `campaign`.
    pub fn write_back(&self, campaign: &mut Campaign) -> Result<(), String> {
        let engine = self.host.engine().clone();
        write_back(&self.host.authority(), campaign, self.campaign_path.parent(), &engine)
    }

    /// Takes in the invite changes written by `chummer-authority invite`.
    /// Returns how many changed something.
    pub fn merge_invites(&self) -> usize {
        let file = invites_path(&self.campaign_path);
        let ops = take_invite_ops(&file);
        if ops.is_empty() {
            return 0;
        }
        let n = self.host.apply_invite_ops(ops);
        if self.host.save().is_ok() {
            invite_ops_done(&file);
        }
        n
    }

    /// [`adopt_owner_changes`] into `campaign`.
    pub fn adopt_owner_changes(&self, campaign: &mut Campaign) -> bool {
        let mut a = self.host.authority();
        if !a.has_owner_changes() {
            return false;
        }
        adopt_owner_changes(&mut a, campaign)
    }

    /// A new invite for one player; see [`AuthorityHost::create_invite`].
    /// With a node, the link carries its relay when that is not the
    /// project's default relay.
    pub fn create_invite(&self, label: &str, assign: Option<CharacterId>, expires: Option<u64>, node: Option<&Node>) -> (Invite, InviteLink) {
        let r = self.host.create_invite(Role::Player, label, assign, expires, node.and_then(Node::relay_hint));
        let _ = self.host.save();
        r
    }

    /// The current link of invite `id`.
    pub fn invite_link(&self, id: &InviteId, node: Option<&Node>) -> Option<InviteLink> {
        self.host.invite_link(id, node.and_then(Node::relay_hint))
    }

    /// A new link for the member of invite `id`.
    pub fn reissue_invite(&self, id: &InviteId, expires: Option<u64>, node: Option<&Node>) -> Result<InviteLink, String> {
        let r = self.host.reissue_invite(id, expires, node.and_then(Node::relay_hint));
        let _ = self.host.save();
        r
    }

    pub fn revoke_invite(&self, id: &InviteId) -> Result<(), String> {
        let r = self.host.revoke_invite(id);
        let _ = self.host.save();
        r
    }

    pub fn remove_invite(&self, id: &InviteId) {
        self.host.remove_invite(id);
        let _ = self.host.save();
    }

    /// A new GM campaign key ([`AuthorityHost::rotate_campaign_key`]),
    /// saved at once. Returns the new generation.
    pub fn rotate_campaign_key(&self) -> u32 {
        let gen = self.host.rotate_campaign_key();
        let _ = self.host.save();
        gen
    }

    pub fn remove_member(&self, peer: &EndpointId) -> bool {
        let r = self.host.remove_member(peer);
        let _ = self.host.save();
        r
    }

    pub fn character(&self, m: MemberId) -> Option<Character> {
        self.host.authority().character(&character_id(m)).cloned()
    }
}
