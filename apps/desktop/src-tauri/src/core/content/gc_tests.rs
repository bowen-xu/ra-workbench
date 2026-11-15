use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::*;

fn test_store() -> (AppStore, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("research-assistant-content-gc-{nonce}"));
    let objects = root.join("objects");
    fs::create_dir_all(&objects).expect("objects");
    let mut connection = Connection::open_in_memory().expect("database");
    crate::storage::migrate(&mut connection).expect("migrate");
    connection
        .execute(
            "update content_gc_state set backfill_status = 'complete' where id = 1",
            [],
        )
        .expect("backfill complete");
    (AppStore::for_test(connection, objects), root)
}

fn insert_object(store: &AppStore, bytes: &[u8]) -> String {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let relative = expected_relative_path(&sha256).expect("relative");
    let path = store.content_directory().join(&relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("parent directory");
    fs::write(&path, bytes).expect("object");
    let connection = store.connection().expect("connection");
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', ?3, 'now')",
            params![sha256, relative.to_string_lossy(), bytes.len() as i64],
        )
        .expect("metadata");
    sha256
}

#[test]
fn sweep_deletes_only_due_unreferenced_objects() {
    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let removable = insert_object(&store, b"unused image");
    let retained = insert_object(&store, b"used image");
    {
        let connection = store.connection().expect("connection");
        connection.execute(
            "insert into note_pages (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 0, 1, 'now', 'now')",
            [],
        ).expect("page");
        connection.execute(
            "insert into note_blocks (id, scope_kind, page_id, position, block_type, props_json, content_json, plain_text, schema_version, revision, status, is_favorite, created_at, updated_at)
             values ('block', 'page', 'page', 0, 'image', '{}', '{}', '', 1, 0, 'active', 0, 'now', 'now')",
            [],
        ).expect("block");
        connection.execute(
            "insert into note_block_assets (block_id, content_sha256, display_name, mime_type, created_at)
             values ('block', ?1, 'used.png', 'image/png', 'now')",
            [retained.clone()],
        ).expect("reference");
        connection
            .execute(
                "update content_object_gc_candidates set not_before = '2000-01-01T00:00:00Z'
             where content_sha256 = ?1",
                [removable.clone()],
            )
            .expect("due");
    }

    let (count, _, warnings) = sweep_candidates(&store, &manager, false).expect("sweep");
    assert_eq!(count, 1);
    assert!(warnings.is_empty());
    let connection = store.connection().expect("connection");
    let removable_exists: bool = connection
        .query_row(
            "select exists(select 1 from content_objects where sha256 = ?1)",
            [removable],
            |row| row.get(0),
        )
        .expect("removable metadata");
    let retained_exists: bool = connection
        .query_row(
            "select exists(select 1 from content_objects where sha256 = ?1)",
            [retained],
            |row| row.get(0),
        )
        .expect("retained metadata");
    assert!(!removable_exists);
    assert!(retained_exists);
    drop(connection);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn startup_recovery_restores_a_quarantined_live_object() {
    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let sha256 = insert_object(&store, b"recover me");
    let source = store
        .content_directory()
        .join(expected_relative_path(&sha256).expect("relative"));
    let quarantine = store.content_directory().join(".gc");
    fs::create_dir_all(&quarantine).expect("quarantine");
    fs::rename(&source, quarantine.join(&sha256)).expect("simulate crash");

    assert!(
        recover_quarantine(&store, &manager)
            .expect("recover")
            .is_empty()
    );
    assert!(source.exists());
    assert!(!quarantine.join(&sha256).exists());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn grace_period_is_preserved_unless_the_user_explicitly_includes_it() {
    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let sha256 = insert_object(&store, b"recent orphan");

    let (count, _, _) = sweep_candidates(&store, &manager, false).expect("safe sweep");
    assert_eq!(count, 0);
    assert!(
        store
            .content_directory()
            .join(expected_relative_path(&sha256).expect("path"))
            .exists()
    );

    let (count, _, _) = sweep_candidates(&store, &manager, true).expect("explicit sweep");
    assert_eq!(count, 1);
    assert!(
        !store
            .content_directory()
            .join(expected_relative_path(&sha256).expect("path"))
            .exists()
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn cancellation_leaves_persistent_job_state_idle() {
    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    assert!(manager.cancel());
    assert!(matches!(
        run_scan(&store, &manager),
        Err(ContentError::Cancelled)
    ));
    let state: String = store
        .connection()
        .expect("connection")
        .query_row(
            "select job_state from content_gc_state where id = 1",
            [],
            |row| row.get(0),
        )
        .expect("job state");
    assert_eq!(state, "idle");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn reference_added_after_quarantine_restores_the_object() {
    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let sha256 = insert_object(&store, b"race protected");
    let source = store
        .content_directory()
        .join(expected_relative_path(&sha256).expect("path"));
    {
        let connection = store.connection().expect("connection");
        connection.execute(
            "insert into note_pages
             (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values ('page', 'Page', 0, 'active', 0, 0, 1, 'now', 'now')",
            [],
        ).expect("page");
        connection
            .execute(
                "insert into note_blocks
             (id, scope_kind, page_id, position, block_type, props_json, content_json,
              plain_text, schema_version, revision, status, is_favorite, created_at, updated_at)
             values ('block', 'page', 'page', 0, 'image', '{}', '{}', '', 1, 0,
                     'active', 0, 'now', 'now')",
                [],
            )
            .expect("block");
        connection
            .execute(
                "update content_object_gc_candidates set not_before = '2000-01-01T00:00:00Z'
             where content_sha256 = ?1",
                [&sha256],
            )
            .expect("due");
    }

    let (count, _, warnings) =
        sweep_candidates_with_hook(&store, &manager, false, |store, content_sha256| {
            let connection = store.connection()?;
            connection.execute(
                "insert into note_block_assets
                 (block_id, content_sha256, display_name, mime_type, created_at)
                 values ('block', ?1, 'race.png', 'image/png', 'now')",
                [content_sha256],
            )?;
            Ok(())
        })
        .expect("sweep race");
    assert_eq!(count, 0);
    assert!(warnings.is_empty());
    assert!(source.exists());
    assert!(!store.content_directory().join(".gc").join(&sha256).exists());
    let metadata_exists: bool = store
        .connection()
        .expect("connection")
        .query_row(
            "select exists(select 1 from content_objects where sha256 = ?1)",
            [&sha256],
            |row| row.get(0),
        )
        .expect("metadata");
    assert!(metadata_exists);
    fs::remove_dir_all(root).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn sweep_refuses_symlinked_content_objects() {
    use std::os::unix::fs::symlink;

    let (store, root) = test_store();
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let bytes = b"outside object";
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let relative = expected_relative_path(&sha256).expect("relative");
    let object = store.content_directory().join(&relative);
    fs::create_dir_all(object.parent().expect("parent")).expect("parent directory");
    let outside = root.join("outside.bin");
    fs::write(&outside, bytes).expect("outside file");
    symlink(&outside, &object).expect("object symlink");
    {
        let connection = store.connection().expect("connection");
        connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', ?3, 'now')",
            params![sha256, relative.to_string_lossy(), bytes.len() as i64],
        ).expect("metadata");
        connection
            .execute(
                "update content_object_gc_candidates set not_before = '2000-01-01T00:00:00Z'
             where content_sha256 = ?1",
                [&sha256],
            )
            .expect("due");
    }

    let (count, _, warnings) = sweep_candidates(&store, &manager, false).expect("sweep");
    assert_eq!(count, 0);
    assert_eq!(warnings, vec!["unsafe_path"]);
    assert_eq!(fs::read(&outside).expect("outside remains"), bytes);
    let metadata_exists: bool = store
        .connection()
        .expect("connection")
        .query_row(
            "select exists(select 1 from content_objects where sha256 = ?1)",
            [&sha256],
            |row| row.get(0),
        )
        .expect("metadata");
    assert!(metadata_exists);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
#[ignore = "isolated performance budget; pnpm check:rust runs it serially"]
fn marks_ten_thousand_objects_within_the_frozen_budget() {
    let (store, root) = test_store();
    {
        let mut connection = store.connection().expect("connection");
        let transaction = connection.transaction().expect("transaction");
        for index in 0..10_000_u64 {
            let sha256 = format!("{index:064x}");
            transaction
                .execute(
                    "insert into content_objects
                 (sha256, relative_path, mime_type, byte_size, created_at)
                 values (?1, ?2, 'application/octet-stream', 1, 'now')",
                    params![
                        sha256,
                        format!("{}/{}/{}", &sha256[..2], &sha256[2..4], sha256)
                    ],
                )
                .expect("object");
        }
        transaction
            .execute("delete from content_object_gc_candidates", [])
            .expect("simulate legacy objects");
        transaction.commit().expect("commit fixtures");
    }
    let manager = ContentGcManager::default();
    manager.begin().expect("begin");
    let started = Instant::now();
    assert_eq!(run_scan(&store, &manager).expect("scan"), 10_000);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "10,000-object mark exceeded 500 ms: {:?}",
        started.elapsed()
    );
    fs::remove_dir_all(root).expect("cleanup");
}
