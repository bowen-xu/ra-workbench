use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::status::{StatusAction, change_status},
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

async fn change(fixture: &Fixture, id: &str, action: StatusAction) -> NativeResult<Manuscript> {
    change_status(
        Arc::clone(&fixture.database),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        id.into(),
        action,
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_archive_restore_preserves_project_files_and_missing_identity() {
    let fixture = Fixture::new();
    let original =
        super::creation::create(&fixture, uuid::Uuid::new_v4(), None, "Archive and restore")
            .await
            .unwrap();
    let source = fixture
        .directory
        .path()
        .join("manuscripts")
        .join(&original.id)
        .join("source/sections/main.tex");
    let bytes = std::fs::read(&source).unwrap();
    let indexed: (String, i64) = fixture
        .database
        .lock()
        .unwrap()
        .query_row(
            "select sha256,byte_size from manuscript_files where manuscript_id=?1",
            [&original.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let archived = change(
        &fixture,
        &format!(" {} ", original.id),
        StatusAction::Archive,
    )
    .await
    .unwrap();
    assert_eq!(archived.id, original.id);
    assert_eq!(archived.project_id, original.project_id);
    assert_eq!(archived.created_at, original.created_at);
    assert_eq!(archived.root_path, original.root_path);
    assert_eq!(archived.title, original.title);
    assert_eq!(archived.status, "archived");
    let restored = change(&fixture, &original.id, StatusAction::Restore)
        .await
        .unwrap();
    assert_eq!(restored.id, original.id);
    assert_eq!(restored.project_id, original.project_id);
    assert_eq!(restored.created_at, original.created_at);
    assert_eq!(restored.root_path, original.root_path);
    assert_eq!(restored.title, original.title);
    assert_eq!(restored.status, "active");
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    let after: (String, i64) = fixture
        .database
        .lock()
        .unwrap()
        .query_row(
            "select sha256,byte_size from manuscript_files where manuscript_id=?1",
            [&original.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, indexed);
    let missing_id = uuid::Uuid::new_v4().to_string();
    for action in [StatusAction::Archive, StatusAction::Restore] {
        let missing = change(&fixture, &missing_id, action).await.unwrap_err();
        assert_eq!(missing.code, "not_found");
        assert_eq!(missing.message, "稿件不存在");
        assert_eq!(missing.operation_id.as_deref(), Some(missing_id.as_str()));
        assert_eq!(missing.save_outcome, Some(SaveOutcome::Unknown));
        assert!(missing.instance_id.is_some() && missing.request_id.is_some());
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_status_failure_preserves_original_autocommit_prefix_and_diagnostics() {
    let fixture = Fixture::new();
    fixture
        .database
        .lock()
        .unwrap()
        .execute_batch(
            "create table synthetic_writing_status_prefix(value text not null);
        create temp trigger test_writing_status_fail before update of status on manuscripts
        begin insert into synthetic_writing_status_prefix values('committed prefix');
        select raise(fail,'PRIVATE_STATUS_PREFIX');end;",
        )
        .unwrap();
    let error = change(&fixture, &fixture.active, StatusAction::Archive)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.operation_id.as_deref(), Some(fixture.active.as_str()));
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.archive"
    );
    assert!(!error.message.contains("PRIVATE_STATUS_PREFIX"));
    assert!(error.instance_id.is_some());
    assert!(error.request_id.is_some());
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select status from manuscripts where id=?1",
                [&fixture.active],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "active"
        );
        assert_eq!(
            db.query_row(
                "select count(*) from synthetic_writing_status_prefix",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    fixture.service.shutdown().await.unwrap();
    fixture.database.shutdown().unwrap();
    drop(fixture.database);
    let sql =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    let reopened = Database::open(sql, &fixture.directory.path().join("data-log")).unwrap();
    let db = reopened.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from synthetic_writing_status_prefix",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn status_host_startup_failure_precedes_the_original_update() {
    let fixture = Fixture::new();
    let broken_packages = fixture.directory.path().join("broken-packages");
    let package = broken_packages.join("writing_projects");
    std::fs::create_dir_all(package.join("data")).unwrap();
    let manifest = fixture
        .catalog
        .resource(&fixture.key, "manifest.json")
        .unwrap()
        .bytes;
    let decoded: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let entry = decoded["native"]["entry"].as_str().unwrap();
    assert!(entry.starts_with("native/") && !entry.split('/').any(|component| component == ".."));
    std::fs::write(package.join("manifest.json"), manifest).unwrap();
    for name in ["list", "get", "archive"] {
        let relative = format!("data/project-{name}.v1.sql");
        let resource = fixture.catalog.resource(&fixture.key, &relative).unwrap();
        std::fs::write(package.join(relative), resource.bytes).unwrap();
    }
    let mut frontend_resources = Vec::new();
    if let Some(entry) = decoded["entry"].as_str() {
        frontend_resources.push(entry);
    }
    if let Some(styles) = decoded["styles"].as_array() {
        frontend_resources.extend(styles.iter().map(|style| style.as_str().unwrap()));
    }
    for relative in frontend_resources {
        let resource = fixture.catalog.resource(&fixture.key, relative).unwrap();
        let destination = package.join(relative);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, resource.bytes).unwrap();
    }
    let executable = package.join(entry);
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, b"#!/bin/sh\nexit 43\n").unwrap();
    std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(broken_packages)));
    let discovery = catalog.discover().unwrap();
    assert_eq!(discovery.packages.len(), 1);
    let key = discovery.packages[0].package_key.clone();
    let service = Arc::new(NativePreviewService::new(Arc::clone(&catalog)));
    let writes = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.database.lock().unwrap();
        let observed = Arc::clone(&writes);
        db.create_scalar_function(
            "test_writing_status_attempt",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch("create temp trigger test_writing_status_attempt before update of status on manuscripts begin select test_writing_status_attempt();end;").unwrap();
    }
    let error = change_status(
        Arc::clone(&fixture.database),
        Arc::clone(&service),
        catalog,
        key,
        fixture.active.clone(),
        StatusAction::Archive,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "plugin_host_exited");
    assert_eq!(error.exit_code, Some(43));
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .database
            .lock()
            .unwrap()
            .query_row(
                "select status from manuscripts where id=?1",
                [&fixture.active],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "active"
    );
    service.shutdown().await.unwrap();
    fixture.service.shutdown().await.unwrap();
}
