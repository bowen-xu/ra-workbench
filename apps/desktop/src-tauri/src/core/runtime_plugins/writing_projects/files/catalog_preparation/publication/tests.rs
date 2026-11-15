//! Exercise the deferred failure order against an actual accepted package.
use super::super::resources::Resources;
use super::*;
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeProfile, package},
    },
    storage::AppStore,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, Operation, PendingFile) {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let packages = std::fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
            .expect("Set the actual independent package directory"),
    )
    .unwrap();
    assert!(packages.starts_with(repository.join(".tmp")));
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(packages)));
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 1);
    let key = listing.packages[0].package_key.clone();
    let package = package::accept(&catalog, &key, NativeProfile::WritingProjects).unwrap();
    let resources = Resources::load(&catalog, &package).unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open(root.path().join("synthetic.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values('synthetic','Synthetic','','now')",[]).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let hash = "a".repeat(64);
    db.execute("insert into manuscripts(id,project_id,title,root_path,status,created_at,updated_at) values(?1,'synthetic','Synthetic','main.tex','active','now','now')",[&id]).unwrap();
    db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'note.ranote','other',0,?2,'now')",rusqlite::params![id,hash]).unwrap();
    let file = PendingFile {
        manuscript_id: id,
        path: "note.ranote".into(),
        hash,
    };
    let operation = Operation {
        store: AppStore::for_test(db, root.path().join("objects")),
        service: Arc::new(NativePreviewService::new(Arc::clone(&catalog))),
        resources,
        key,
        runtime: tokio::runtime::Handle::current(),
    };
    (root, operation, file)
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn changed_registration_skips_even_a_deferred_native_failure() {
    let (_root, operation, file) = fixture();
    operation
        .store
        .connection()
        .unwrap()
        .execute(
            "delete from writing_note_catalog where manuscript_id=?1",
            [&file.manuscript_id],
        )
        .unwrap();
    let mut outcome = SaveOutcome::NotStarted;
    let candidate = Err(failure(
        "plugin_host_read_failed",
        "synthetic deferred failure",
        &operation.key,
    ));
    assert!(!operation.publish(&file, candidate, &mut outcome).unwrap());
    assert_eq!(outcome, SaveOutcome::NotStarted);
    assert!(operation.store.connection().unwrap().is_autocommit());
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn matching_registration_consumes_the_original_deferred_native_failure() {
    let (_root, operation, file) = fixture();
    let mut outcome = SaveOutcome::NotStarted;
    let candidate = Err(failure(
        "plugin_host_read_failed",
        "synthetic deferred failure",
        &operation.key,
    ));
    let error = operation
        .publish(&file, candidate, &mut outcome)
        .unwrap_err();
    assert_eq!(error.code, "plugin_host_read_failed");
    assert_eq!(outcome, SaveOutcome::NotStarted);
    let db = operation.store.connection().unwrap();
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select state from writing_note_catalog", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "pending"
    );
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn pending_hash_read_failure_precedes_the_deferred_native_failure() {
    let (_root, operation, file) = fixture();
    let mut outcome = SaveOutcome::NotStarted;
    operation.store.connection().unwrap().authorizer(Some(
        |context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "writing_note_catalog",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        },
    ));
    let candidate = Err(failure(
        "plugin_host_read_failed",
        "synthetic deferred failure",
        &operation.key,
    ));
    let error = operation
        .publish(&file, candidate, &mut outcome)
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.note-prepare-current"
    );
    assert_eq!(outcome, SaveOutcome::NotStarted);
    assert!(operation.store.connection().unwrap().is_autocommit());
}
