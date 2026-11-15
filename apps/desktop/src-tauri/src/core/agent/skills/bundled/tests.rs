use super::*;
use crate::agent::skills::{application, files, resources};

#[test]
fn embedded_package_is_discovered_selected_and_resources_are_read_by_existing_ports() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("application-config");
    install_defaults(&root).unwrap();
    let catalog = application::discover(&root).unwrap();
    assert!(catalog.rejected.is_empty());
    assert_eq!(catalog.skills.len(), 3);
    let descriptor = &catalog.skills[0];
    assert_eq!(descriptor.name, "edit-notes");
    assert_eq!(descriptor.identity.environment_id, "application");
    assert_eq!(
        descriptor.identity.source_id,
        ".agents/skills/edit-notes/SKILL.md"
    );
    let selected = application::freeze(&root, "$edit-notes").unwrap();
    assert_eq!(selected.iter().filter(|skill| skill.explicit).count(), 1);
    assert!(selected[0].explicit);
    let root = root.canonicalize().unwrap();
    let loaded = files::load_selected(&root, descriptor, true, 1_048_576, |_| Ok(())).unwrap();
    assert!(loaded.body.contains("research_apply_patch"));
    let resource = resources::load_resource(
        &root,
        descriptor,
        true,
        &resources::ResourceRequest {
            identity: descriptor.identity.clone(),
            path: "references/text-view.md".into(),
            expected_revision: None,
        },
        1_048_576,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(resource.text, PROTOCOL);
    // Reload is idempotent and retains the catalog identity.
    install_defaults(&root).unwrap();
    assert_eq!(
        application::discover(&root).unwrap().skills[0].identity,
        descriptor.identity
    );
}

#[test]
fn existing_package_is_never_overwritten_and_disabled_policy_is_preserved() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let package = root.join(".agents/skills/edit-notes");
    fs::create_dir_all(&package).unwrap();
    let user_skill =
        "---\nname: edit-notes\ndescription: My customized skill\n---\nMy instructions\n";
    fs::write(package.join("SKILL.md"), user_skill).unwrap();
    crate::agent::config::skills::set_enabled(root, ".agents/skills/edit-notes/SKILL.md", false)
        .unwrap();
    install_defaults(root).unwrap();
    assert_eq!(
        fs::read_to_string(package.join("SKILL.md")).unwrap(),
        user_skill
    );
    assert!(!package.join("references").exists());
    assert!(
        !application::freeze(root, "$edit-notes")
            .unwrap()
            .iter()
            .any(|skill| skill.explicit)
    );
}

#[test]
fn removal_is_not_reversed_on_restart() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    install_defaults(root).unwrap();
    fs::remove_dir_all(root.join(".agents/skills/edit-notes")).unwrap();
    install_defaults(root).unwrap();
    assert!(
        !application::discover(root)
            .unwrap()
            .skills
            .iter()
            .any(|skill| skill.name == "edit-notes")
    );
}

#[test]
fn existing_same_name_at_another_source_does_not_become_ambiguous() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let package = root.join(".agents/skills/custom-notes");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        "---\nname: edit-notes\ndescription: Customized notes\n---\nCustom\n",
    )
    .unwrap();
    install_defaults(root).unwrap();
    assert!(!root.join(".agents/skills/edit-notes").exists());
    let selected = application::freeze(root, "$edit-notes").unwrap();
    let explicit = selected
        .iter()
        .filter(|skill| skill.explicit)
        .collect::<Vec<_>>();
    assert_eq!(explicit.len(), 1);
    assert_eq!(
        selected
            .iter()
            .filter(|skill| skill.descriptor.name == "edit-notes")
            .count(),
        1
    );
    assert_eq!(
        explicit[0].descriptor.identity.source_id,
        ".agents/skills/custom-notes/SKILL.md"
    );
}

#[cfg(unix)]
#[test]
fn linked_installation_boundary_is_rejected_without_touching_the_destination() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join(".agents")).unwrap();
    symlink(outside.path(), temporary.path().join(".agents/skills")).unwrap();
    assert!(install_defaults(temporary.path()).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn structured_package_has_independent_removal_receipt_and_versioned_references() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    install_defaults(root).unwrap();
    let catalog = application::discover(root).unwrap();
    let descriptor = catalog
        .skills
        .iter()
        .find(|skill| skill.name == "edit-structured-resources")
        .unwrap();
    let canonical = root.canonicalize().unwrap();
    for path in ["portrait.md", "reading.md", "templates.md"] {
        let reference = resources::load_resource(
            &canonical,
            descriptor,
            true,
            &resources::ResourceRequest {
                identity: descriptor.identity.clone(),
                path: format!("references/{path}"),
                expected_revision: None,
            },
            1_048_576,
            |_| Ok(()),
        )
        .unwrap();
        assert!(!reference.text.is_empty());
    }
    fs::remove_dir_all(root.join(".agents/skills/edit-structured-resources")).unwrap();
    install_defaults(root).unwrap();
    assert!(
        !application::discover(root)
            .unwrap()
            .skills
            .iter()
            .any(|skill| skill.name == "edit-structured-resources")
    );
    assert!(root.join(".agents/skills/edit-notes").exists());
}
