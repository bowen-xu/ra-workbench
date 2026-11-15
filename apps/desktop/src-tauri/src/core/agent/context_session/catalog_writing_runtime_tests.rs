//! Directory admission is separate from later configuration and file authority.
use super::*;
use crate::{
    agent::context_catalog::{ContextCatalogReadError, ManuscriptCatalogReader},
    runtime_plugins::{
        RuntimePluginCatalog, native::NativePreviewService, writing_projects::context::Binding,
    },
};
use rusqlite::params;
use sources::Resource;
use std::{path::Path, sync::Arc};

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
fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values('p','Synthetic','','now')",[]).unwrap();
    db
}
fn manuscript(db: &Connection, index: usize) -> String {
    let id = Uuid::from_u128(index as u128 + 1).to_string();
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values(?1,'p',?2,'now',?3)",
        params![id,format!("Synthetic Needle {index:02}"),format!("{index:03}")]).unwrap();
    for (path, kind) in [
        ("main.tex", "tex"),
        ("refs.bib", "bib"),
        ("class.cls", "style"),
        ("image.pdf", "image"),
        ("data.txt", "other"),
    ] {
        db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,?3,0,?4,'now')",
            params![id,path,kind,"a".repeat(64)]).unwrap();
    }
    id
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_package_manuscript_directory_preserves_original_inventory_and_live_snapshot() {
    let binding = binding();
    let resources = binding.context_catalog_resources().await.unwrap();
    let db = database();
    for index in 0..65 {
        manuscript(&db, index);
    }
    db.execute(
        "update manuscripts set status='archived' where id=?1",
        [Uuid::from_u128(65).to_string()],
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let store = store
        .without_static_writing_context_sources()
        .with_native_writing_context(binding.clone());
    assert!(store.writing_resource_source().is_none());
    assert!(store.context_catalog_source().search_manuscripts.is_none());
    let actual = search_catalog(&store, "Needle".into()).await.unwrap();
    assert_eq!(actual.len(), 60);
    assert_eq!(
        actual
            .iter()
            .map(|selection| match &selection.resource {
                Resource::Manuscript { resource_id, .. } => {
                    (resource_id.clone(), selection.label.clone())
                }
                _ => panic!("manuscript"),
            })
            .collect::<Vec<_>>(),
        (4..=63)
            .rev()
            .map(|index| {
                (
                    Uuid::from_u128(index + 1).to_string(),
                    format!("Synthetic Needle {index:02}"),
                )
            })
            .collect::<Vec<_>>()
    );
    let Resource::Manuscript {
        parts,
        new_member_access,
        group_access,
        ..
    } = &actual[0].resource
    else {
        panic!("manuscript")
    };
    assert_eq!(new_member_access, &Some(sources::Access::ReadWrite));
    assert_eq!(group_access, &Some(sources::Access::ReadWrite));
    assert_eq!(
        parts
            .iter()
            .map(|part| part.label.as_str())
            .collect::<Vec<_>>(),
        ["class.cls", "main.tex", "refs.bib"]
    );
    assert!(
        parts
            .iter()
            .all(|part| part.selected && part.access == sources::Access::ReadWrite)
    );
    let mut db = store.connection().unwrap();
    let tx = db.transaction().unwrap();
    let id = Uuid::from_u128(64).to_string();
    tx.execute(
        "update manuscripts set status='archived' where id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        resources.search_manuscripts(&tx, "Needle").unwrap()[0].0,
        Uuid::from_u128(63).to_string()
    );
    tx.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'sections/intro.tex','tex',0,?2,'now')",params![id,"b".repeat(64)]).unwrap();
    assert!(
        resources
            .list_text_paths(&tx, &id)
            .unwrap()
            .contains(&"sections/intro.tex".into())
    );
    tx.rollback().unwrap();
    assert!(
        !resources
            .list_text_paths(&db, &id)
            .unwrap()
            .contains(&"sections/intro.tex".into())
    );
    drop(db);
    for query in ["main.tex", "image.pdf"] {
        assert_eq!(
            serde_json::to_value(search_catalog(&store, query.into()).await.unwrap()).unwrap(),
            serde_json::to_value(&actual).unwrap()
        );
    }
    let manuscripts = search_catalog(&store, "".into())
        .await
        .unwrap()
        .into_iter()
        .filter(|selection| matches!(&selection.resource, Resource::Manuscript { .. }))
        .collect::<Vec<_>>();
    assert_eq!(
        serde_json::to_value(manuscripts).unwrap(),
        serde_json::to_value(&actual).unwrap()
    );
    assert!(
        search_catalog(&store, "unmatched".into())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search_catalog(&store, "a".repeat(513)).await.err(),
        Some(Error::InvalidInput)
    );
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_package_manuscript_directory_retains_safe_data_diagnostics() {
    let binding = binding();
    let resources = binding.context_catalog_resources().await.unwrap();
    let db = Connection::open_in_memory().unwrap();
    let ContextCatalogReadError::Native(error) =
        resources.search_manuscripts(&db, "Needle").unwrap_err()
    else {
        panic!("native diagnostic")
    };
    assert_eq!(error.package_key, binding.package_key);
    assert_eq!(
        error.resource_id.as_deref(),
        Some("writing.projects.context-catalog")
    );
    assert_eq!(error.resource_version, Some(1));
    assert_eq!(error.phase, "read");
    assert!(error.sqlite_code.is_some());
    assert!(error.sqlite_extended_code.is_some());
    let db = database();
    let id = manuscript(&db, 0);
    db.execute(
        "update manuscripts set title=x'50524956415445' where id=?1",
        [&id],
    )
    .unwrap();
    let wire = serde_json::to_string(&Error::from(
        resources.search_manuscripts(&db, "").err().unwrap(),
    ))
    .unwrap();
    assert!(wire.contains(&binding.package_key));
    assert!(wire.contains("writing.projects.context-catalog"));
    assert!(!wire.contains("PRIVATE"));
}

#[tokio::test]
#[ignore = "requires freshly built independent writing package collection"]
async fn actual_package_manuscript_directory_does_not_fall_back_after_selection_failure() {
    let mut binding = binding();
    binding.package_key = "f".repeat(64);
    let root = tempfile::tempdir().unwrap();
    let db = database();
    manuscript(&db, 0);
    let store = AppStore::for_test(db, root.path().join("objects"));
    let store = store.with_native_writing_context(binding.clone());
    assert_eq!(
        search_catalog(&store, "字".repeat(171)).await.err(),
        Some(Error::InvalidInput)
    );
    let Error::NativeCatalogUnavailable(error) =
        search_catalog(&store, "Needle".into()).await.err().unwrap()
    else {
        panic!("selected package failure must remain observable")
    };
    assert_eq!(error.package_key, binding.package_key);
    assert_eq!(error.phase, "admission");
    assert!(!error.code.is_empty());
    assert!(!error.safe_cause.is_empty());
}
