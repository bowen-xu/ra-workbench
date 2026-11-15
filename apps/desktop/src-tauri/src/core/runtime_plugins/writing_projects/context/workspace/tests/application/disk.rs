use super::*;

#[tokio::test]
#[ignore = "requires actual independent writing package; C04/C05 original disk application and ACK"]
async fn actual_package_disk_application_replaces_creates_deletes_and_replays_without_redispatch() {
    let (f, binding, scope) = proposal().await;
    begin(&f.store, &scope).await;
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let pending = state(&f.store, &scope).await;
    source_error(apply(&missing, &scope, "main.tex").await, &scope);
    assert_eq!(state(&f.store, &scope).await, pending);
    let root = source_root(&f);
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!root.join("new.tex").exists());
    assert!(root.join("remove.tex").is_file());
    let replaced =
        serde_json::to_value(apply(&f.store, &scope, "main.tex").await.unwrap()).unwrap();
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    assert_eq!(
        serde_json::to_value(apply(&missing, &scope, "main.tex").await.unwrap()).unwrap(),
        replaced
    );
    apply(&f.store, &scope, "remove.tex").await.unwrap();
    assert!(!root.join("remove.tex").exists());
    let applied = serde_json::to_value(apply(&f.store, &scope, "new.tex").await.unwrap()).unwrap();
    assert_eq!(applied["status"], "applied");
    assert_eq!(
        std::fs::read(root.join("new.tex")).unwrap(),
        b"Added manuscript"
    );
    assert_eq!(
        serde_json::to_value(apply(&missing, &scope, "new.tex").await.unwrap()).unwrap(),
        applied
    );
    {
        use sha2::{Digest, Sha256};
        let db = f.store.connection().unwrap();
        for (path, body) in [
            ("main.tex", b"Revised manuscript".as_slice()),
            ("new.tex", b"Added manuscript".as_slice()),
        ] {
            let (kind, size, sha): (String, u64, String) = db
                .query_row(
                    "select file_kind,byte_size,sha256 from manuscript_files
                     where manuscript_id=?1 and relative_path=?2",
                    rusqlite::params![f.manuscript.to_string(), path],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(kind, "tex");
            assert_eq!(size, body.len() as u64);
            assert_eq!(sha, format!("{:x}", Sha256::digest(body)));
        }
        let removed: bool = db
            .query_row(
                "select exists(select 1 from manuscript_files
                 where manuscript_id=?1 and relative_path='remove.tex')",
                [f.manuscript.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!removed);
    }
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; C04/C05 lost index ACK reconciliation"]
async fn actual_package_disk_ack_failure_keeps_original_proof_and_reconciles_without_rewriting() {
    let (f, binding, scope) = proposal().await;
    begin(&f.store, &scope).await;
    f.store.connection().unwrap().execute_batch("create trigger synthetic_agent_index_ack before insert on manuscript_files when new.relative_path='main.tex' begin select raise(abort,'synthetic Agent index failure'); end;").unwrap();
    let error = serde_json::to_value(
        apply(&f.store, &scope, "main.tex")
            .await
            .err()
            .expect("index ACK failure"),
    )
    .unwrap();
    assert_eq!(error["code"], "storage_error");
    assert_eq!(error["operationId"], scope.operation_id.to_string());
    assert_eq!(error["saveOutcome"]["status"], "unknown");
    assert_eq!(
        error["dataError"]["resourceId"],
        "writing.projects.file-upsert"
    );
    let root = source_root(&f);
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    let after = std::fs::metadata(root.join("main.tex")).unwrap();
    let failed = state(&f.store, &scope).await;
    assert_eq!(failed["status"], "recoveryRequired");
    // Repeating Apply never redispatches an accepted disk intent.
    assert_eq!(
        serde_json::to_value(apply(&f.store, &scope, "main.tex").await.unwrap()).unwrap(),
        failed
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger synthetic_agent_index_ack")
        .unwrap();
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let unavailable = commands::execute(
        &missing,
        Request::ReconcileDisk {
            scope: scope.clone(),
            path: "main.tex".into(),
        },
    )
    .await
    .err()
    .expect("reconciliation keeps its proof when the provider is absent");
    let unavailable = serde_json::to_value(unavailable).unwrap();
    assert_eq!(unavailable["code"], "plugin_host_package_unavailable");
    assert_eq!(unavailable["operationId"], scope.operation_id.to_string());
    assert_eq!(unavailable["saveOutcome"]["status"], "unknown");
    assert_eq!(state(&f.store, &scope).await, failed);
    let reconciled = commands::execute(
        &f.store,
        Request::ReconcileDisk {
            scope: scope.clone(),
            path: "main.tex".into(),
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(reconciled).unwrap();
    let main = value["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "main.tex")
        .unwrap();
    assert_eq!(main["phase"], "acknowledged");
    use std::os::unix::fs::MetadataExt;
    let checked = std::fs::metadata(root.join("main.tex")).unwrap();
    assert_eq!(
        (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec()
        ),
        (
            checked.dev(),
            checked.ino(),
            checked.len(),
            checked.mtime(),
            checked.mtime_nsec()
        )
    );
    assert_eq!(
        std::fs::read(root.join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
