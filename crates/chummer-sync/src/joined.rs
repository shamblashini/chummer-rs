//! The campaigns this player has joined, as the app lists them: one JSON
//! file, `campaigns/joined.json` in the chummer-rs config directory
//! (next to `node.key`), and one replica file per campaign,
//! `campaigns/<campaign-id>.replica`.

use std::path::{Path, PathBuf};

use chummer_net::invite::InviteLink;
use serde::{Deserialize, Serialize};

/// One joined campaign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joined {
    /// The invite link it was joined with (it keeps working for this
    /// device; it holds the member key, so the file is private).
    pub link: String,
    /// The campaign's name, once the GM sent it.
    #[serde(default)]
    pub name: String,
}

impl Joined {
    pub fn link(&self) -> Option<InviteLink> {
        self.link.parse().ok()
    }

    /// The campaign id as text, or the whole link when it does not parse.
    pub fn key(&self) -> String {
        self.link().map(|l| l.campaign.to_string()).unwrap_or_else(|| self.link.clone())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinedList {
    pub campaigns: Vec<Joined>,
}

/// `campaigns` in the chummer-rs config directory.
pub fn dir() -> Option<PathBuf> {
    chummer_net::identity::config_dir().map(|d| d.join("campaigns"))
}

impl JoinedList {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("joined.json")
    }

    /// The replica file of a campaign.
    pub fn replica_path(dir: &Path, j: &Joined) -> PathBuf {
        dir.join(format!("{}.replica", j.key()))
    }

    /// The list in `dir`; empty when there is none (or it is damaged).
    pub fn load(dir: &Path) -> JoinedList {
        std::fs::read_to_string(Self::path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("serialises");
        crate::persist::write_atomic_private(&Self::path(dir), text.as_bytes())
    }

    /// Adds `link` (a campaign joined again replaces its old entry, keeping
    /// the name). Returns the entry.
    pub fn add(&mut self, link: &InviteLink) -> Joined {
        let key = link.campaign.to_string();
        let name = self.campaigns.iter().find(|j| j.key() == key).map(|j| j.name.clone()).unwrap_or_default();
        self.campaigns.retain(|j| j.key() != key);
        let j = Joined { link: link.to_string(), name };
        self.campaigns.push(j.clone());
        j
    }

    /// Forgets a campaign (its replica file stays until removed).
    pub fn remove(&mut self, key: &str) -> Option<Joined> {
        let i = self.campaigns.iter().position(|j| j.key() == key)?;
        Some(self.campaigns.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_save_load() {
        let dir = std::env::temp_dir().join(format!("chummer-sync-joined-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let host = chummer_net::SecretKey::from_bytes(&[5; 32]).public();
        let link = InviteLink { host, campaign: chummer_net::invite::CampaignId([1; 16]), member: Some(chummer_net::invite::MemberSecret([2; 32])), gm_key: None, relay: None };
        let mut l = JoinedList::default();
        let j = l.add(&link);
        l.campaigns[0].name = "Seattle".into();
        l.add(&link);
        assert_eq!(l.campaigns.len(), 1);
        assert_eq!(l.campaigns[0].name, "Seattle", "joining again keeps the name");
        l.save(&dir).unwrap();
        assert_eq!(JoinedList::load(&dir), l);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(JoinedList::path(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the links hold member keys");
        }
        assert!(JoinedList::replica_path(&dir, &j).ends_with(format!("{}.replica", "01".repeat(16))));
        assert!(l.remove(&j.key()).is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
