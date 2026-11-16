//! Owned waiting, rollback and revalidation use the production Core storage path.
use super::*;
use crate::runtime_plugins::{
    literature_derived_view::{
        DerivedError, DerivedPolicy, DerivedRequest, DerivedResult, DerivedSql,
    },
    literature_matrix::{MatrixError, MatrixPolicy, MatrixSql},
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    derived_view::{
        DerivedResearchViewDraft, DerivedValidationInputV1, manual::DerivedViewDecisionV1,
    },
    evidence_matrix::MatrixValidationInputV1,
};
use std::{fs, sync::Arc};
struct Static;
impl MatrixPolicy for Static {
    fn validate(&mut self, input: MatrixValidationInputV1) -> Result<(), MatrixError> {
        match input {
            MatrixValidationInputV1::Matrix { draft } => {
                assert_eq!(draft.project_id, "matrix-project");
            }
            MatrixValidationInputV1::Cell { draft, matrix } => {
                assert_eq!(draft.paper_id, "paper");
                assert_eq!(matrix.project_id, "matrix-project");
            }
            MatrixValidationInputV1::Statement { statement, .. } => {
                assert_eq!(statement.id, "statement-one");
            }
        }
        Ok(())
    }
}
impl DerivedPolicy for Static {
    fn validate(&mut self, input: DerivedValidationInputV1) -> Result<(), DerivedError> {
        match input {
            DerivedValidationInputV1::Header { draft } => {
                assert_eq!(draft.project_id, "matrix-project");
                assert_eq!(draft.title, "Manual comparison");
            }
            DerivedValidationInputV1::Input { ordinal, input } => {
                assert_eq!(ordinal, 0);
                assert_eq!(input.object_kind, "matrix_cell");
            }
            DerivedValidationInputV1::Coverage { ordinal, snapshot } => {
                assert_eq!(ordinal, 0);
                assert_eq!(snapshot.paper_id, "paper");
            }
            DerivedValidationInputV1::Item { ordinal, item, .. } => {
                assert_eq!(ordinal, 0);
                assert_eq!(item.id, "i");
            }
            DerivedValidationInputV1::Provenance { .. } => {
                panic!("manual fixture preparation has no AI review stage")
            }
        }
        Ok(())
    }
    fn provenance(&mut self, current: String, review: String) -> Result<String, DerivedError> {
        assert_eq!(current, "human_original");
        assert_eq!(review, "accepted");
        Ok("human_original".into())
    }
}
fn ready() -> CatalogOutput {
    CatalogOutput::DerivedViewDecision {
        decision: DerivedViewDecisionV1::Validated { rejection: None },
    }
}
fn setup(update: bool, gate: usize) -> (Fixture, DerivedResearchViewDraft) {
    let mut f = Fixture::new();
    matrix_fixture::seed(&f);
    let mut draft = {
        let mut db = f.store.connection().unwrap();
        let sql = MatrixSql::for_test(&super::analysis_workflow_waits::fixtures::MATRIX_SQL);
        let matrix = crate::runtime_plugins::literature_matrix::save_matrix_from(
            &sql,
            &mut Static,
            false,
            &mut db,
            matrix_fixture::draft(),
        )
        .unwrap();
        let mut cell = matrix_fixture::cell(&matrix.id);
        cell.coverage_state = "populated".into();
        cell.statements = vec![matrix_fixture::statement()];
        cell.statements[0].evidence_anchor_ids = vec!["matrix-first-anchor".into()];
        let cell = crate::runtime_plugins::literature_matrix::save_matrix_cell_from(
            &sql,
            &mut Static,
            false,
            &mut db,
            cell,
        )
        .unwrap();
        let mut draft:DerivedResearchViewDraft=serde_json::from_value(json!({"projectId":"matrix-project","viewKind":"taxonomy","title":"Manual comparison","inputs":[{"objectKind":"matrix_cell","objectId":cell.id,"objectRevision":1,"selectionReason":"Chosen"}],"items":[{"id":"i","text":"Synthetic synthesis","epistemicRole":"synthesis","evidenceAnchorIds":["matrix-first-anchor"],"hasConflict":true}],"reviewState":"accepted","changeReason":"manual"})).unwrap();
        if update {
            let sql = DerivedSql::for_test(&super::analysis_workflow_waits::fixtures::DERIVED_SQL);
            let view = crate::runtime_plugins::literature_derived_view::save_derived_view_from(
                &sql,
                &mut Static,
                false,
                &mut db,
                draft.clone(),
            )
            .unwrap();
            draft.id = Some(view.id);
            draft.expected_revision = Some(1);
        }
        draft
    };
    let directory = f.package.path().join("writing");
    let path = directory.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.derived-views","version":1}));
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in [
        "ids",
        "insert",
        "header",
        "update",
        "revision",
        "view",
        "input-snapshot",
    ] {
        let file = format!("derived-view-{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    let mut outputs = vec![ready(), ready(), ready(), ready()];
    if update {
        outputs.push(CatalogOutput::DerivedViewDecision {
            decision: DerivedViewDecisionV1::Provenance {
                value: "human_original".into(),
            },
        });
    }
    f.configure(&outputs, Some(gate), false);
    draft.title = "Changed title".into();
    (f, draft)
}
fn start(
    f: &Fixture,
    draft: DerivedResearchViewDraft,
) -> tokio::task::JoinHandle<crate::runtime_plugins::native::NativeResult<DerivedResult>> {
    let service = Arc::clone(&f.service);
    let db = f.store.shared_agent_connection();
    let key = f.key.clone();
    tokio::spawn(async move {
        service
            .derived_views(
                db,
                key,
                DerivedRequest::Save {
                    draft: Box::new(draft),
                },
            )
            .await
    })
}
async fn finish(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
#[tokio::test]
async fn derived_wait_releases_guard_and_rechecks_historical_input_scope() {
    let (f, draft) = setup(false, 2);
    let cell = draft.inputs[0].object_id.clone();
    let task = start(&f, draft);
    assert!(f.entered(2).await);
    let store = f.store.clone();
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let db = store.connection().unwrap();
            assert!(db.is_autocommit());
            db.execute("insert into research_projects(id,title,summary,created_at) values('other','Other synthetic project','','now')", []).unwrap();
            db.execute(
                "update evidence_matrices set project_id='other' where id=(select matrix_id from evidence_matrix_cells where id=?1)",
                [cell],
            )
            .unwrap();
        }),
    )
    .await
    .unwrap()
    .unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("派生视图输入修订不在项目范围内"));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from derived_research_views", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    finish(f).await;
}
#[tokio::test]
async fn derived_provenance_wait_rolls_back_before_releasing_guard_and_rechecks_revision() {
    let (f, draft) = setup(true, 4);
    let id = draft.id.clone().unwrap();
    let task = start(&f, draft);
    assert!(f.entered(4).await);
    let store = f.store.clone();
    tokio::time::timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            let db = store.connection().unwrap();
            assert!(db.is_autocommit());
            db.execute(
                "update derived_research_views set current_revision=2 where id=?1",
                [id],
            )
            .unwrap();
        }),
    )
    .await
    .unwrap()
    .unwrap();
    f.release();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(2));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from derived_research_view_revisions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    finish(f).await;
}
#[tokio::test]
async fn derived_list_uses_fixed_reads_without_starting_a_child() {
    let (f, _) = setup(false, 0);
    let result = f
        .service
        .derived_views(
            f.store.shared_agent_connection(),
            f.key.clone(),
            DerivedRequest::List {
                project_id: "matrix-project".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(result,DerivedResult::Views(views) if views.is_empty()));
    assert!(f.service.host.lock().await.is_none());
    finish(f).await;
}
#[tokio::test]
async fn derived_shutdown_cancels_validation_before_any_write() {
    let (f, draft) = setup(false, 0);
    let task = start(&f, draft);
    assert!(f.entered(0).await);
    let stopping = f.service.stopped.notified();
    tokio::pin!(stopping);
    stopping.as_mut().enable();
    let service = Arc::clone(&f.service);
    let shutdown = tokio::spawn(async move { service.shutdown().await });
    tokio::time::timeout(WAIT, &mut stopping).await.unwrap();
    f.release();
    assert_eq!(
        task.await.unwrap().unwrap_err().code,
        "plugin_host_stopping"
    );
    tokio::time::timeout(WAIT, shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from derived_research_views", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
async fn derived_caller_loss_keeps_the_owned_save_until_settled() {
    let (f, draft) = setup(false, 0);
    let task = start(&f, draft);
    assert!(f.entered(0).await);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            let count: i64 = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "select count(*) from derived_research_view_revisions",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if count == 1 {
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
async fn derived_response_family_mismatch_cannot_write() {
    let (mut f, draft) = setup(false, 0);
    f.configure(
        &[CatalogOutput::DerivedViewDecision {
            decision: DerivedViewDecisionV1::Provenance {
                value: "human_original".into(),
            },
        }],
        None,
        false,
    );
    let error = start(&f, draft).await.unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_response_mismatch");
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from derived_research_views", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    finish(f).await;
}

#[tokio::test]
async fn derived_provenance_pause_with_unconfirmed_rollback_fences_without_host_wait() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let (f, draft) = setup(true, 4);
    let id = draft.id.clone().unwrap();
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = tokio::time::timeout(WAIT, start(&f, draft))
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let diagnostic = error.derived_view.as_ref().unwrap();
    assert_eq!(
        diagnostic.outcome,
        crate::hbs_storage::boundary::HbsOutcome::Unknown
    );
    assert_eq!(diagnostic.object_id.as_deref(), Some(id.as_str()));
    assert!(diagnostic.rollback_error.is_some());
    assert!(f.store.connection().is_err());
    assert!(!f.lifecycle().lines().any(|line| line == "call 4"));
    f.service.shutdown().await.unwrap();
    assert!(
        f.store
            .shared_agent_connection()
            .shutdown()
            .unwrap_err()
            .to_string()
            .contains("data_recovery_required")
    );
}
