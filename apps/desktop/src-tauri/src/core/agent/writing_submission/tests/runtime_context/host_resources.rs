use super::*;
use crate::agent::{
    instructions::InstructionRootIdentity, repository::instruction_trust, resource_catalog,
    skill_document_commands,
};
use research_agent_core::user_input::{MentionTarget, UserInput};
use std::{fs, os::unix::fs::MetadataExt};

fn scope(f: &Fixture) -> resource_catalog::Request {
    resource_catalog::Request {
        conversation_id: f.conversation,
        kind: "manuscript".into(),
        resource_id: f.manuscript.to_string(),
    }
}
fn select(f: &mut Fixture, binding: Binding) {
    f.store = f
        .store
        .clone()
        .without_static_writing_context_sources()
        .with_native_writing_context(binding);
    assert!(f.store.writing_resource_source().is_none());
}
fn add_file(f: &Fixture, path: &str, kind: &str) {
    f.store.connection().unwrap().execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,?3,1,?4,'frozen-time')", rusqlite::params![f.manuscript.to_string(),path,kind,f.hash]).unwrap();
}
fn trust(f: &Fixture, expected: u64, granted: bool) -> PathBuf {
    let root = f.store.writing_source_directory().unwrap()(
        f.store.manuscript_directory(),
        &f.manuscript.to_string(),
    )
    .unwrap()
    .canonicalize()
    .unwrap();
    let metadata = fs::metadata(&root).unwrap();
    instruction_trust::update(
        &mut f.store.connection().unwrap(),
        f.manuscript,
        expected,
        &InstructionRootIdentity {
            canonical_path: &root,
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        granted,
    )
    .unwrap();
    root
}
fn write_skill(root: &Path) -> PathBuf {
    let directory = root.join(".agents/skills/review");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("SKILL.md");
    fs::write(
        &file,
        "---\ndescription: Review prose\n---\nOriginal skill body",
    )
    .unwrap();
    file
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_host_inventory_preserves_mentions_sort_skill_identity_and_read_only_owner() {
    let mut f = fixture(true);
    let binding = binding(packages());
    select(&mut f, binding.clone());
    add_file(&f, "Z.tex", "tex");
    add_file(&f, "a.png", "image");
    let root = trust(&f, 0, true);
    let skill = write_skill(&root);
    let changes = f.store.connection().unwrap().total_changes();
    let resources = resource_catalog::list_current(f.store.clone(), scope(&f))
        .await
        .unwrap();
    assert_eq!(
        resources
            .iter()
            .filter(|item| item.group == "files")
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["main.tex", "Z.tex"]
    );
    let UserInput::Mention {
        target:
            MentionTarget::File {
                resource_id,
                revision,
                source_url,
                ..
            },
        ..
    } = &resources[0].input
    else {
        panic!()
    };
    assert_eq!(resource_id, "main.tex");
    assert_eq!(revision, &f.hash);
    assert!(source_url.as_ref().unwrap().ends_with("/main.tex"));
    let identity = resources
        .iter()
        .find_map(|item| match &item.input {
            UserInput::Skill { identity, .. } => Some(identity.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        identity.environment_id,
        format!("manuscript:{}", f.manuscript)
    );
    assert_eq!(
        skill_document_commands::read_current(f.store.clone(), identity.clone(), Some(scope(&f)))
            .await
            .unwrap(),
        "Original skill body"
    );
    assert_eq!(f.store.connection().unwrap().total_changes(), changes);
    assert!(f.store.connection().unwrap().is_autocommit());
    fs::write(
        &skill,
        "---\ndescription: Review prose\n---\nChanged skill body",
    )
    .unwrap();
    assert!(matches!(
        skill_document_commands::read_current(f.store.clone(), identity.clone(), Some(scope(&f)))
            .await,
        Err(skill_document_commands::Error::InvalidSelection)
    ));
    fs::write(
        &skill,
        "---\ndescription: Review prose\n---\nOriginal skill body",
    )
    .unwrap();
    trust(&f, 1, false);
    assert!(matches!(
        skill_document_commands::read_current(f.store.clone(), identity, Some(scope(&f))).await,
        Err(skill_document_commands::Error::InvalidSelection)
    ));
    binding.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_host_inventory_counts_images_before_the_original_limit() {
    let mut f = fixture(true);
    let binding = binding(packages());
    select(&mut f, binding.clone());
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from manuscript_files where manuscript_id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    for index in 0..257 {
        add_file(&f, &format!("image-{index:03}.png"), "image");
    }
    assert!(matches!(
        resource_catalog::list_current(f.store.clone(), scope(&f)).await,
        Err(resource_catalog::Error::Invalid)
    ));
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from manuscript_files where manuscript_id=?1 and relative_path='image-256.png'",
            [f.manuscript.to_string()],
        )
        .unwrap();
    assert!(
        resource_catalog::list_current(f.store.clone(), scope(&f))
            .await
            .unwrap()
            .is_empty()
    );
    binding.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn host_inventory_rejects_invalid_unbound_scope_before_loading_an_unavailable_package() {
    let mut f = fixture(true);
    let catalog = Arc::new(RuntimePluginCatalog::new(None));
    let binding = Binding {
        service: Arc::new(NativePreviewService::new(Arc::clone(&catalog))),
        catalog,
        package_key: "a".repeat(64),
    };
    select(&mut f, binding);
    let mut request = scope(&f);
    request.conversation_id = Uuid::nil();
    assert!(matches!(
        resource_catalog::list_current(f.store.clone(), request).await,
        Err(resource_catalog::Error::Invalid)
    ));
    let mut request = scope(&f);
    request.resource_id = Uuid::new_v4().to_string();
    assert!(matches!(
        resource_catalog::list_current(f.store.clone(), request).await,
        Err(resource_catalog::Error::Unbound)
    ));
    let error = match resource_catalog::list_current(f.store.clone(), scope(&f)).await {
        Err(error) => error,
        Ok(_) => panic!("unavailable package must fail"),
    };
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["code"], "plugin_host_package_unavailable");
    assert_eq!(
        serde_json::to_value(resource_catalog::Error::Invalid).unwrap(),
        "invalid"
    );
    assert_eq!(
        serde_json::to_value(resource_catalog::Error::Unbound).unwrap(),
        "unbound"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_host_inventory_rechecks_binding_full_rows_and_skill_trust_after_native_wait() {
    for action in ["binding", "file", "trust"] {
        let mut f = fixture(true);
        let (binding, pause) = super::pause::paused_host_files_binding(f.root.path());
        select(&mut f, binding.clone());
        let root = trust(&f, 0, true);
        write_skill(&root);
        let store = f.store.clone();
        let request = scope(&f);
        let listing =
            tokio::spawn(async move { resource_catalog::list_current(store, request).await });
        pause.entered().await;
        let store = f.store.clone();
        let m = f.manuscript;
        let c = f.conversation;
        let root = root.clone();
        tokio::time::timeout(Duration::from_secs(20), tokio::task::spawn_blocking(move || {
            let mut db = store.connection().unwrap();
            assert!(db.is_autocommit());
            match action {
                "binding" => {db.execute("delete from agent_conversation_bindings where conversation_id=?1",[c.to_string()]).unwrap();},
                "file" => {db.execute("update manuscript_files set byte_size=byte_size+1 where manuscript_id=?1",[m.to_string()]).unwrap();},
                "trust" => {
                    let metadata = fs::metadata(&root).unwrap();
                    instruction_trust::update(&mut db, m, 1, &InstructionRootIdentity {canonical_path: &root, device: metadata.dev(), inode: metadata.ino()}, false).unwrap();
                },
                _ => unreachable!(),
            }
        })).await.unwrap().unwrap();
        pause.release();
        let result = tokio::time::timeout(Duration::from_secs(20), listing)
            .await
            .unwrap()
            .unwrap();
        match action {
            "binding" => assert!(matches!(result, Err(resource_catalog::Error::Unbound))),
            "file" => {
                let Err(resource_catalog::Error::Native(error)) = result else {
                    panic!()
                };
                assert_eq!(error.code, "plugin_data_snapshot_changed");
                assert_eq!(
                    error.data_error.unwrap().resource_id,
                    "writing.projects.files"
                );
                assert!(error.instance_id.is_some() && error.request_id.is_some());
            }
            "trust" => assert!(result.unwrap().iter().all(|item| item.group != "skills")),
            _ => unreachable!(),
        }
        binding.service.shutdown().await.unwrap();
    }
}
