//! Same managed configuration owner and original faults with an actual package.
use super::*;
use crate::{
    agent::{
        context_session::{
            configuration, group_scope,
            sources::{Access, Part, Resource},
        },
        writing_resources::{WritingScope, WritingScopeReader},
    },
    runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    },
    storage::AppStore,
};
use std::sync::Arc;

fn binding() -> Binding {
    let root = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
            .expect("actual independent writing package collection"),
    )
    .unwrap();
    assert!(
        root.starts_with(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .unwrap()
                .join(".tmp")
        )
    );
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root)));
    let packages = catalog.discover().unwrap();
    assert_eq!(packages.packages.len(), 1);
    Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog,
        package_key: packages.packages[0].package_key.clone(),
    }
}
fn manuscript(db: &SharedDatabase, intent: &mut ContextConfigurationIntent) -> Uuid {
    let id = Uuid::new_v4();
    let sql = db.lock().unwrap();
    sql.execute("insert into research_projects(id,title,summary,created_at) values('p','Synthetic','','now')",[]).unwrap();
    sql.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values(?1,'p','Synthetic','now','now')",[id.to_string()]).unwrap();
    for (path, kind) in [
        ("main.tex", "tex"),
        ("refs.bib", "bib"),
        ("class.cls", "style"),
        ("figure.pdf", "image"),
        ("legacy.ranote", "tex"),
    ] {
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,?3,0,?4,'now')",params![id.to_string(),path,kind,"a".repeat(64)]).unwrap();
    }
    let resource = Resource::Manuscript {
        resource_id: id.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadWrite),
        parts: vec![],
    };
    intent.state = json!({"schemaVersion":1,"persistent":[{"label":"Synthetic","resource":resource}],"temporary":[]});
    id
}
fn resource(intent: &ContextConfigurationIntent) -> Resource {
    serde_json::from_value(intent.state["persistent"][0]["resource"].clone()).unwrap()
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_scope_saves_original_configuration_and_reads_live_members_with_original_permissions()
 {
    let (root, db, mut intent) = fixture();
    let id = manuscript(&db, &mut intent);
    let scope = binding().scope_resources().await;
    scope.as_ref().unwrap();
    let store = AppStore::for_test_database(db.clone(), root.path().join("objects"))
        .without_static_writing_context_sources()
        .with_native_writing_context(binding());
    assert!(store.writing_resource_source().is_none());
    let proof = configuration::save_configuration_proven(&store, intent.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(proof).unwrap()["operationId"],
        json!(intent.operation_id)
    );
    let replay = db
        .save_context_configuration_with_scope(&intent, Some(WritingScope::Native(&scope)))
        .unwrap();
    assert_eq!(replay.operation_id, intent.operation_id);
    let mut sql = db.lock().unwrap();
    assert_eq!(
        configuration::read_in(&sql, intent.conversation_id)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    let group = resource(&intent);
    let parts =
        group_scope::parts_with_scope(&sql, &group, Some(WritingScope::Native(&scope))).unwrap();
    assert_eq!(
        parts.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
        ["class.cls", "legacy.ranote", "main.tex", "refs.bib"]
    );
    let tx = sql.transaction().unwrap();
    tx.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'sections/new.tex','tex',0,?2,'now')",params![id.to_string(),"b".repeat(64)]).unwrap();
    assert!(
        scope
            .list_text_paths(&tx, &id.to_string())
            .unwrap()
            .contains(&"sections/new.tex".into())
    );
    tx.rollback().unwrap();
    assert!(
        !scope
            .list_text_paths(&sql, &id.to_string())
            .unwrap()
            .contains(&"sections/new.tex".into())
    );
    for path in [
        "new.TEX",
        "nested/ref.BIB",
        "nested/style.CLS",
        "style.bbx",
        "style.cbx",
    ] {
        assert!(
            group_scope::path_access_with_scope(
                &group,
                path,
                true,
                Some(WritingScope::Native(&scope))
            )
            .unwrap()
        );
    }
    for path in ["image.pdf", "other.ranote", "../escape.tex", ".git/config"] {
        assert!(
            !group_scope::path_access_with_scope(
                &group,
                path,
                true,
                Some(WritingScope::Native(&scope))
            )
            .unwrap()
        );
    }
    let Resource::Manuscript { resource_id, .. } = group else {
        panic!("manuscript")
    };
    let protected = Resource::Manuscript {
        resource_id,
        new_member_access: Some(Access::ReadWrite),
        group_access: Some(Access::ReadOnly),
        parts: vec![Part {
            resource: Resource::ManuscriptFile {
                resource_id: id,
                path: "main.tex".into(),
            },
            label: "main.tex".into(),
            access: Access::ReadWrite,
            selected: false,
        }],
    };
    assert!(
        !group_scope::path_access_with_scope(
            &protected,
            "main.tex",
            false,
            Some(WritingScope::Native(&scope))
        )
        .unwrap()
    );
    assert!(
        !group_scope::path_access_with_scope(
            &protected,
            "new.tex",
            true,
            Some(WritingScope::Native(&scope))
        )
        .unwrap()
    );
    drop(sql);
    let mut stale = intent.clone();
    stale.operation_id = Uuid::new_v4();
    assert!(matches!(
        db.save_context_configuration_with_scope(&stale, Some(WritingScope::Native(&scope))),
        Err(ContextConfigurationFailure::Domain(
            Error::ConfirmationMismatch
        ))
    ));
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_scope_keeps_original_validation_precedence_and_diagnostic_without_committing() {
    let (_root, db, mut intent) = fixture();
    manuscript(&db, &mut intent);
    let mut bad_binding = binding();
    bad_binding.package_key = "f".repeat(64);
    let scope = bad_binding.scope_resources().await;
    assert!(scope.is_err());
    let mut invalid = intent.clone();
    invalid.state["persistent"][0]["label"] = json!("");
    assert!(matches!(
        db.save_context_configuration_with_scope(&invalid, Some(WritingScope::Native(&scope))),
        Err(ContextConfigurationFailure::Domain(Error::InvalidInput))
    ));
    let error = db
        .save_context_configuration_with_scope(&intent, Some(WritingScope::Native(&scope)))
        .unwrap_err();
    let ContextConfigurationFailure::Domain(Error::NativeCatalogUnavailable(error)) = error else {
        panic!("native diagnostic")
    };
    assert_eq!(error.package_key, bad_binding.package_key);
    assert_eq!(error.phase, "admission");
    assert!(
        configuration::read_in(&db.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        db.context_configuration_status(&intent).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    let good = binding();
    db.save_context_configuration_with_scope(
        &intent,
        Some(WritingScope::Native(&good.scope_resources().await)),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_scope_preserves_uncertain_commit_and_original_receipt_after_reopen() {
    let (root, db, mut intent) = fixture();
    manuscript(&db, &mut intent);
    let scope = binding().scope_resources().await;
    scope.as_ref().unwrap();
    assert!(matches!(
        db.save_context_configuration_owned_with_scope(
            &intent,
            super::super::super::Fault::AfterCommit,
            Some(WritingScope::Native(&scope))
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unknown(_)
        ))
    ));
    drop(db);
    let reopened = open(root.path());
    let receipt = reopened
        .save_context_configuration_with_scope(&intent, Some(WritingScope::Native(&scope)))
        .unwrap();
    assert_eq!(receipt.operation_id, intent.operation_id);
    assert_eq!(
        configuration::read_in(&reopened.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert!(matches!(
        reopened.context_configuration_status(&intent).unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
}
