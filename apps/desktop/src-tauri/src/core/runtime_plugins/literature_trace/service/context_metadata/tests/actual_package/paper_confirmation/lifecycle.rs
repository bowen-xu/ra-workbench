//! Four original App entries against Core storage and the current independent package.
use super::*;
use crate::runtime_plugins::literature_paper_confirmation::ConfirmationResult;
use rusqlite::params;

pub(super) async fn execute(
    f: &Fixture,
    request: Request,
) -> Result<ConfirmationResult, crate::ai::error::AiError> {
    crate::paper_confirmation_app::execute_selected(&f.store, request).await
}
pub(super) fn overview(result: ConfirmationResult) -> AiOverviewConfirmationResult {
    match result {
        ConfirmationResult::Overview(value) => *value,
        _ => panic!("overview result"),
    }
}
pub(super) fn paper(result: ConfirmationResult) -> AiPaperReviewConfirmationResult {
    match result {
        ConfirmationResult::Paper(value) => *value,
        _ => panic!("paper result"),
    }
}
pub(super) fn field_request(candidate: &str, field: &str, key: &str) -> Request {
    Request::ConfirmOverview {
        draft: AiOverviewFieldConfirmationDraft {
            candidate_id: candidate.into(),
            candidate_revision: 1,
            field_kinds: vec![field.into()],
            scope_kind: "field".into(),
            project_id: None,
            idempotency_key: key.into(),
        },
    }
}
pub(super) fn body(f: &Fixture, id: &str) -> (String, i64) {
    f.store.connection().unwrap().query_row("select body,revision from structured_reading_units where id=?1 and reverted_at is null",[id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap()
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_four_confirmation_commands_preserve_partial_modules_reset_restore_and_reopen()
 {
    let mut f = Fixture::new();
    let candidate = seed(&f);
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into structured_reading_units(id,project_id,paper_id,unit_kind,title,body,provenance_kind,epistemic_role,revision,created_at,updated_at) values('original-human-unit',null,'paper','approach','Human title','Human body Ω','human_original','working_note',1,'old','old')",[]).unwrap();
        db.execute("insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at) values('paper','title','crossref',0,'metadata-old')",[]).unwrap();
        // The first module genuinely fails. A later module then makes that source available.
        // Re-entering the entire paper loop would retry it and lose the original failure.
        db.execute_batch("create trigger confirmation_late_metadata after update of confirmed_by_user on metadata_provenance when new.item_id='paper' and new.field_name='title' and new.confirmed_by_user=1 begin insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at) values('paper','abstractText','crossref',0,'late-source'); end").unwrap();
    }
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    assert!(f.store.generation_policy_source().is_none());
    let draft = AiPaperReviewConfirmationDraft {
        paper_id: "paper".into(),
        project_id: Some("original-project-context".into()),
        metadata_field_names: vec!["title".into(), "abstractText".into()],
        overview_targets: vec![AiPaperReviewOverviewTargetDraft {
            candidate_id: candidate.id.clone(),
            candidate_revision: 1,
            field_kinds: vec!["approach".into()],
        }],
        idempotency_key: "whole-paper-key".into(),
    };
    let first = paper(
        execute(
            &f,
            Request::ConfirmPaper {
                draft: draft.clone(),
            },
        )
        .await
        .unwrap(),
    );
    assert_eq!(first.module_failures.len(), 1);
    assert!(first.module_failures[0].contains("abstractText"));
    assert!(first.module_failures[0].contains("字段没有可确认的来源"));
    assert_eq!(first.batch.metadata_confirmations.len(), 1);
    assert_eq!(first.batch.overview_batch_ids.len(), 1);
    assert_eq!(
        body(&f, "original-human-unit"),
        ("Original method Ω".into(), 2)
    );
    {
        let db = f.store.connection().unwrap();
        let confirmed:bool=db.query_row("select confirmed_by_user from metadata_provenance where item_id='paper' and field_name='abstractText'",[],|row|row.get(0)).unwrap();
        assert!(
            !confirmed,
            "the failed module is retained until an explicit retry"
        );
        let quote:String=db.query_row("select a.quote_exact from reading_unit_evidence_links l join source_anchors a on a.id=l.anchor_id where l.reading_unit_id='original-human-unit'",[],|row|row.get(0)).unwrap();
        assert_eq!(quote, "evidence Ω");
        let project: Option<String> = db
            .query_row(
                "select project_id from structured_reading_units where id='original-human-unit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(project.is_none());
    }
    let repeated = paper(execute(&f, Request::ConfirmPaper { draft }).await.unwrap());
    assert_eq!(repeated.batch.id, first.batch.id);
    assert!(repeated.module_failures.is_empty());
    assert_eq!(repeated.batch.metadata_confirmations.len(), 2);
    assert_eq!(body(&f, "original-human-unit").1, 2);
    let undo = Request::UndoPaper {
        draft: AiPaperReviewConfirmationUndoDraft {
            batch_id: first.batch.id.clone(),
            idempotency_key: "whole-paper-undo".into(),
        },
    };
    let undone = paper(execute(&f, undo.clone()).await.unwrap());
    assert!(undone.module_failures.is_empty());
    assert!(!undone.batch.active);
    assert_eq!(body(&f, "original-human-unit"), ("Human body Ω".into(), 3));
    assert_eq!(
        serde_json::to_value(paper(execute(&f, undo.clone()).await.unwrap())).unwrap(),
        serde_json::to_value(&undone).unwrap()
    );

    let conclusion = overview(
        execute(
            &f,
            field_request(&candidate.id, "conclusion", "single-conclusion"),
        )
        .await
        .unwrap(),
    );
    let target = &conclusion.batch.field_confirmations[0].target_reading_unit_id;
    let before_reset = body(&f, target);
    let reset = Request::UndoOverview {
        draft: AiOverviewConfirmationUndoDraft {
            batch_id: conclusion.batch.id.clone(),
            field_kind: Some("conclusion".into()),
            idempotency_key: "reset-conclusion-mark".into(),
        },
    };
    let reset_result = overview(execute(&f, reset.clone()).await.unwrap());
    assert!(!reset_result.batch.active);
    assert_eq!(body(&f, target), before_reset);
    assert_eq!(
        serde_json::to_value(overview(execute(&f, reset).await.unwrap())).unwrap(),
        serde_json::to_value(&reset_result).unwrap()
    );

    let approach = overview(
        execute(
            &f,
            field_request(&candidate.id, "approach", "single-approach"),
        )
        .await
        .unwrap(),
    );
    let applied = approach.batch.field_confirmations[0].target_applied_revision;
    assert_eq!(applied, 4);
    let direct_undo = Request::UndoOverview {
        draft: AiOverviewConfirmationUndoDraft {
            batch_id: approach.batch.id.clone(),
            field_kind: None,
            idempotency_key: "undo-single-approach".into(),
        },
    };
    f.store.connection().unwrap().execute("update structured_reading_units set revision=revision+1 where id='original-human-unit'",[]).unwrap();
    let conflict = execute(&f, direct_undo.clone())
        .await
        .err()
        .expect("later revision rejects undo");
    assert!(conflict.to_string().contains("版本冲突"));
    f.store
        .connection()
        .unwrap()
        .execute(
            "update structured_reading_units set revision=?1 where id='original-human-unit'",
            [applied],
        )
        .unwrap();
    let restored = overview(execute(&f, direct_undo.clone()).await.unwrap());
    assert!(!restored.batch.active);
    assert_eq!(body(&f, "original-human-unit"), ("Human body Ω".into(), 5));
    let audit_count: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select count(*) from ai_overview_field_review_resets",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit_count, 1);
    let before = serde_json::to_value(&restored).unwrap();
    f.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
    drop(f.store);
    f.store = crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: f._data.path().into(),
            agent_configuration_directory: f._data.path().join("configuration"),
        },
        crate::storage::AppStorePorts::with_events(
            crate::runtime_events::RuntimeEvents::new(|_, _| Ok(())),
            crate::storage::AppStoreSources::default(),
        ),
    )
    .unwrap();
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    assert_eq!(body(&f, target), before_reset);
    assert_eq!(body(&f, "original-human-unit"), ("Human body Ω".into(), 5));
    assert_eq!(
        serde_json::to_value(overview(execute(&f, direct_undo).await.unwrap())).unwrap(),
        before
    );
    assert!(!paper(execute(&f, undo).await.unwrap()).batch.active);
    let active_metadata:i64=f.store.connection().unwrap().query_row("select count(*) from metadata_provenance where item_id=?1 and field_name in('title','abstractText') and confirmed_by_user=1",params!["paper"],|row|row.get(0)).unwrap();
    assert_eq!(active_metadata, 0);
    f.service.shutdown().await.unwrap();
}
