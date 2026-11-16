use super::*;
use crate::{
    ai::paper_reading_source::PaperReadingSource,
    runtime_events::RuntimeEvents,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    },
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::Arc};

struct Fixture {
    _data: tempfile::TempDir,
    package: tempfile::TempDir,
    store: AppStore,
    key: String,
    object: PathBuf,
    hash: String,
}

impl Fixture {
    fn new(source: Option<PaperReadingSource>) -> Self {
        // An accidental Native handshake would fail. Location is a fixed Core read.
        let package = tempfile::tempdir().unwrap();
        let selected = package.path().join("literature");
        fs::create_dir_all(selected.join("data")).unwrap();
        fs::write(selected.join("host"), b"#!/bin/sh\nexit 59\n").unwrap();
        fs::write(
            selected.join("manifest.json"),
            serde_json::to_vec(&json!({
                "formatVersion":2,"id":"literature_library","version":"0.1.0","apiVersion":1,
                "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
                "provides":[{"id":"literature.note-trace-decision","version":1},{"id":"literature.attachments","version":1}],
                "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
            }))
            .unwrap(),
        )
        .unwrap();
        let original = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/attachment-resources-v1/attachment-content-path.v1.sql");
        fs::copy(
            original,
            selected.join("data/attachment-content-path.v1.sql"),
        )
        .unwrap();
        let catalog = Arc::new(
            RuntimePluginCatalog::new(Some(package.path().to_path_buf()))
                .with_native_writing_preview(true),
        );
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let data = tempfile::tempdir().unwrap();
        let store = AppStore::open_with_paths(
            AppStorePaths {
                data_directory: data.path().join("store"),
                agent_configuration_directory: data.path().join("configuration"),
            },
            AppStorePorts::with_events(
                RuntimeEvents::new(|_, _| Ok(())),
                AppStoreSources {
                    paper_reading_source: source,
                    ..Default::default()
                },
            ),
        )
        .unwrap()
        .with_native_note_trace(NoteTraceBinding {
            service: Arc::new(LiteratureTraceService::new(catalog)),
            package_key: key.clone(),
        });
        let bytes = b"%PDF-1.7\nSynthetic path-only fixture.\n";
        let hash = format!("{:x}", Sha256::digest(bytes));
        let object = store.content_directory().join("synthetic.pdf");
        fs::write(&object, bytes).unwrap();
        {
            let db = store.connection().unwrap();
            db.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Synthetic target','before','before'),('other','document','Synthetic other','before','before');").unwrap();
            db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic.pdf','application/pdf',?2,'before')",rusqlite::params![hash,bytes.len() as i64]).unwrap();
            db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('pdf','paper',?1,'中文 α.pdf','before')",[&hash]).unwrap();
        }
        Self {
            _data: data,
            package,
            store,
            key,
            object,
            hash,
        }
    }

    fn package_file(&self, name: &str) -> PathBuf {
        self.package.path().join("literature").join(name)
    }
}

#[tokio::test]
async fn native_location_needs_only_content_path_and_returns_owned_input_without_writes() {
    let fixture = Fixture::new(None);
    assert!(
        !fixture
            .package_file("data/attachment-rename.v1.sql")
            .exists()
    );
    let before = fixture.store.connection().unwrap().total_changes();
    let (settings, path, name, hash) = prepare(&fixture.store, "paper", "pdf").await.unwrap();
    assert_eq!(path, fixture.object.canonicalize().unwrap());
    assert_eq!(name, "中文 α.pdf");
    assert_eq!(hash, fixture.hash);
    assert_eq!(
        settings.default_port,
        repository::get_settings(&fixture.store.connection().unwrap())
            .unwrap()
            .default_port
    );
    let db = fixture.store.connection().unwrap();
    assert_eq!(db.total_changes(), before);
    assert!(db.is_autocommit());
    assert_eq!(fs::read(path).unwrap(), fs::read(&fixture.object).unwrap());
}

#[tokio::test]
async fn settings_failure_precedes_native_package_admission() {
    let fixture = Fixture::new(None);
    fixture
        .store
        .connection()
        .unwrap()
        .execute_batch("drop table docling_settings")
        .unwrap();
    fs::remove_dir_all(fixture.package_file("")).unwrap();
    assert!(matches!(
        prepare(&fixture.store, "paper", "pdf").await,
        Err(DoclingError::Database(_))
    ));
}

