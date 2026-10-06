//! The offline commands: invite, assign and status on a campaign file.

use std::path::Path;
use std::process::Command;

use chummer_core::campaign::{Campaign, Member, MemberKind};
use chummer_core::character::Character;
use chummer_net::invite::{InviteLink, Role};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_chummer-authority"));
    // Never touch the user's own config.
    c.env("XDG_CONFIG_HOME", std::env::temp_dir().join("chummer-authority-cli-none"));
    c
}

#[test]
fn invite_assign_status() {
    let dir = std::env::temp_dir().join(format!("chummer-authority-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("gm.key");
    let gm = chummer_net::SecretKey::generate();
    chummer_net::identity::write_key(&key, &gm).unwrap();
    let file = dir.join("Seattle.chummercampaign");
    let mut c = Campaign::new("Seattle");
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../chummer-core/tests/fixtures/Munin.chum5");
    let mut m = Member::embedded(MemberKind::Player, &Character::load(&p).unwrap());
    m.name = "Ghost".into();
    let id = c.add(m);
    c.save(&file).unwrap();

    // Without a key nothing is made up.
    let out = bin().args(["invite"]).arg(&file).output().unwrap();
    assert!(!out.status.success());

    let out = bin().arg("--key").arg(&key).args(["invite", "--label", "group"]).arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let link: InviteLink = String::from_utf8(out.stdout).unwrap().trim().parse().unwrap();
    assert_eq!(link.host, gm.public());
    assert_eq!(link.campaign.0, c.id.0);
    let invites = std::fs::read_to_string(dir.join("Seattle.invites")).unwrap();
    assert!(invites.starts_with(&format!("{} player group", link.invite.unwrap())), "{invites}");
    // The host takes the token in.
    let mut auth = chummer_sync::Authority::new(link.campaign, gm.public(), "GM");
    assert_eq!(chummer_sync::hosted::merge_invites(&mut auth, &dir.join("Seattle.invites")), 1);
    assert_eq!(auth.invites().redeem(&link.invite.unwrap()), Some(Role::Player));

    let player = chummer_net::SecretKey::generate().public();
    let out = bin().arg("--key").arg(&key).args(["assign"]).arg(&file).args(["ghost", &player.to_string()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(Campaign::load(&file).unwrap().member(id).unwrap().owner, Some(player.to_string()));
    let out = bin().arg("--key").arg(&key).args(["assign"]).arg(&file).args(["Ghost", "gm"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(Campaign::load(&file).unwrap().member(id).unwrap().owner.as_deref(), Some("gm"));
    let out = bin().arg("--key").arg(&key).args(["assign"]).arg(&file).args(["Nobody", "gm"]).output().unwrap();
    assert!(!out.status.success());

    let out = bin().arg("--key").arg(&key).args(["status"]).arg(&file).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("has not been hosted yet"));
    std::fs::remove_dir_all(&dir).unwrap();
}
