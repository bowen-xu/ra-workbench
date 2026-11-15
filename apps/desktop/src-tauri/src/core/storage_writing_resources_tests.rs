use super::*;
use crate::agent::{
    context_session::{
        Error, group_scope,
        sources::{Access, Part, Resource},
    },
    resource_catalog,
};
use uuid::Uuid;

fn store() -> (tempfile::TempDir, AppStore, Uuid, Uuid) {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table agent_conversations (id text, archived integer); create table agent_conversation_bindings (conversation_id text, resource_kind text, resource_id text); create table manuscripts (id text, status text); create table manuscript_files (manuscript_id text, relative_path text, file_kind text, byte_size integer, sha256 text, modified_at text);").unwrap();
    let conversation = Uuid::new_v4();
    let manuscript = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations values (?1,0)",
        [conversation.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into agent_conversation_bindings values (?1,'manuscript',?2)",
        rusqlite::params![conversation.to_string(), manuscript.to_string()],
    )
    .unwrap();
    db.execute(
        "insert into manuscripts values (?1,'active')",
        [manuscript.to_string()],
    )
    .unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    (root, store, conversation, manuscript)
}

fn scope(manuscript: Uuid) -> Resource {
    Resource::Manuscript {
        resource_id: manuscript.to_string(),
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: manuscript,
                path: "main.tex".into(),
            },
            label: "Main".into(),
            selected: true,
            access: Access::ReadOnly,
        }],
        group_access: Some(Access::ReadWrite),
        new_member_access: Some(Access::ReadWrite),
    }
}

#[test]
fn native_directory_uses_original_core_locator_without_static_resource_callbacks() {
    use crate::runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    };
    let (root, store, _, _) = store();
    let store = store.without_static_writing_context_sources();
    assert!(store.writing_source_directory().is_none());
    let catalog = Arc::new(RuntimePluginCatalog::new(None));
    let binding = Binding {
        service: Arc::new(NativePreviewService::new(Arc::clone(&catalog))),
        catalog,
        package_key: "a".repeat(64),
    };
    let store = store.with_native_writing_context(binding.clone());
    for store in [&store, &store.clone()] {
        assert!(store.writing_resource_source().is_none());
        assert!(Arc::ptr_eq(
            &store.native_writing_context().unwrap().service,
            &binding.service
        ));
        let locate = store.writing_source_directory().unwrap();
        let container = root.path().join("not-created");
        for id in [
            "AAAAAAAA-1111-4111-8111-111111111111",
            "00000000-0000-0000-0000-000000000000",
        ] {
            assert_eq!(
                locate(&container, id).unwrap(),
                container.join(id).join("source")
            );
        }
        for id in ["", "not-a-uuid", " AAAAAAAA-1111-4111-8111-111111111111 "] {
            assert_eq!(locate(&container, id).unwrap_err(), "稿件标识无效");
        }
        assert!(!container.exists());
    }
}

#[test]
fn missing_provider_fails_closed_without_overriding_explicit_scope_rules() {
    let (_root, mut store, conversation, manuscript) = store();
    store.writing_resource_source = None;
    let resource = scope(manuscript);
    let db = store.connection().unwrap();
    assert!(matches!(
        group_scope::parts(&db, &resource, None),
        Err(Error::Unavailable)
    ));
    assert!(group_scope::path_access(&resource, "main.tex", false, None));
    assert!(!group_scope::path_access(&resource, "main.tex", true, None));
    assert!(!group_scope::path_access(&resource, "new.tex", false, None));
    // Unrelated note validation needs no writing provider or table access.
    let note = Uuid::new_v4();
    let part = Part {
        resource: Resource::Note { resource_id: note },
        label: "Note".into(),
        selected: true,
        access: Access::ReadWrite,
    };
    let owner = Resource::NoteGroup {
        resource_id: note.to_string(),
        parts: vec![part.clone()],
        group_access: Some(Access::ReadWrite),
        new_member_access: Some(Access::ReadWrite),
    };
    assert!(group_scope::parts(&db, &owner, None).is_ok());
    drop(db);
    let request = resource_catalog::Request {
        conversation_id: conversation,
        kind: "manuscript".into(),
        resource_id: manuscript.to_string(),
    };
    assert!(matches!(
        resource_catalog::list(&store, &request),
        Err(resource_catalog::Error::Unavailable)
    ));
    let other = resource_catalog::Request {
        resource_id: Uuid::new_v4().to_string(),
        ..request
    };
    assert!(matches!(
        resource_catalog::list(&store, &other),
        Err(resource_catalog::Error::Unbound)
    ));
}

#[test]
fn host_file_budget_precedes_image_filter_and_file_identity_revision_are_unchanged() {
    use research_agent_core::user_input::{MentionTarget, UserInput};
    use sha2::{Digest, Sha256};
    let (_root, store, conversation, manuscript) = store();
    {
        let db = store.connection().unwrap();
        for index in 0..256 {
            db.execute(
                "insert into manuscript_files values (?1,?2,'image',1,'image-hash','now')",
                rusqlite::params![manuscript.to_string(), format!("image-{index}.png")],
            )
            .unwrap();
        }
        db.execute(
            "insert into manuscript_files values (?1,'main.tex','tex',1,'revision-hash','now')",
            [manuscript.to_string()],
        )
        .unwrap();
    }
    let request = resource_catalog::Request {
        conversation_id: conversation,
        kind: "manuscript".into(),
        resource_id: manuscript.to_string(),
    };
    assert!(matches!(
        resource_catalog::list(&store, &request),
        Err(resource_catalog::Error::Invalid)
    ));
    store
        .connection()
        .unwrap()
        .execute(
            "delete from manuscript_files where relative_path='image-0.png'",
            [],
        )
        .unwrap();
    let resources = resource_catalog::list(&store, &request).unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].label, "main.tex");
    let UserInput::Mention {
        target:
            MentionTarget::File {
                source_id,
                resource_id,
                source_url,
                revision,
            },
        ..
    } = &resources[0].input
    else {
        unreachable!()
    };
    assert_eq!(
        source_id,
        &format!(
            "writing:{:x}",
            Sha256::digest(format!("{manuscript}/main.tex").as_bytes())
        )
    );
    assert_eq!(resource_id, "main.tex");
    assert_eq!(revision, "revision-hash");
    assert!(
        source_url
            .as_deref()
            .unwrap()
            .contains(&manuscript.to_string())
    );
}
