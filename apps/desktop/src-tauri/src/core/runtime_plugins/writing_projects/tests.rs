use super::*;
use crate::data_interface::Database;

mod bibliography_adoption;
mod bibliography_entries;
#[cfg(target_os = "macos")]
mod bibliography_host_failure;
mod bibliography_library;
mod bibliography_mutations;
mod bibliography_rekey;
mod bibliography_runtime;
mod catalog_preparation;
mod catalog_query;
mod checkpoints;
mod citation_metadata;
mod creation;
mod directories;
mod directory_reads;
mod file_changes;
mod file_create;
mod file_save;
pub(super) mod files;
mod metadata;
mod note_asset_catalog;
mod note_assets;
mod note_assets_unknown;
mod project_deletion;
mod project_transfer;
mod recovery;
mod resource_import;
mod sessions;
mod status;

pub(super) struct Fixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) database: SharedDatabase,
    pub(super) service: Arc<NativePreviewService>,
    pub(super) catalog: Arc<RuntimePluginCatalog>,
    pub(super) key: String,
    pub(super) active: String,
    pub(super) archived: String,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap();
        let directory = std::fs::canonicalize(
            std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES")
                .expect("Set the actual independent package directory"),
        )
        .unwrap();
        assert!(directory.starts_with(repository.join(".tmp")));
        let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory)));
        let listing = catalog.discover().unwrap();
        assert_eq!(listing.packages.len(), 1);
        let key = listing.packages[0].package_key.clone();
        let fixture = tempfile::tempdir().unwrap();
        let mut sql = rusqlite::Connection::open(fixture.path().join("synthetic.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('synthetic','Synthetic','','now')",[]).unwrap();
        let active = uuid::Uuid::new_v4().to_string();
        let archived = uuid::Uuid::new_v4().to_string();
        for (id, status, title) in [
            (&archived, "archived", "Archived"),
            (&active, "active", "  Legacy title  "),
        ] {
            sql.execute("insert into manuscripts(id,project_id,title,root_path,status,created_at,updated_at) values(?1,'synthetic',?2,'main.tex',?3,'created','updated')",rusqlite::params![id,title,status]).unwrap();
        }
        let database = Database::open(sql, &fixture.path().join("data-log")).unwrap();
        let service = Arc::new(NativePreviewService::new(Arc::clone(&catalog)));
        Self {
            directory: fixture,
            database,
            service,
            catalog,
            key,
            active,
            archived,
        }
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_reads_original_database_and_preserves_identity_order_and_not_found() {
    let Fixture {
        directory: _directory,
        database,
        service,
        catalog,
        key,
        active,
        archived,
    } = Fixture::new();
    let items = list_projects(
        Arc::clone(&database),
        &service,
        Arc::clone(&catalog),
        key.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        [active.as_str(), archived.as_str()]
    );
    assert_eq!(items[0].title, "  Legacy title  ");
    let loaded = get_project(
        Arc::clone(&database),
        &service,
        Arc::clone(&catalog),
        key.clone(),
        format!(" {active} "),
    )
    .await
    .unwrap();
    assert_eq!(loaded, items[0]);
    let first = get_project(
        Arc::clone(&database),
        &service,
        Arc::clone(&catalog),
        key.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .await
    .unwrap_err();
    assert_eq!(first.code, "not_found");
    assert_eq!(first.message, "稿件不存在");
    assert_eq!(first.plugin_id, "writing_projects");
    assert_eq!(first.package_key.as_deref(), Some(key.as_str()));
    assert!(first.instance_id.is_some() && first.request_id.is_some());
    let second = get_project(
        database,
        &service,
        catalog,
        key,
        uuid::Uuid::new_v4().to_string(),
    )
    .await
    .unwrap_err();
    assert_eq!(second.instance_id, first.instance_id);
    assert_ne!(second.request_id, first.request_id);
    service.shutdown().await.unwrap();
}
