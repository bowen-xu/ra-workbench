use super::undo::run;
use super::*;
use serde_json::json;

fn missing(f: &Fixture, binding: &Binding) -> AppStore {
    f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    })
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original application disk compensation"]
async fn actual_package_application_compensation_restores_all_dispatched_file_kinds_and_replays() {
    let (f, binding, scope) = proposal().await;
    begin(&f.store, &scope).await;
    apply(&f.store, &scope, "main.tex").await.unwrap();
    apply(&f.store, &scope, "new.tex").await.unwrap();
    f.store.connection().unwrap().execute_batch(
        "create trigger synthetic_remove_ack before delete on manuscript_files
         when old.relative_path='remove.tex' begin select raise(abort,'synthetic removal ACK failure'); end;"
    ).unwrap();
    let failed = apply(&f.store, &scope, "remove.tex")
        .await
        .err()
        .expect("forward index ACK failure");
    assert_eq!(
        serde_json::to_value(failed).unwrap()["code"],
        "storage_error"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_remove_ack")
        .unwrap();
    let absent = missing(&f, &binding);
    source_error(
        commands::execute(
            &absent,
            Request::PrepareRollback {
                scope: scope.clone(),
                buffers: vec![],
                checkpoint_versions: vec![],
            },
        )
        .await,
        &scope,
    );
    let root = source_root(&f);
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    assert!(root.join("new.tex").is_file());
    assert!(!root.join("remove.tex").exists());
    std::fs::write(root.join("main.tex"), b"User edit").unwrap();
    assert!(
        commands::execute(
            &f.store,
            Request::PrepareRollback {
                scope: scope.clone(),
                buffers: vec![],
                checkpoint_versions: vec![],
            }
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(root.join("main.tex")).unwrap(), b"User edit");
    assert_eq!(
        std::fs::read(root.join("new.tex")).unwrap(),
        b"Added manuscript"
    );
    assert!(!root.join("remove.tex").exists());
    std::fs::write(root.join("main.tex"), b"Revised manuscript").unwrap();
    commands::execute(
        &f.store,
        Request::PrepareRollback {
            scope: scope.clone(),
            buffers: vec![],
            checkpoint_versions: vec![],
        },
    )
    .await
    .unwrap();
    let failed = commands::execute(
        &absent,
        Request::RollbackDisk {
            scope: scope.clone(),
        },
    )
    .await
    .err()
    .expect("compensation source unavailable");
    let failed = serde_json::to_value(failed).unwrap();
    assert_eq!(failed["code"], "plugin_host_package_unavailable");
    assert_eq!(failed["operationId"], scope.operation_id.to_string());
    assert_eq!(failed["saveOutcome"]["status"], "unknown");
    let restored = serde_json::to_value(
        commands::execute(
            &f.store,
            Request::RollbackDisk {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(restored["status"], "rolledBack");
    assert!(
        restored["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["phase"] == "restored")
    );
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!root.join("new.tex").exists());
    assert_eq!(
        std::fs::read(root.join("remove.tex")).unwrap(),
        b"Retained original"
    );
    assert_eq!(
        serde_json::to_value(
            commands::execute(
                &absent,
                Request::RollbackDisk {
                    scope: scope.clone()
                }
            )
            .await
            .unwrap()
        )
        .unwrap(),
        restored
    );
    assert_eq!(
        serde_json::to_value(
            commands::execute(
                &absent,
                Request::PrepareRollback {
                    scope: scope.clone(),
                    buffers: vec![],
                    checkpoint_versions: vec![]
                }
            )
            .await
            .unwrap()
        )
        .unwrap(),
        restored
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; inverse compensation with failed index ACK"]
async fn actual_package_undo_compensation_keeps_original_proof_and_recovers_failed_restoration_ack()
{
    let (f, binding, scope) = super::undo::applied().await;
    let operation = Uuid::new_v4();
    for action in ["prepare", "start"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":action,"buffers":[]}),
        )
        .await
        .unwrap();
    }
    for path in ["main.tex", "new.tex"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":path}),
        )
        .await
        .unwrap();
    }
    let absent = missing(&f, &binding);
    let missing = run(
        &absent,
        &scope,
        operation,
        json!({"action":"prepareRollback","buffers":[]}),
    )
    .await
    .unwrap_err();
    let missing = serde_json::to_value(missing).unwrap();
    assert_eq!(missing["code"], "plugin_host_package_unavailable");
    assert_eq!(missing["operationId"], operation.to_string());
    let unregistered = source_root(&f).join("new.tex");
    std::fs::write(&unregistered, b"User file").unwrap();
    assert!(
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"prepareRollback","buffers":[]})
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&unregistered).unwrap(), b"User file");
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    std::fs::remove_file(unregistered).unwrap();
    run(
        &f.store,
        &scope,
        operation,
        json!({"action":"prepareRollback","buffers":[]}),
    )
    .await
    .unwrap();
    let root = source_root(&f);
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!root.join("new.tex").exists());
    f.store.connection().unwrap().execute_batch(
        "create trigger synthetic_compensation_ack before insert on manuscript_files
         when new.relative_path='main.tex' begin select raise(abort,'synthetic compensation ACK failure'); end;"
    ).unwrap();
    let failed = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"rollbackDisk","buffers":[]}),
    )
    .await
    .unwrap_err();
    let failed = serde_json::to_value(failed).unwrap();
    assert_eq!(failed["code"], "storage_error");
    assert_eq!(failed["operationId"], operation.to_string());
    assert_eq!(failed["saveOutcome"]["status"], "unknown");
    let before = std::fs::metadata(root.join("main.tex")).unwrap();
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    assert_eq!(
        std::fs::read(root.join("new.tex")).unwrap(),
        b"Added manuscript"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_compensation_ack")
        .unwrap();
    let restored = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"rollbackDisk","buffers":[]}),
    )
    .await
    .unwrap();
    assert_eq!(restored["result"]["status"], "rolledBack");
    assert_eq!(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"rollbackDisk","buffers":[]})
        )
        .await
        .unwrap(),
        restored
    );
    assert_eq!(state(&f.store, &scope).await["status"], "applied");
    assert!(!root.join("remove.tex").exists());
    use std::os::unix::fs::MetadataExt;
    let after = std::fs::metadata(root.join("main.tex")).unwrap();
    assert_eq!(
        (
            before.dev(),
            before.ino(),
            before.mtime(),
            before.mtime_nsec()
        ),
        (after.dev(), after.ino(), after.mtime(), after.mtime_nsec())
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
