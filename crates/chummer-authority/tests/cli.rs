//! The offline commands: invite create/list/revoke/reissue, assign and
//! status on a campaign file.

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
    let out = bin().args(["invite", "create", "--label", "Anna"]).arg(&file).output().unwrap();
    assert!(!out.status.success());

    let run = |args: &[&str]| {
        let out = bin().arg("--key").arg(&key).args(&args[..2]).arg(&file).args(&args[2..]).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let link: InviteLink = run(&["invite", "create", "--label", "Anna", "--assign", "ghost", "--expires", "7d"]).trim().parse().unwrap();
    assert_eq!(link.host, gm.public());
    assert_eq!(link.campaign.0, c.id.0);
    assert_eq!(link.gm_key, Some(chummer_net::invite::derive_campaign_key(&gm, link.campaign, 0).public()));
    let anna_key = link.member.clone().expect("a member key").public();
    let bert: InviteLink = run(&["invite", "create", "--label", "Bert"]).trim().parse().unwrap();
    assert_ne!(bert.member, link.member, "one key per invite");
    let list = run(&["invite", "list"]);
    assert!(list.contains("Anna") && list.contains("unclaimed, expires") && list.contains("Ghost (on claim)"), "{list}");
    assert!(list.contains("Bert"), "{list}");
    run(&["invite", "revoke", "bert"]);
    assert!(run(&["invite", "list"]).contains("revoked"));
    let again: InviteLink = run(&["invite", "reissue", "Anna"]).trim().parse().unwrap();
    assert_ne!(again.member.as_ref().unwrap().public(), anna_key);
    // Asking about an invite that is not there fails.
    let out = bin().arg("--key").arg(&key).args(["invite", "revoke"]).arg(&file).arg("Nobody").output().unwrap();
    assert!(!out.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("Seattle.invites")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the invites file holds member keys");
    }
    // A host takes the changes in: Anna's re-issued link works, the first
    // one does not, Bert is revoked.
    let mut auth = chummer_sync::Authority::new(link.campaign, gm.public(), "GM");
    assert_eq!(chummer_sync::hosted::merge_invites(&mut auth, &dir.join("Seattle.invites")), 4);
    let node = chummer_net::SecretKey::generate().public();
    assert_eq!(auth.admit(node, link.campaign, Some(anna_key), 0), Err(chummer_net::campaign::DenyReason::Superseded));
    assert_eq!(auth.admit(node, link.campaign, Some(bert.member.unwrap().public()), 0), Err(chummer_net::campaign::DenyReason::Revoked));
    let ok = auth.admit(node, link.campaign, Some(again.member.unwrap().public()), 0).unwrap();
    assert_eq!((ok.role, ok.label.as_str()), (Role::Player, "Anna"));
    chummer_sync::hosted::invite_ops_done(&dir.join("Seattle.invites"));

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
