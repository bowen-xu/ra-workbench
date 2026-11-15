//! Production Core and the independently built matrix capability, with synthetic data only.
use super::*;
use crate::runtime_plugins::literature_matrix::{MatrixRequest, MatrixResult};
use research_workbench_native_sdk::evidence_matrix::{
    EvidenceMatrix, EvidenceMatrixCell, EvidenceMatrixCellDraft, EvidenceMatrixDraft,
};

use super::super::matrix_fixture::{cell, draft, seed, statement};
pub(super) async fn fixture() -> Fixture {
    let (catalog, key) = actual_package();
    let mut f = Fixture::new();
    seed(&f);
    f.bind_actual_package(catalog, key);
    f
}
async fn call(
    f: &Fixture,
    request: MatrixRequest,
) -> crate::runtime_plugins::native::NativeResult<MatrixResult> {
    f.service
        .evidence_matrix(f.store.shared_agent_connection(), f.key.clone(), request)
        .await
}
async fn save(
    f: &Fixture,
    draft: EvidenceMatrixDraft,
) -> crate::runtime_plugins::native::NativeResult<EvidenceMatrix> {
    match call(
        f,
        MatrixRequest::Save {
            draft: Box::new(draft),
        },
    )
    .await?
    {
        MatrixResult::Matrix(value) => Ok(value),
        _ => panic!("wrong matrix result"),
    }
}
async fn save_cell(
    f: &Fixture,
    draft: EvidenceMatrixCellDraft,
) -> crate::runtime_plugins::native::NativeResult<EvidenceMatrixCell> {
    match call(
        f,
        MatrixRequest::SaveCell {
            draft: Box::new(draft),
        },
    )
    .await?
    {
        MatrixResult::Cell(value) => Ok(value),
        _ => panic!("wrong cell result"),
    }
}
pub(super) fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from evidence_matrices),(select count(*) from evidence_matrix_revisions),(select count(*) from evidence_matrix_cells),(select count(*) from evidence_matrix_cell_revisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
pub(super) async fn close(f: Fixture) {
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_three_states_reordering_immutable_revisions_and_reopen() {
    let f = fixture().await;
    let created = save(&f, draft()).await.unwrap();
    assert_eq!(created.title, "Compare Ω");
    let first = save_cell(&f, cell(&created.id)).await.unwrap();
    assert_eq!(first.coverage_state, "unreviewed");
    let mut changed = cell(&created.id);
    changed.coverage_state = "not_found".into();
    changed.coverage_note = " No result Ω ".into();
    changed.expected_revision = Some(1);
    let absence = save_cell(&f, changed.clone()).await.unwrap();
    assert_eq!(absence.coverage_note, "No result Ω");
    assert!(absence.statements.is_empty());
    changed.coverage_state = "populated".into();
    changed.statements.push(statement());
    changed.expected_revision = Some(2);
    let populated = save_cell(&f, changed.clone()).await.unwrap();
    assert_eq!(populated.revision, 3);
    assert_eq!(populated.provenance_kind, "human_original");
    assert_eq!(populated.statements, changed.statements);
    let mut reordered = draft();
    reordered.id = Some(created.id.clone());
    reordered.expected_revision = Some(1);
    reordered.paper_ids.reverse();
    reordered.dimensions.reverse();
    let updated = save(&f, reordered).await.unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.dimensions[0].id, "method");
    assert_eq!(updated.paper_ids[0], "matrix-second");
    assert_eq!(counts(&f), (1, 2, 1, 3));
    let original:String=f.store.connection().unwrap().query_row("select snapshot_json from evidence_matrix_cell_revisions where cell_id=?1 and revision=1",[&first.id],|r|r.get(0)).unwrap();
    let old: EvidenceMatrixCellDraft = serde_json::from_str(&original).unwrap();
    assert_eq!(old.coverage_state, "unreviewed");
    assert!(old.statements.is_empty());
    let f = reopen_actual_fixture(f, false).await;
    let MatrixResult::Matrices(matrices) = call(
        &f,
        MatrixRequest::List {
            project_id: "matrix-project".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong list")
    };
    assert_eq!(matrices, vec![updated]);
    let MatrixResult::Cells(cells) = call(
        &f,
        MatrixRequest::ListCells {
            matrix_id: created.id,
        },
    )
    .await
    .unwrap() else {
        panic!("wrong cells")
    };
    assert_eq!(cells, vec![populated]);
    assert_eq!(counts(&f), (1, 2, 1, 3));
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_preserves_membership_cas_and_interleaved_error_order() {
    let f = fixture().await;
    let created = save(&f, draft()).await.unwrap();
    let mut stale = draft();
    stale.id = Some(created.id.clone());
    stale.expected_revision = Some(9);
    let error = save(&f, stale).await.unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(1));
    let mut outside = draft();
    outside.project_id = "missing-project".into();
    let error = save(&f, outside).await.unwrap_err();
    assert!(error.message.contains("当前项目中的文献"));
    let mut body = cell(&created.id);
    body.coverage_state = "populated".into();
    let mut wrong = statement();
    wrong.evidence_anchor_ids = vec!["matrix-second-anchor".into()];
    body.statements = vec![wrong, statement()];
    body.statements[1].text.clear();
    // The first source membership failure precedes the later malformed statement.
    let error = save_cell(&f, body.clone()).await.unwrap_err();
    assert!(error.message.contains("当前单元的论文"));
    body.statements[0] = statement();
    let error = save_cell(&f, body).await.unwrap_err();
    assert!(error.message.contains("矩阵陈述参数无效"));
    for state in ["unreviewed", "not_found"] {
        let mut invalid = cell(&created.id);
        invalid.coverage_state = state.into();
        invalid.statements.push(statement());
        let error = save_cell(&f, invalid).await.unwrap_err();
        assert!(error.message.contains("单元参数无效"));
    }
    assert_eq!(counts(&f), (1, 1, 0, 0));
    close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_history_failures_roll_back_matrix_and_cell_then_reopen() {
    for table in [
        "evidence_matrix_revisions",
        "evidence_matrix_cell_revisions",
    ] {
        let f = fixture().await;
        let created = if table.ends_with("cell_revisions") {
            Some(save(&f, draft()).await.unwrap())
        } else {
            None
        };
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger synthetic_matrix_history_failure before insert on {table} begin select raise(abort,'private-synthetic-matrix-failure'); end;")).unwrap();
        let error = if let Some(matrix) = created {
            save_cell(&f, cell(&matrix.id)).await.unwrap_err()
        } else {
            save(&f, draft()).await.unwrap_err()
        };
        assert_eq!(error.code, "storage_error");
        assert_eq!(
            error.evidence_matrix.as_ref().unwrap().outcome,
            crate::hbs_storage::boundary::HbsOutcome::RolledBack
        );
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private-synthetic-matrix-failure")
        );
        let expected = if table.ends_with("cell_revisions") {
            (1, 1, 0, 0)
        } else {
            (0, 0, 0, 0)
        };
        assert_eq!(counts(&f), expected);
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(counts(&f), expected);
        close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_unknown_commit_and_rollback_fence_owner_until_reopen() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for rollback in [false, true] {
        let f = fixture().await;
        {
            let db = f.store.connection().unwrap();
            if rollback {
                db.execute_batch("create temp trigger synthetic_matrix_history_failure before insert on evidence_matrix_revisions begin select raise(abort,'private-synthetic-matrix-failure'); end;").unwrap();
                db.authorizer(Some(|context: AuthContext<'_>| match context.action {
                    AuthAction::Transaction {
                        operation: TransactionOperation::Rollback,
                    } => Authorization::Deny,
                    _ => Authorization::Allow,
                }));
            } else {
                db.commit_hook(Some(|| true));
            }
        }
        let error = save(&f, draft()).await.unwrap_err();
        let diagnostic = error.evidence_matrix.as_ref().unwrap();
        uuid::Uuid::parse_str(
            diagnostic
                .object_id
                .as_deref()
                .expect("allocated matrix identity retained"),
        )
        .unwrap();
        assert_eq!(
            diagnostic.outcome,
            crate::hbs_storage::boundary::HbsOutcome::Unknown
        );
        assert_eq!(diagnostic.rollback_error.is_some(), rollback);
        assert!(error.operation_id.is_some());
        assert!(error.message.contains("禁止自动重试"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), (0, 0, 0, 0));
        close(f).await;
    }
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_update_history_failure_preserves_both_original_objects() {
    let f = fixture().await;
    let matrix = save(&f, draft()).await.unwrap();
    let original = save_cell(&f, cell(&matrix.id)).await.unwrap();
    f.store.connection().unwrap().execute_batch("create temp trigger synthetic_matrix_update_failure before insert on evidence_matrix_revisions begin select raise(abort,'private-update-failure'); end;
    create temp trigger synthetic_cell_update_failure before insert on evidence_matrix_cell_revisions begin select raise(abort,'private-update-failure'); end;").unwrap();
    let mut update = draft();
    update.id = Some(matrix.id.clone());
    update.expected_revision = Some(1);
    update.title = "Rejected replacement".into();
    let mut update_cell = cell(&matrix.id);
    update_cell.expected_revision = Some(1);
    update_cell.coverage_state = "not_found".into();
    for error in [
        save(&f, update).await.unwrap_err(),
        save_cell(&f, update_cell).await.unwrap_err(),
    ] {
        assert_eq!(
            error.evidence_matrix.as_ref().unwrap().outcome,
            crate::hbs_storage::boundary::HbsOutcome::RolledBack
        );
    }
    let f = reopen_actual_fixture(f, false).await;
    let MatrixResult::Matrices(matrices) = call(
        &f,
        MatrixRequest::List {
            project_id: "matrix-project".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong matrix list")
    };
    let MatrixResult::Cells(cells) = call(
        &f,
        MatrixRequest::ListCells {
            matrix_id: matrix.id.clone(),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong cell list")
    };
    assert_eq!(matrices, vec![matrix]);
    assert_eq!(cells, vec![original]);
    assert_eq!(counts(&f), (1, 1, 1, 1));
    close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_committed_readback_failure_keeps_actual_identity_and_never_replays()
{
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("create temp trigger synthetic_matrix_readback_failure after insert on evidence_matrix_revisions begin update evidence_matrices set paper_ids_json='{}' where id=new.matrix_id; end;").unwrap();
    let error = save(&f, draft()).await.unwrap_err();
    let diagnostic = error.evidence_matrix.as_ref().unwrap();
    assert_eq!(
        diagnostic.outcome,
        crate::hbs_storage::boundary::HbsOutcome::Committed
    );
    let id = diagnostic.object_id.clone().unwrap();
    uuid::Uuid::parse_str(&id).unwrap();
    assert!(error.message.contains("已提交") && error.message.contains("禁止自动重放"));
    assert_eq!(counts(&f), (1, 1, 0, 0));
    let f = reopen_actual_fixture(f, false).await;
    let stored: (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select id,paper_ids_json from evidence_matrices",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (id, "{}".into()));
    assert_eq!(counts(&f), (1, 1, 0, 0));
    close(f).await;
}
