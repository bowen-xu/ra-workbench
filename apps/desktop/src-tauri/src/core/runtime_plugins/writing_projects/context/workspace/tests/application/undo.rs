use super::*;
use crate::agent::writing_application_commands::undo;
use serde_json::{Value, json};

pub(super) async fn run(
    store: &AppStore,
    scope: &Scope,
    operation: Uuid,
    command: Value,
) -> Result<Value, writing_workspace_commands::Error> {
    let request = serde_json::from_value(json!({
        "scope": scope, "undoId": operation, "command": command,
    }))
    .unwrap();
    undo::execute(store, request)
        .await
        .map(|reply| serde_json::to_value(reply).unwrap())
}
pub(super) async fn applied() -> (Fixture, Binding, Scope) {
    let (f, binding, scope) = proposal().await;
    begin(&f.store, &scope).await;
    for path in ["main.tex", "remove.tex", "new.tex"] {
        apply(&f.store, &scope, path).await.unwrap();
    }
    (f, binding, scope)
}
fn missing(f: &Fixture, binding: &Binding) -> AppStore {
    f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    })
}
fn unavailable(result: Result<Value, writing_workspace_commands::Error>, operation: Uuid) {
    let error = serde_json::to_value(result.expect_err("missing native source")).unwrap();
    assert_eq!(error["code"], "plugin_host_package_unavailable");
    assert_eq!(error["operationId"], operation.to_string());
}
fn restored(f: &Fixture) {
    let root = source_root(f);
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert_eq!(
        std::fs::read(root.join("remove.tex")).unwrap(),
        b"Retained original"
    );
    assert!(!root.join("new.tex").exists());
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original undo lifecycle"]
async fn actual_package_undo_prepares_starts_resumes_and_restores_all_disk_changes() {
    let (f, binding, scope) = applied().await;
    let operation = Uuid::new_v4();
    let absent = missing(&f, &binding);
    unavailable(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"prepare","buffers":[]}),
        )
        .await,
        operation,
    );
    assert!(
        run(&f.store, &scope, operation, json!({"action":"read"}))
            .await
            .unwrap()["result"]
            .is_null()
    );
    let prepared = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"prepare","buffers":[]}),
    )
    .await
    .unwrap();
    assert_eq!(prepared["result"]["status"], "prepared");
    assert_eq!(prepared["result"]["files"].as_array().unwrap().len(), 3);
    assert_eq!(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"prepare","buffers":[]})
        )
        .await
        .unwrap(),
        prepared
    );
    unavailable(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"start","buffers":[]}),
        )
        .await,
        operation,
    );
    run(
        &f.store,
        &scope,
        operation,
        json!({"action":"start","buffers":[]}),
    )
    .await
    .unwrap();
    run(&f.store, &scope, operation, json!({"action":"defer"}))
        .await
        .unwrap();
    unavailable(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"resume","buffers":[]}),
        )
        .await,
        operation,
    );
    run(
        &f.store,
        &scope,
        operation,
        json!({"action":"resume","buffers":[]}),
    )
    .await
    .unwrap();
    unavailable(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"applyDisk","path":"main.tex"}),
        )
        .await,
        operation,
    );
    let first = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"applyDisk","path":"main.tex"}),
    )
    .await
    .unwrap();
    assert_eq!(
        run(
            &absent,
            &scope,
            operation,
            json!({"action":"applyDisk","path":"main.tex"})
        )
        .await
        .unwrap(),
        first
    );
    for path in ["new.tex", "remove.tex"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":path}),
        )
        .await
        .unwrap();
    }
    let done = run(
        &absent,
        &scope,
        operation,
        json!({"action":"start","buffers":[]}),
    )
    .await
    .unwrap();
    assert_eq!(done["result"]["status"], "applied");
    assert_eq!(done["undoId"], operation.to_string());
    assert_eq!(
        done["originalScope"]["operationId"],
        scope.operation_id.to_string()
    );
    assert_eq!(state(&f.store, &scope).await["status"], "applied");
    restored(&f);
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original undo index ACK recovery"]
async fn actual_package_undo_ack_failure_reconciles_same_file_before_remaining_dispatch() {
    let (f, binding, scope) = applied().await;
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
    f.store.connection().unwrap().execute_batch(
        "create trigger synthetic_undo_index_ack before insert on manuscript_files
         when new.relative_path='main.tex' begin select raise(abort,'synthetic undo index failure'); end;"
    ).unwrap();
    let error = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"applyDisk","path":"main.tex"}),
    )
    .await
    .unwrap_err();
    let error = serde_json::to_value(error).unwrap();
    assert_eq!(error["code"], "storage_error");
    assert_eq!(error["operationId"], operation.to_string());
    assert_eq!(error["saveOutcome"]["status"], "unknown");
    let before = std::fs::metadata(source_root(&f).join("main.tex")).unwrap();
    let failed = run(&f.store, &scope, operation, json!({"action":"read"}))
        .await
        .unwrap();
    assert_eq!(failed["result"]["status"], "recoveryRequired");
    assert_eq!(
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":"main.tex"})
        )
        .await
        .unwrap(),
        failed
    );
    let absent = missing(&f, &binding);
    let error = run(
        &absent,
        &scope,
        operation,
        json!({"action":"reconcileDisk","path":"main.tex"}),
    )
    .await
    .unwrap_err();
    let error = serde_json::to_value(error).unwrap();
    assert_eq!(error["code"], "plugin_host_package_unavailable");
    assert_eq!(error["operationId"], operation.to_string());
    assert_eq!(error["saveOutcome"]["status"], "unknown");
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_undo_index_ack")
        .unwrap();
    run(
        &f.store,
        &scope,
        operation,
        json!({"action":"reconcileDisk","path":"main.tex"}),
    )
    .await
    .unwrap();
    use std::os::unix::fs::MetadataExt;
    let after = std::fs::metadata(source_root(&f).join("main.tex")).unwrap();
    assert_eq!(
        (
            before.dev(),
            before.ino(),
            before.mtime(),
            before.mtime_nsec()
        ),
        (after.dev(), after.ino(), after.mtime(), after.mtime_nsec())
    );
    run(
        &f.store,
        &scope,
        operation,
        json!({"action":"resume","buffers":[]}),
    )
    .await
    .unwrap();
    for path in ["new.tex", "remove.tex"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":path}),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        run(&f.store, &scope, operation, json!({"action":"read"}))
            .await
            .unwrap()["result"]["status"],
        "applied"
    );
    restored(&f);
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; undo source pause rollback fence"]
async fn actual_package_undo_admission_settles_source_pause_and_fences_uncertain_rollback() {
    use crate::agent::writing_submission::workspace::{
        self,
        undo_admission::{self, Kind},
    };
    use crate::ai::structured_resource_validation::Flow;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for kind in [Kind::Prepare, Kind::Start, Kind::Resume] {
        for fail_rollback in [false, true] {
            let (f, binding, scope) = applied().await;
            let operation = Uuid::new_v4();
            if !matches!(kind, Kind::Prepare) {
                run(
                    &f.store,
                    &scope,
                    operation,
                    json!({"action":"prepare","buffers":[]}),
                )
                .await
                .unwrap();
            }
            if matches!(kind, Kind::Resume) {
                run(
                    &f.store,
                    &scope,
                    operation,
                    json!({"action":"start","buffers":[]}),
                )
                .await
                .unwrap();
                run(&f.store, &scope, operation, json!({"action":"defer"}))
                    .await
                    .unwrap();
            }
            let before = run(&f.store, &scope, operation, json!({"action":"read"}))
                .await
                .unwrap();
            if fail_rollback {
                f.store
                    .connection()
                    .unwrap()
                    .authorizer(Some(|context: AuthContext<'_>| match context.action {
                        AuthAction::Transaction {
                            operation: TransactionOperation::Rollback,
                        } => Authorization::Deny,
                        _ => Authorization::Allow,
                    }));
            }
            let mut captured = false;
            let paused = undo_admission::admit(
                &f.store,
                application_scope(&scope),
                operation,
                kind,
                |_, _, _| {
                    captured = true;
                    Err(Flow::Need("checkpoint-or-project"))
                },
            );
            assert!(captured);
            if fail_rollback {
                assert!(
                    matches!(paused, Err(Flow::Fail(cause)) if matches!(cause.error, workspace::Error::RecoveryRequired))
                );
                assert!(f.store.connection().is_err());
            } else {
                assert!(matches!(paused, Err(Flow::Need("checkpoint-or-project"))));
                assert!(f.store.connection().unwrap().is_autocommit());
                assert_eq!(
                    run(&f.store, &scope, operation, json!({"action":"read"}))
                        .await
                        .unwrap(),
                    before
                );
            }
            assert_eq!(
                std::fs::read(source_root(&f).join("main.tex")).unwrap(),
                b"Revised manuscript"
            );
            binding.service.shutdown().await.unwrap();
            let shutdown = f.store.shared_agent_connection().shutdown();
            if fail_rollback {
                assert!(
                    matches!(shutdown, Err(crate::data_interface::DataError::Admission(cause)) if cause.contains("data_recovery_required"))
                );
            } else {
                shutdown.unwrap();
            }
        }
    }
}
