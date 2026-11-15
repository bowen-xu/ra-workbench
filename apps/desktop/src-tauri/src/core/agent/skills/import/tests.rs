use super::*;
fn skill(root: &Path, name: &str, body: &str) {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("scripts")).unwrap();
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Import test\n---\n{body}"),
    )
    .unwrap();
    fs::write(directory.join("scripts/run.sh"), "echo hello").unwrap();
    fs::create_dir_all(directory.join("assets")).unwrap();
    fs::write(directory.join("assets/sample.txt"), "asset").unwrap();
}
#[tokio::test]
async fn multi_select_copies_complete_packages_and_requires_explicit_update() {
    let source = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    skill(source.path(), "one", "old");
    skill(source.path(), "two", "second");
    let preview = scan(
        app.path().to_owned(),
        Source::Local {
            directory: source.path().to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(preview.candidates.len(), 2);
    let selections = preview
        .candidates
        .iter()
        .map(|candidate| Selection {
            source_id: candidate.source_id.clone(),
            expected_existing_revision: None,
        })
        .collect();
    install(app.path(), preview.import_id, selections).unwrap();
    assert_eq!(
        fs::read_to_string(app.path().join(".agents/skills/one/assets/sample.txt")).unwrap(),
        "asset"
    );
    assert!(
        app.path()
            .join(".agents/skills/two/scripts/run.sh")
            .is_file()
    );
    skill(source.path(), "one", "updated");
    let preview = scan(
        app.path().to_owned(),
        Source::Local {
            directory: source.path().to_owned(),
        },
    )
    .await
    .unwrap();
    let one = preview
        .candidates
        .iter()
        .find(|item| item.name == "one")
        .unwrap();
    assert!(matches!(
        install(
            app.path(),
            preview.import_id,
            vec![Selection {
                source_id: one.source_id.clone(),
                expected_existing_revision: None
            }]
        ),
        Err(Error::Conflict)
    ));
    install(
        app.path(),
        preview.import_id,
        vec![Selection {
            source_id: one.source_id.clone(),
            expected_existing_revision: one.existing_revision.clone(),
        }],
    )
    .unwrap();
    assert!(
        fs::read_to_string(app.path().join(".agents/skills/one/SKILL.md"))
            .unwrap()
            .contains("updated")
    );
}
#[tokio::test]
async fn stale_revision_and_invalid_batch_leave_existing_package_intact() {
    let source = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    skill(source.path(), "one", "initial");
    let preview = scan(
        app.path().to_owned(),
        Source::Local {
            directory: source.path().to_owned(),
        },
    )
    .await
    .unwrap();
    install(
        app.path(),
        preview.import_id,
        vec![Selection {
            source_id: preview.candidates[0].source_id.clone(),
            expected_existing_revision: None,
        }],
    )
    .unwrap();
    skill(source.path(), "one", "updated");
    let preview = scan(
        app.path().to_owned(),
        Source::Local {
            directory: source.path().to_owned(),
        },
    )
    .await
    .unwrap();
    fs::write(
        app.path().join(".agents/skills/one/SKILL.md"),
        "---\nname: one\ndescription: Changed\n---\nconcurrent edit",
    )
    .unwrap();
    assert!(matches!(
        install(
            app.path(),
            preview.import_id,
            vec![Selection {
                source_id: preview.candidates[0].source_id.clone(),
                expected_existing_revision: preview.candidates[0].existing_revision.clone()
            }]
        ),
        Err(Error::Conflict)
    ));
    assert!(
        fs::read_to_string(app.path().join(".agents/skills/one/SKILL.md"))
            .unwrap()
            .contains("concurrent edit")
    );
    cancel(preview.import_id).unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn rejects_linked_assets_and_linked_install_roots() {
    let source = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(source.path(), "one", "body");
    std::os::unix::fs::symlink(outside.path(), source.path().join("one/assets/link")).unwrap();
    assert!(matches!(
        scan(
            app.path().to_owned(),
            Source::Local {
                directory: source.path().to_owned()
            }
        )
        .await,
        Err(Error::Invalid)
    ));
    std::os::unix::fs::symlink(outside.path(), app.path().join(".agents/skills/link")).unwrap();
    assert!(
        checked_child(
            &app.path().join(".agents/skills"),
            Path::new("link/subdirectory")
        )
        .is_err()
    );
}
#[tokio::test]
async fn git_source_rejects_credentials_non_https_and_invalid_refs_before_fetch() {
    let app = tempfile::tempdir().unwrap();
    for url in [
        "file:///tmp/example",
        "https://user:password@example.com/repo",
        "https://example.com/repo?token=private",
    ] {
        assert!(matches!(
            scan(
                app.path().to_owned(),
                Source::Git {
                    url: url.into(),
                    reference: None,
                    subdirectory: None
                }
            )
            .await,
            Err(Error::Invalid)
        ));
    }
    assert!(matches!(
        scan(
            app.path().to_owned(),
            Source::Git {
                url: "https://example.com/repo".into(),
                reference: Some("--invalid".into()),
                subdirectory: None
            }
        )
        .await,
        Err(Error::Invalid)
    ));
}
