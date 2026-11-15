use super::*;
use crate::agent::skills::files::read_descriptor;
use std::{fs, os::unix::fs::symlink};

fn fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    SkillDescriptor,
    ResourceRequest,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("review/references")).unwrap();
    fs::write(
        root.join("review/SKILL.md"),
        "---\ndescription: Review\n---\nRead references/rules.md",
    )
    .unwrap();
    fs::write(root.join("review/references/rules.md"), "引文必须保留。\n").unwrap();
    let skill =
        read_descriptor(&root, Path::new("review/SKILL.md"), "project", |_| Ok(())).unwrap();
    let request = ResourceRequest {
        identity: skill.identity.clone(),
        path: "references/rules.md".into(),
        expected_revision: None,
    };
    (temp, root, skill, request)
}
#[test]
fn resource_revision_is_independent_and_rereads_can_require_it() {
    let (_temp, root, skill, mut request) = fixture();
    let loaded = load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).unwrap();
    assert_eq!(loaded.text, "引文必须保留。\n");
    assert_eq!(loaded.identity, skill.identity);
    assert_eq!(loaded.path, request.path);
    request.expected_revision = Some(loaded.revision);
    assert!(load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).is_ok());
    fs::write(root.join("review/references/rules.md"), "Changed").unwrap();
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1024, |_| Ok(())),
        Err(SkillError::Stale.into())
    );
    request.expected_revision = None;
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1024, |_| Ok(()))
            .unwrap()
            .text,
        "Changed"
    );
    fs::write(
        root.join("review/SKILL.md"),
        "---\ndescription: Review\n---\nChanged skill",
    )
    .unwrap();
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1024, |_| Ok(())),
        Err(SkillError::Stale.into())
    );
}
#[test]
fn path_and_policy_rejection_happen_before_authorization_or_resource_access() {
    let (_temp, root, mut skill, mut request) = fixture();
    for path in [
        "",
        "/etc/passwd",
        "../rules.md",
        "references/../SKILL.md",
        "references//rules.md",
        "./SKILL.md",
        "C:\\rules.md",
        "https://example.org",
        "rules.md\0",
        "references/",
    ] {
        request.path = path.into();
        assert!(
            load_resource(&root, &skill, true, &request, 1024, |_| panic!(
                "invalid path reached filesystem"
            ))
            .is_err()
        );
    }
    request.path = "references/rules.md".into();
    skill.allow_implicit_invocation = false;
    assert_eq!(
        load_resource(&root, &skill, false, &request, 1024, |_| panic!()),
        Err(SkillError::ExplicitRequired.into())
    );
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1024, |_| Err(
            InstructionReadError::Unauthorized
        )),
        Err(InstructionReadError::Unauthorized.into())
    );
}
#[test]
fn resource_files_are_bounded_regular_utf8_and_cannot_follow_links() {
    let (_temp, root, skill, mut request) = fixture();
    let path = root.join("review/references/rules.md");
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1, |_| Ok(())),
        Err(SkillError::Budget.into())
    );
    for bytes in [vec![0xff], vec![0]] {
        fs::write(&path, bytes).unwrap();
        assert!(load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).is_err());
    }
    fs::remove_file(&path).unwrap();
    assert_eq!(
        load_resource(&root, &skill, true, &request, 1024, |_| Ok(())),
        Err(SkillReadError::NotFound)
    );
    symlink(root.join("review/SKILL.md"), &path).unwrap();
    assert!(load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).is_err());
    symlink(root.join("review/references"), root.join("review/linked")).unwrap();
    request.path = "linked/rules.md".into();
    assert!(load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).is_err());
    request.path = "pipe".into();
    nix::unistd::mkfifo(&root.join("review/pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
    assert!(load_resource(&root, &skill, true, &request, 1024, |_| Ok(())).is_err());
}
#[test]
fn authorized_root_replacement_does_not_redirect_resource_reads() {
    let (temp, root, skill, request) = fixture();
    let moved = temp.path().with_extension("pinned-skill-root");
    let result = load_resource(&root, &skill, true, &request, 1024, |_| {
        fs::rename(&root, &moved).unwrap();
        fs::create_dir_all(root.join("review/references")).unwrap();
        fs::write(root.join("review/references/rules.md"), "replacement").unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(result.text, "引文必须保留。\n");
    fs::remove_dir_all(moved).unwrap();
}
