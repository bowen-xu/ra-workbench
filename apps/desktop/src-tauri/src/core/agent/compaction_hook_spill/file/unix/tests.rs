use super::*;

#[test]
fn replaced_root_cannot_redirect_output_or_publish_a_locator() {
    let temporary = tempfile::tempdir().unwrap();
    let parent = temporary.path().canonicalize().unwrap();
    let root = parent.join("authorized");
    std::fs::create_dir(&root).unwrap();
    let opened = Directory::root(&root).unwrap();
    let retained = parent.join("retained");
    std::fs::rename(&root, &retained).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert!(save_opened(&root, &opened, "output.txt", b"private output").is_err());
    assert!(!root.join("output.txt").exists());
    assert_eq!(
        std::fs::read(retained.join("output.txt")).unwrap(),
        b"private output"
    );
}

#[test]
fn linked_root_and_linked_ancestor_cannot_receive_output() {
    let temporary = tempfile::tempdir().unwrap();
    let parent = temporary.path().canonicalize().unwrap();
    let actual = parent.join("actual");
    std::fs::create_dir(&actual).unwrap();
    std::fs::create_dir(actual.join("child")).unwrap();
    let alias = parent.join("alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    assert!(save(&alias, &alias.join("output.txt"), b"private output").is_err());
    assert!(
        save(
            &alias.join("child"),
            &alias.join("child/output.txt"),
            b"private output"
        )
        .is_err()
    );
    assert!(!actual.join("output.txt").exists());
    assert!(!actual.join("child/output.txt").exists());
}

#[test]
fn publication_is_private_complete_and_never_overwrites_existing_names() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let target = root.join("output.txt");
    save(&root, &target, "中文🙂".repeat(10_000).as_bytes()).unwrap();
    let before = std::fs::read(&target).unwrap();
    assert_eq!(before, "中文🙂".repeat(10_000).as_bytes());
    assert_eq!(std::fs::metadata(&target).unwrap().mode() & 0o777, 0o600);
    assert!(save(&root, &target, b"replacement").is_err());
    assert_eq!(std::fs::read(&target).unwrap(), before);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}
