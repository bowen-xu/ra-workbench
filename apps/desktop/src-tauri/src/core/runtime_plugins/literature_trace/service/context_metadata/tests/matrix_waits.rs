//! Canned replies exercise waiting and owner races; no substitute field validator.
use super::matrix_fixture::{cell, draft, seed, statement};
use super::*;
use crate::runtime_plugins::literature_matrix::{MatrixRequest, MatrixResult};
use research_workbench_native_sdk::catalog::CatalogOutput;
use research_workbench_native_sdk::evidence_matrix::{
    MatrixRejectionV1, MatrixValidationDecisionV1,
};
use std::{fs, sync::Arc};

fn ready() -> CatalogOutput {
    CatalogOutput::EvidenceMatrixValidation {
        decision: MatrixValidationDecisionV1 { rejection: None },
    }
}
fn setup(outputs: &[CatalogOutput], gate: usize) -> Fixture {
    let mut f = Fixture::new();
    seed(&f);
    let directory = f.package.path().join("writing");
    let path = directory.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.evidence-matrix","version":1}));
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in [
        "ids",
        "cell-header",
        "cell-keys",
        "insert",
        "header",
        "update",
        "revision",
        "cell-insert",
        "cell-update",
        "cell-revision",
        "matrix",
        "cell",
        "project-belongs",
        "anchor-belongs",
    ] {
        let file = format!("evidence-matrix-{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    f.configure(outputs, Some(gate), false);
    f
}
fn start(
    f: &Fixture,
    request: MatrixRequest,
) -> tokio::task::JoinHandle<crate::runtime_plugins::native::NativeResult<MatrixResult>> {
    let service = Arc::clone(&f.service);
    let db = f.store.shared_agent_connection();
    let key = f.key.clone();
    tokio::spawn(async move { service.evidence_matrix(db, key, request).await })
}
async fn matrix(f: &Fixture) -> String {
    match start(
        f,
        MatrixRequest::Save {
            draft: Box::new(draft()),
        },
    )
    .await
    .unwrap()
    .unwrap()
    {
        MatrixResult::Matrix(value) => value.id,
        _ => panic!("wrong matrix result"),
    }
}
async fn finish(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
async fn matrix_wait_releases_core_guard_and_rejects_changed_matrix_before_cell_write() {
    let f = setup(&[ready(), ready()], 1);
    let id = matrix(&f).await;
    let task = start(
        &f,
        MatrixRequest::SaveCell {
            draft: Box::new(cell(&id)),
        },
    );
    assert!(f.entered(1).await);
    let store = f.store.clone();
    let changed = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update evidence_matrices set title='Concurrent title' where id=?1",
            [id],
        )
        .unwrap();
    });
    tokio::time::timeout(WAIT, changed).await.unwrap().unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_source_changed");
    assert!(error.request_id.is_some());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from evidence_matrix_cells", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    finish(f).await;
}
#[tokio::test]
async fn matrix_later_statement_wait_rechecks_prior_anchor_before_any_write() {
    let f = setup(&[ready(), ready(), ready(), ready()], 3);
    let id = matrix(&f).await;
    let mut draft = cell(&id);
    draft.coverage_state = "populated".into();
    let first = statement();
    let mut second = first.clone();
    second.id = "second".into();
    draft.statements = vec![first, second];
    let task = start(
        &f,
        MatrixRequest::SaveCell {
            draft: Box::new(draft),
        },
    );
    assert!(f.entered(3).await);
    let store = f.store.clone();
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            store
                .connection()
                .unwrap()
                .execute(
                    "delete from source_anchors where id='matrix-first-anchor'",
                    [],
                )
                .unwrap()
        }),
    )
    .await
    .unwrap()
    .unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("当前单元的论文"));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from evidence_matrix_cells", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    finish(f).await;
}
#[tokio::test]
async fn matrix_rejects_wrong_rule_reply_without_writes_or_static_fallback() {
    let output = CatalogOutput::EvidenceMatrixValidation {
        decision: MatrixValidationDecisionV1 {
            rejection: Some(MatrixRejectionV1::Cell),
        },
    };
    let f = setup(&[output], 0);
    let task = start(
        &f,
        MatrixRequest::Save {
            draft: Box::new(draft()),
        },
    );
    assert!(f.entered(0).await);
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert!(error.request_id.is_some());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from evidence_matrices", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    finish(f).await;
}
#[tokio::test]
async fn matrix_caller_loss_retains_owned_operation_until_original_save_finishes() {
    let f = setup(&[ready()], 0);
    let task = start(
        &f,
        MatrixRequest::Save {
            draft: Box::new(draft()),
        },
    );
    assert!(f.entered(0).await);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            if f.store
                .connection()
                .unwrap()
                .query_row("select count(*) from evidence_matrix_revisions", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
                == 1
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    finish(f).await;
}

#[tokio::test]
async fn matrix_shutdown_cancels_pending_validation_before_the_original_writer() {
    let f = setup(&[ready()], 0);
    let task = start(
        &f,
        MatrixRequest::Save {
            draft: Box::new(draft()),
        },
    );
    assert!(f.entered(0).await);
    let stopping = f.service.stopped.notified();
    tokio::pin!(stopping);
    stopping.as_mut().enable();
    let service = Arc::clone(&f.service);
    let shutdown = tokio::spawn(async move { service.shutdown().await });
    tokio::time::timeout(WAIT, &mut stopping).await.unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_stopping");
    tokio::time::timeout(WAIT, shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from evidence_matrices", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.store.shared_agent_connection().shutdown().unwrap();
}
