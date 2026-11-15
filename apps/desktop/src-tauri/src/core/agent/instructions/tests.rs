use super::*;

// Existing filesystem fixtures explicitly authorize their synthetic root.
fn read_project_instructions(
    root: &Path,
    cwd: &Path,
    environment: &str,
    fallbacks: &[String],
) -> Result<Vec<InstructionDocument>, InstructionReadError> {
    super::read_project_instructions(root, cwd, environment, fallbacks, |_| Ok(()))
}

#[test]
fn root_to_cwd_override_empty_and_revision_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    fs::create_dir(root.join("chapter")).unwrap();
    fs::write(root.join("AGENTS.md"), "root").unwrap();
    fs::write(root.join("chapter/AGENTS.override.md"), "").unwrap();
    fs::write(root.join("chapter/AGENTS.md"), "must not win").unwrap();
    let documents = read_project_instructions(&root, Path::new("chapter"), "paper", &[]).unwrap();
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].text, "root");
    assert_eq!(documents[0].scope_id, ".");
    assert_eq!(documents[1].scope_id, "chapter");
    assert_eq!(documents[1].source_id, "chapter/AGENTS.override.md");
    assert!(documents[1].text.is_empty());
    assert_eq!(
        documents[0].revision,
        format!("sha256:{:x}", Sha256::digest(b"root"))
    );
    fs::write(root.join("AGENTS.md"), "changed").unwrap();
    let changed = read_project_instructions(&root, Path::new(""), "paper", &[]).unwrap();
    assert_ne!(changed[0].revision, documents[0].revision);
}

#[test]
fn explicit_root_is_a_boundary_and_fallback_encoding_and_budget_are_checked() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("AGENTS.md"), "outside").unwrap();
    fs::create_dir(temp.path().join("project")).unwrap();
    let root = fs::canonicalize(temp.path().join("project")).unwrap();
    assert!(
        read_project_instructions(&root, Path::new(""), "paper", &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        read_project_instructions(&root, Path::new(".."), "paper", &[]),
        Err(InstructionReadError::InvalidScope)
    );
    fs::write(root.join("GUIDE.md"), "fallback").unwrap();
    assert_eq!(
        read_project_instructions(&root, Path::new(""), "paper", &["GUIDE.md".into()]).unwrap()[0]
            .text,
        "fallback"
    );
    fs::write(root.join("AGENTS.md"), [0xff]).unwrap();
    assert_eq!(
        read_project_instructions(&root, Path::new(""), "paper", &[]),
        Err(InstructionReadError::Encoding)
    );
    fs::File::create(root.join("AGENTS.md"))
        .unwrap()
        .set_len(MAX_READ_BYTES as u64 + 1)
        .unwrap();
    assert_eq!(
        read_project_instructions(&root, Path::new(""), "paper", &[]),
        Err(InstructionReadError::Budget)
    );
}

#[cfg(unix)]
#[test]
fn links_in_source_or_scope_cannot_import_outside_instructions() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("AGENTS.md"), "outside").unwrap();
    symlink(outside.path(), root.join("linked")).unwrap();
    assert_eq!(
        read_project_instructions(&root, Path::new("linked"), "paper", &[]),
        Err(InstructionReadError::InvalidScope)
    );
    symlink(outside.path().join("AGENTS.md"), root.join("AGENTS.md")).unwrap();
    assert_eq!(
        read_project_instructions(&root, Path::new(""), "paper", &[]),
        Err(InstructionReadError::InvalidScope)
    );
}

#[test]
fn cumulative_read_limit_spans_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    for path in ["AGENTS.md", "nested/AGENTS.md"] {
        fs::File::create(root.join(path))
            .unwrap()
            .set_len((MAX_READ_BYTES / 2 + 1) as u64)
            .unwrap();
    }
    assert_eq!(
        read_project_instructions(&root, Path::new("nested"), "paper", &[]),
        Err(InstructionReadError::Budget)
    );
}

#[test]
fn pinned_directory_does_not_follow_a_replacement_path() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    fs::create_dir(root.join("scope")).unwrap();
    fs::write(root.join("scope/AGENTS.md"), "authorized").unwrap();
    let directory = Directory::root(&root)
        .unwrap()
        .child(Path::new("scope"))
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("AGENTS.md"), "outside").unwrap();
    fs::rename(root.join("scope"), root.join("moved")).unwrap();
    symlink(outside.path(), root.join("scope")).unwrap();
    let mut text = String::new();
    directory
        .file("AGENTS.md")
        .unwrap()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "authorized");
}

#[test]
fn file_replacement_with_symlink_is_rejected_and_fifo_is_not_read() {
    use nix::{sys::stat::Mode, unistd::mkfifo};
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let directory = Directory::root(&root).unwrap();
    symlink("missing", root.join("AGENTS.md")).unwrap();
    assert!(matches!(
        directory.file("AGENTS.md"),
        Err(InstructionReadError::InvalidScope)
    ));
    fs::remove_file(root.join("AGENTS.md")).unwrap();
    mkfifo(&root.join("AGENTS.md"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    assert!(directory.file("AGENTS.md").unwrap().is_none());
}

#[test]
fn authorization_checks_opened_identity_before_reading_and_pins_that_root() {
    let temp = tempfile::tempdir().unwrap();
    let parent = fs::canonicalize(temp.path()).unwrap();
    let root = parent.join("root");
    fs::create_dir(&root).unwrap();
    // If denial occurs after reading, this malformed content would return Encoding.
    fs::write(root.join("AGENTS.md"), [0xff]).unwrap();
    assert_eq!(
        super::read_project_instructions(&root, Path::new(""), "paper", &[], |_| Err(
            InstructionReadError::Unauthorized
        )),
        Err(InstructionReadError::Unauthorized)
    );
    fs::write(root.join("AGENTS.md"), "original").unwrap();
    let expected = Directory::root(&root).unwrap().identity().unwrap();
    let loaded = super::read_project_instructions(&root, Path::new(""), "paper", &[], |identity| {
        assert_eq!(identity.canonical_path, root);
        assert_eq!((identity.device, identity.inode), expected);
        fs::rename(&root, parent.join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("AGENTS.md"), "replacement").unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(loaded[0].text, "original");
    // A stale grant for the former root must not authorize its replacement.
    assert_eq!(
        super::read_project_instructions(&root, Path::new(""), "paper", &[], |identity| {
            if (identity.device, identity.inode) == expected {
                Ok(())
            } else {
                Err(InstructionReadError::Unauthorized)
            }
        }),
        Err(InstructionReadError::Unauthorized)
    );
}

#[test]
fn multiple_selected_scopes_share_ancestors_and_budget_without_duplicate_instructions() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    for directory in ["a", "a/nested", "b"] {
        fs::create_dir(root.join(directory)).unwrap();
    }
    for directory in ["", "a", "a/nested", "b"] {
        fs::write(root.join(directory).join("AGENTS.md"), directory).unwrap();
    }
    let documents = super::read_project_instruction_scopes(
        &root,
        &[Path::new("a/nested"), Path::new("b"), Path::new("a")],
        "paper",
        &[],
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        documents
            .iter()
            .map(|d| d.scope_id.as_str())
            .collect::<Vec<_>>(),
        vec![".", "a", "a/nested", "b"]
    );
}
