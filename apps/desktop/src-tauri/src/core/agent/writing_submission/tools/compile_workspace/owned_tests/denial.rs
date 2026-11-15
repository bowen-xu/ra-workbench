use super::*;

#[tokio::test]
async fn compile_denies_active_uncertain_foreign_revoked_and_handed_off_owners() {
    for fault in [
        "active",
        "uncertain",
        "record",
        "binding",
        "revoked",
        "handoff",
    ] {
        let h = Harness::new().await;
        h.read().await;
        let mut compiler = h.compiler.clone();
        match fault {
            "active" => {
                let output = h.execute(h.call("active", crate::agent::shell_tools::EXEC,
                    json!({"command":"read value; printf '%s' \"$value\" > main.tex","yield_ms":0}))).await;
                assert_eq!(output["state"], "running");
            }
            "uncertain" => {
                h.source
                    .workspace(&h.record)
                    .await
                    .unwrap()
                    .host
                    .with_owned_lease_mut(|lease| {
                        drop(lease.begin_execution().unwrap());
                    })
                    .await
                    .unwrap();
            }
            "record" => compiler.record.input.text.push_str(" foreign frozen input"),
            "binding" => compiler.binding.context_snapshot_id = Uuid::new_v4(),
            "revoked" => {
                let mut db = h.fixture.store.connection().unwrap();
                let tx = db.transaction().unwrap();
                execution_workspaces::revoke_in(&tx, h.record.conversation_id, h.record.request_id)
                    .unwrap();
                tx.commit().unwrap();
            }
            "handoff" => assert!(h.tools.release_for_finalization().await.unwrap()),
            _ => unreachable!(),
        }
        let result = compiler.authorize_and_execute(h.compile_call(fault)).await;
        assert!(
            matches!(result, Err(RunError::ToolRejected)),
            "{fault}: {:?}",
            result.as_ref().err()
        );
        assert!(h.outputs().is_empty(), "{fault}");
        assert_eq!(
            std::fs::read_to_string(h.work.join("main.tex")).unwrap(),
            "Frozen manuscript"
        );
        assert_eq!(h.original(), "Frozen manuscript");
        h.sessions.processes().shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn artifact_validation_failure_still_cleans_output_without_faking_execution_or_delivery() {
    let mut h = Harness::new().await;
    let work = h.work.clone();
    h.compiler.before_output = Some(Arc::new(move || {
        let output = std::fs::read_dir(&work)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".research-assistant-compile-")
            })
            .unwrap();
        std::fs::write(output.join("main.pdf"), b"").unwrap();
    }));
    assert!(matches!(
        h.compiler
            .authorize_and_execute(h.compile_call("invalid-artifact"))
            .await,
        Err(RunError::EventSink)
    ));
    assert!(h.outputs().is_empty());
    assert!(!h.work.parent().unwrap().join("execution.json").exists());
    h.read().await;
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_cleanup_uses_existing_unknown_barrier_without_inventing_a_process_marker() {
    let mut h = Harness::new().await;
    let work = h.work.clone();
    let retained = h.fixture.root.path().join("retained-compile-output");
    let retained_in = retained.clone();
    h.compiler.before_output = Some(Arc::new(move || {
        let output = std::fs::read_dir(&work)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".research-assistant-compile-")
            })
            .unwrap();
        std::fs::rename(&output, &retained_in).unwrap();
        std::os::unix::fs::symlink(&retained_in, &output).unwrap();
    }));
    let result = h
        .compiler
        .authorize_and_execute(h.compile_call("cleanup-unconfirmed"))
        .await;
    let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) = result else {
        panic!("cleanup must preserve its safe recovery diagnostic");
    };
    let detail = serde_json::to_value(detail).unwrap();
    assert_eq!(detail["code"], "WORKSPACE_COMPILE_CLEANUP_REQUIRED");
    assert_eq!(detail["phase"], "compile-output-cleanup");
    assert_eq!(detail["safeCause"], "ephemeral_output_cleanup_unconfirmed");
    assert_eq!(
        detail["conversationId"],
        h.record.conversation_id.to_string()
    );
    assert_eq!(detail["requestId"], h.record.request_id.to_string());
    assert_eq!(detail["turnId"], h.record.turn_id.to_string());
    assert_eq!(
        detail["contextSnapshotId"],
        h.record.context_snapshot_id.to_string()
    );
    assert_eq!(detail["manuscriptId"], h.fixture.manuscript.to_string());
    assert_eq!(detail["leaseId"], h.compiler.scope.lease_id.to_string());
    assert!(
        !Uuid::parse_str(detail["resultId"].as_str().unwrap())
            .unwrap()
            .is_nil()
    );
    assert_eq!(detail.as_object().unwrap().len(), 10);
    assert!(!h.work.parent().unwrap().join("execution.json").exists());
    assert_eq!(h.outputs().len(), 1);
    assert!(retained.join("main.pdf").is_file());
    assert_eq!(h.original(), "Frozen manuscript");
    // Repair only this synthetic fault so TempDir cleanup remains deterministic.
    std::fs::remove_file(&h.outputs()[0]).unwrap();
    std::fs::remove_dir_all(retained).unwrap();
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn revoked_authority_after_process_exit_withholds_snapshot_and_still_cleans_output() {
    let mut h = Harness::new().await;
    let store = h.fixture.store.clone();
    let record = h.record.clone();
    h.compiler.before_output = Some(Arc::new(move || {
        let mut db = store.connection().unwrap();
        let tx = db.transaction().unwrap();
        execution_workspaces::revoke_in(&tx, record.conversation_id, record.request_id).unwrap();
        tx.commit().unwrap();
    }));
    assert!(matches!(
        h.compiler
            .authorize_and_execute(h.compile_call("revoke-before-output"))
            .await,
        Err(RunError::ToolRejected)
    ));
    assert!(h.outputs().is_empty());
    assert!(!h.work.parent().unwrap().join("execution.json").exists());
    assert!(
        !h.fixture
            .store
            .writing_cache_directory()
            .join("agent-compile-results")
            .exists()
    );
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn result_persistence_failure_still_cleans_ephemeral_output() {
    let mut h = Harness::new().await;
    let cache = h
        .fixture
        .store
        .writing_cache_directory()
        .join("agent-compile-results");
    h.compiler.before_output = Some(Arc::new(move || {
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, b"synthetic cache failure").unwrap();
    }));
    assert!(matches!(
        h.compiler
            .authorize_and_execute(h.compile_call("persist-failed"))
            .await,
        Err(RunError::EventSink)
    ));
    assert!(h.outputs().is_empty());
    assert!(!h.work.parent().unwrap().join("execution.json").exists());
    assert!(h.tools.finalization_call().await.unwrap().is_none());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