#[tokio::test]
async fn path_failure_precedes_ownership_and_ownership_keeps_its_original_error() {
    let fixture = Fixture::new(None);
    let bytes = fs::read(&fixture.object).unwrap();
    fs::remove_file(&fixture.object).unwrap();
    let error = prepare(&fixture.store, "other", "pdf").await.unwrap_err();
    assert!(error.to_string().contains("附件文件已不存在"));
    fs::write(&fixture.object, bytes).unwrap();
    let error = prepare(&fixture.store, "other", "pdf").await.unwrap_err();
    assert!(matches!(error, DoclingError::Attachment(ref value) if value == "附件不属于当前文献"));
}

#[tokio::test]
async fn display_name_decode_precedes_the_original_ownership_comparison() {
    let fixture = Fixture::new(None);
    fixture
        .store
        .connection()
        .unwrap()
        .execute(
            "update attachments set display_name=?1 where id='pdf'",
            [vec![0xff_u8]],
        )
        .unwrap();
    assert!(matches!(
        prepare(&fixture.store, "other", "pdf").await,
        Err(DoclingError::Database(rusqlite::Error::InvalidColumnType(
            ..
        )))
    ));
}

#[tokio::test]
async fn ownership_rejection_precedes_the_original_content_hash_decode() {
    let fixture = Fixture::new(None);
    fs::copy(
        &fixture.object,
        fixture.store.content_directory().join("blob-hash.pdf"),
    )
    .unwrap();
    {
        let db = fixture.store.connection().unwrap();
        // Deliberately malformed typed cells exercise the deployed row.get order.
        let blob_hash = vec![b'a'; 64];
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'blob-hash.pdf','application/pdf',1,'before')",[&blob_hash]).unwrap();
        db.execute(
            "update attachments set content_sha256=?1 where id='pdf'",
            [&blob_hash],
        )
        .unwrap();
    }
    let error = prepare(&fixture.store, "other", "pdf").await.unwrap_err();
    assert!(matches!(error, DoclingError::Attachment(ref value) if value == "附件不属于当前文献"));
    assert!(matches!(
        prepare(&fixture.store, "paper", "pdf").await,
        Err(DoclingError::Database(rusqlite::Error::InvalidColumnType(
            ..
        )))
    ));
}

#[tokio::test]
async fn native_location_rejects_unsafe_and_missing_paths() {
    let fixture = Fixture::new(None);
    for relative in ["../outside.pdf", "/outside.pdf"] {
        fixture
            .store
            .connection()
            .unwrap()
            .execute("update content_objects set relative_path=?1", [relative])
            .unwrap();
        let error = prepare(&fixture.store, "paper", "pdf").await.unwrap_err();
        assert!(error.to_string().contains("附件存储路径不安全"));
    }
    let error = prepare(&fixture.store, "paper", "absent")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("找不到文献或附件"));
}

#[tokio::test]
async fn native_path_database_error_retains_only_available_safe_resource_diagnostics() {
    let fixture = Fixture::new(None);
    fixture
        .store
        .connection()
        .unwrap()
        .execute_batch("alter table content_objects rename to private_source_objects")
        .unwrap();
    let error = prepare(&fixture.store, "paper", "pdf").await.unwrap_err();
    let DoclingError::Attachment(message) = error else {
        panic!("expected the native attachment location failure");
    };
    for field in [
        "packageKey=",
        "resourceId=literature.attachments.content-path",
        "resourceVersion=1",
        "dataCode=plugin_data_sql_failed",
        "phase=",
        "safeCause=",
        "sqliteCode=",
        "sqliteExtendedCode=",
    ] {
        assert!(message.contains(field), "missing {field}: {message}");
    }
    assert!(message.contains(&fixture.key));
    assert!(!message.contains(&fixture._data.path().display().to_string()));
    for private in [
        "private_source_objects",
        "select c.relative_path",
        "requestId=",
        "instanceId=",
    ] {
        assert!(!message.contains(private), "unexpected {private}");
    }
}

#[tokio::test]
async fn selected_package_or_resource_failure_is_explicit_without_static_fallback() {
    for missing_resource in [false, true] {
        let fixture = Fixture::new(None);
        if missing_resource {
            fs::remove_file(fixture.package_file("data/attachment-content-path.v1.sql")).unwrap();
        } else {
            fs::remove_dir_all(fixture.package_file("")).unwrap();
        }
        let error = prepare(&fixture.store, "paper", "pdf").await.unwrap_err();
        assert!(matches!(error, DoclingError::Attachment(_)));
        assert!(error.to_string().contains("plugin_"));
        assert!(error.to_string().contains(&fixture.key));
        assert!(!error.to_string().contains("CAPABILITY_UNAVAILABLE"));
    }
}
