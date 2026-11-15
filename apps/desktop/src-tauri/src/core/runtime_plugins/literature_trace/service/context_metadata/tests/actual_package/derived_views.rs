//! Original manual management through real Core and the current independent package.
use super::*;
use crate::{
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_derived_view::{DerivedRequest, DerivedResult},
};
use research_workbench_native_sdk::derived_view::{
    DerivedResearchView, DerivedResearchViewDraft, DerivedViewItem,
};

async fn setup() -> (Fixture, DerivedResearchViewDraft) {
    let (f, _, task) = matrix_synthesis::setup().await;
    let draft = DerivedResearchViewDraft {
        id: None,
        project_id: "matrix-project".into(),
        view_kind: "taxonomy".into(),
        title: " Comparison Ω ".into(),
        scope_note: " Two selected cells ".into(),
        inputs: task.matrix_synthesis_context.unwrap().inputs,
        items: vec![DerivedViewItem {
            id: "item-one".into(),
            text: " Original synthesis Ω ".into(),
            epistemic_role: "synthesis".into(),
            evidence_anchor_ids: vec!["matrix-first-anchor".into(), "matrix-second-anchor".into()],
            has_conflict: true,
        }],
        review_state: "unreviewed".into(),
        expected_revision: None,
        change_reason: " manual original ".into(),
    };
    (f, draft)
}
async fn save(
    f: &Fixture,
    draft: DerivedResearchViewDraft,
) -> crate::runtime_plugins::native::NativeResult<DerivedResearchView> {
    match f
        .service
        .derived_views(
            f.store.shared_agent_connection(),
            f.key.clone(),
            DerivedRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        DerivedResult::View(value) => Ok(*value),
        _ => panic!("derived save result"),
    }
}
pub(super) async fn list(f: &Fixture) -> Vec<DerivedResearchView> {
    match f
        .service
        .derived_views(
            f.store.shared_agent_connection(),
            f.key.clone(),
            DerivedRequest::List {
                project_id: "matrix-project".into(),
            },
        )
        .await
        .unwrap()
    {
        DerivedResult::Views(value) => value,
        _ => panic!("derived list result"),
    }
}
fn counts(f: &Fixture) -> (i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from derived_research_views),(select count(*) from derived_research_view_revisions)", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_derived_views_create_update_preserve_history_and_reopen() {
    let (f, mut draft) = setup().await;
    assert!(list(&f).await.is_empty());
    let created = save(&f, draft.clone()).await.unwrap();
    assert_eq!(created.title, "Comparison Ω");
    assert_eq!(created.scope_note, "Two selected cells");
    assert_eq!(created.provenance_kind, "human_original");
    assert!(created.provider_id.is_none());
    assert_eq!(created.inputs, draft.inputs);
    assert_eq!(created.items, draft.items);
    let snapshot: String = f.store.connection().unwrap().query_row("select snapshot_json from derived_research_view_revisions where view_id=?1 and revision=1", [&created.id], |r|r.get(0)).unwrap();
    assert_eq!(snapshot, serde_json::to_string(&draft).unwrap());
    // Changing the current cell does not replace the explicitly selected historical input.
    f.store
        .connection()
        .unwrap()
        .execute(
            "update evidence_matrix_cells set current_revision=2 where id=?1",
            [&draft.inputs[0].object_id],
        )
        .unwrap();
    draft.id = Some(created.id.clone());
    draft.expected_revision = Some(1);
    draft.items[0].text = "Human revision Ω".into();
    draft.review_state = "accepted".into();
    let changed = save(&f, draft.clone()).await.unwrap();
    assert_eq!(changed.revision, 2);
    assert_eq!(changed.inputs, created.inputs);
    assert_eq!(changed.generated_at, created.generated_at);
    assert_eq!(changed.created_at, created.created_at);
    assert_eq!(list(&f).await, vec![changed.clone()]);
    let error = save(&f, draft).await.unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(error.current_revision, Some(2));
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(list(&f).await, vec![changed]);
    assert_eq!(counts(&f), (1, 2));
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_derived_views_preserve_original_validation_order_and_scope() {
    let (f, draft) = setup().await;
    let created = save(&f, draft.clone()).await.unwrap();
    for (kind, message) in [
        (0, "派生研究视图参数无效"),
        (1, "派生视图输入无效"),
        (2, "派生视图输入修订不在项目范围内"),
        (3, "派生视图条目必须由所选输入证据支持"),
        (4, "派生视图不能移动到另一项目"),
        (5, "新派生视图不能携带预期版本"),
    ] {
        let mut invalid = draft.clone();
        match kind {
            0 => {
                invalid.title.clear();
                invalid.inputs[0].object_kind = "other".into();
            }
            1 => invalid.inputs[0].object_kind = "other".into(),
            2 => invalid.inputs[0].object_revision = 999,
            3 => invalid.items[0].evidence_anchor_ids = vec!["unselected-anchor".into()],
            4 => {
                f.store.connection().unwrap().execute("insert into research_projects(id,title,summary,created_at) values('other','Other','','now')", []).unwrap();
                // The original order checks input scope before the view header.
                f.store.connection().unwrap().execute("insert into project_items(project_id,item_id,added_at) values('other','paper','now'),('other','matrix-second','now')", []).unwrap();
                f.store
                    .connection()
                    .unwrap()
                    .execute("update evidence_matrices set project_id='other'", [])
                    .unwrap();
                invalid.project_id = "other".into();
                invalid.id = Some(created.id.clone());
                invalid.expected_revision = Some(1);
            }
            _ => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "update evidence_matrices set project_id='matrix-project'",
                        [],
                    )
                    .unwrap();
                invalid.expected_revision = Some(9);
            }
        }
        let error = save(&f, invalid).await.unwrap_err();
        assert!(error.message.contains(message), "{kind}: {error}");
        assert_eq!(counts(&f), (1, 1));
    }
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_derived_views_history_failure_rolls_back_create_and_update() {
    for update in [false, true] {
        let (f, mut draft) = setup().await;
        let original = if update {
            Some(save(&f, draft.clone()).await.unwrap())
        } else {
            None
        };
        if let Some(view) = &original {
            draft.id = Some(view.id.clone());
            draft.expected_revision = Some(1);
            draft.title = "Rejected title".into();
        }
        f.store.connection().unwrap().execute_batch("create temp trigger synthetic_derived_history_failure before insert on derived_research_view_revisions begin select raise(abort,'private-derived-content'); end;").unwrap();
        let error = save(&f, draft).await.unwrap_err();
        assert_eq!(error.code, "storage_error");
        assert_eq!(
            error.derived_view.as_ref().unwrap().outcome,
            HbsOutcome::RolledBack
        );
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private-derived-content")
        );
        let f = reopen_actual_fixture(f, false).await;
        assert_eq!(list(&f).await, original.into_iter().collect::<Vec<_>>());
        assert_eq!(counts(&f), if update { (1, 1) } else { (0, 0) });
        evidence_matrix::close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_derived_views_unknown_settlement_fences_then_normal_reopen_recovers() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    for rollback in [false, true] {
        let (f, draft) = setup().await;
        {
            let db = f.store.connection().unwrap();
            if rollback {
                db.execute_batch("create temp trigger synthetic_derived_failure before insert on derived_research_view_revisions begin select raise(abort,'private-derived'); end;").unwrap();
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
        let error = save(&f, draft).await.unwrap_err();
        let diagnostic = error.derived_view.as_ref().unwrap();
        assert_eq!(diagnostic.outcome, HbsOutcome::Unknown);
        assert_eq!(diagnostic.rollback_error.is_some(), rollback);
        uuid::Uuid::parse_str(diagnostic.object_id.as_deref().unwrap()).unwrap();
        assert!(error.operation_id.is_some());
        assert!(error.message.contains("禁止自动重试"));
        assert!(f.store.connection().is_err());
        let f = reopen_actual_fixture(f, true).await;
        assert_eq!(counts(&f), (0, 0));
        evidence_matrix::close(f).await;
    }
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_derived_views_committed_readback_keeps_identity_without_replay() {
    let (f, draft) = setup().await;
    f.store.connection().unwrap().execute_batch("create temp trigger synthetic_derived_readback after insert on derived_research_view_revisions begin update derived_research_views set inputs_json='{}' where id=new.view_id; end;").unwrap();
    let error = save(&f, draft).await.unwrap_err();
    let diagnostic = error.derived_view.as_ref().unwrap();
    assert_eq!(diagnostic.outcome, HbsOutcome::Committed);
    let id = diagnostic.object_id.clone().unwrap();
    assert!(error.message.contains("已提交") && error.message.contains("禁止自动重放"));
    assert_eq!(counts(&f), (1, 1));
    let f = reopen_actual_fixture(f, false).await;
    let stored: (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select id,inputs_json from derived_research_views",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, (id, "{}".into()));
    assert_eq!(counts(&f), (1, 1));
    evidence_matrix::close(f).await;
}
pub(super) async fn edit_accepted(f: &Fixture) {
    let before = list(f).await.pop().unwrap();
    assert!(before.provider_id.is_some());
    assert!(before.model_id.is_some());
    let mut draft = DerivedResearchViewDraft {
        id: Some(before.id.clone()),
        project_id: before.project_id.clone(),
        view_kind: before.view_kind.clone(),
        title: before.title.clone(),
        scope_note: before.scope_note.clone(),
        inputs: before.inputs.clone(),
        items: before.items.clone(),
        review_state: "accepted".into(),
        expected_revision: Some(before.revision),
        change_reason: "Human edit after promotion".into(),
    };
    draft.items[0].text = "Reviewed after acceptance Ω".into();
    let after = save(f, draft).await.unwrap();
    assert_eq!(after.provenance_kind, "ai_accepted_after_edit");
    assert_eq!(after.provider_id, before.provider_id);
    assert_eq!(after.model_id, before.model_id);
    assert_eq!(after.generated_at, before.generated_at);
    assert_eq!(after.created_at, before.created_at);
    assert_eq!(after.inputs, before.inputs);
    assert_eq!(after.revision, 2);
    assert_eq!(counts(f), (1, 2));
}
