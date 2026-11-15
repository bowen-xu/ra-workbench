use super::*;
use crate::{
    ai::structured_resource_validation::FailureCause,
    hbs_storage::boundary::{HbsNeed, HbsOutcome},
    structured_resources::{HistoryDraft, history::restore_with},
};
use serde_json::json;
#[path = "runtime_tests/fixture.rs"]
mod fixture;
use fixture::*;

#[test]
fn native_keep_pauses_before_writes_and_preserves_protected_fields_without_reading_sources() {
    let mut db = database();
    let target = reading(&db);
    let initial = save(
        &mut db,
        &target,
        body("原问题。").to_string(),
        "agent",
        "initial",
    )
    .resource;
    let kept = decision(&mut db, &initial, "keep", None);
    let verified = decision(
        &mut db,
        &kept,
        "verify",
        Some(vec!["research_question".into()]),
    );
    let mut changed = body("待审的新问题。");
    changed["fields"][1] = json!({"field":"approach","status":"filled","statements":[{"id":"approach-1","text":"尚待核查的方法。","epistemicRole":"uncertain","evidenceItemIds":[]}]});
    let edited = save(&mut db, &verified, changed.to_string(), "agent", "edited").resource;
    let input = request(&edited, "native-keep");
    let before = snapshot(&db, &edited.id);
    let changes = db.total_changes();
    let mut sources = Sources::new();
    sources.ready = false;
    sources.policy.selection_pause = true;
    let BoundaryError::Pause(paused) =
        review_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap_err()
    else {
        panic!("expected a selection pause")
    };
    assert_eq!(paused.outcome(), HbsOutcome::RolledBack);
    let HbsNeed::Unreviewed {
        input: selected, ..
    } = paused.need()
    else {
        panic!("expected the actual selection input")
    };
    assert_eq!(selected.resource_id, edited.id);
    assert_eq!(selected.resource_revision, edited.revision);
    assert_eq!(selected.pending_field_ids, edited.pending_field_ids);
    assert!(
        selected
            .protected_field_ids
            .contains(&"research_question".into())
    );
    assert_eq!(selected.reviewed_field_ids, edited.reviewed_field_ids);
    assert!(db.is_autocommit());
    assert_eq!(db.total_changes(), changes);
    assert_eq!(snapshot(&db, &edited.id), before);
    assert_eq!(sources.requires, 0);

    sources.policy.selection_pause = false;
    let accepted = review_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap();
    assert!(
        accepted
            .pending_field_ids
            .contains(&"research_question".into())
    );
    assert!(!accepted.pending_field_ids.contains(&"approach".into()));
    assert_eq!(accepted.reviewed_field_ids, edited.reviewed_field_ids);
    assert_eq!(accepted.content, edited.content);
    assert_eq!(sources.requires, 0);
    let calls = sources.policy.selected.len();
    sources.policy.selection_pause = true;
    let replayed = review_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap();
    assert_eq!(replayed.revision, accepted.revision);
    assert_eq!(sources.policy.selected.len(), calls);
    let BoundaryError::Failure(stale) = review_with(
        &mut sources,
        &mut db,
        ReviewDraft {
            request_id: "stale".into(),
            ..input.clone()
        },
        ReviewRoute::Native,
    )
    .unwrap_err() else {
        panic!("CAS must precede selection")
    };
    assert!(matches!(stale.failure.error, Error::Conflict));
    let BoundaryError::Failure(reused) = review_with(
        &mut sources,
        &mut db,
        ReviewDraft {
            scope: Some("all".into()),
            ..input
        },
        ReviewRoute::Native,
    )
    .unwrap_err() else {
        panic!("request identity must precede admission")
    };
    assert!(matches!(reused.failure.error, Error::Invalid(_)));
    assert_eq!(sources.policy.selected.len(), calls);
}

#[test]
fn native_review_admission_checks_real_kind_and_exact_operation_including_receipt_replay() {
    let mut db = database();
    let target = reading(&db);
    let mut sources = Sources::new();
    for (action, scope, fields) in [
        ("undo", Some("unreviewed"), None),
        ("undoGeneration", Some("unreviewed"), None),
        ("verify", Some("unreviewed"), None),
        ("resetVerification", Some("unreviewed"), None),
        ("clear", None, None),
        ("keep", Some("all"), None),
        ("keep", None, None),
        ("keep", Some("unreviewed"), Some(vec![])),
    ] {
        let before = snapshot(&db, &target.id);
        let changes = db.total_changes();
        let input = ReviewDraft {
            action: action.into(),
            scope: scope.map(str::to_owned),
            field_ids: fields,
            ..request(&target, &uuid::Uuid::new_v4().to_string())
        };
        let BoundaryError::Failure(error) =
            review_with(&mut sources, &mut db, input, ReviewRoute::Native).unwrap_err()
        else {
            panic!("unsupported operation must be refused")
        };
        assert_eq!(error.outcome, HbsOutcome::RolledBack);
        assert!(matches!(error.failure.error, Error::Invalid(_)));
        assert_eq!(db.total_changes(), changes);
        assert_eq!(snapshot(&db, &target.id), before);
    }
    let template =
        crate::structured_resources::create_template(&db, "原模板", "template", None).unwrap();
    let input = request(&template, "static-template-receipt");
    crate::hbs_storage::test_sources::review(&mut db, input.clone()).unwrap();
    let before = snapshot(&db, &template.id);
    assert!(matches!(
        review_with(&mut sources, &mut db, input, ReviewRoute::Native),
        Err(BoundaryError::Failure(_))
    ));
    assert_eq!(snapshot(&db, &template.id), before);
    assert!(sources.policy.selected.is_empty());
    for kind in ["portrait", "structuredReading"] {
        let resource =
            crate::structured_resources::find_by_paper(&db, "review-paper", kind).unwrap();
        let accepted = review_with(
            &mut sources,
            &mut db,
            request(&resource, kind),
            ReviewRoute::Native,
        )
        .unwrap();
        assert_eq!(accepted.kind, kind);
        assert_eq!(accepted.revision, resource.revision + 1);
    }
    assert_eq!(sources.requires, 0);
}

fn history(resource: &Resource, operation: i64, direction: &str, id: &str) -> HistoryDraft {
    HistoryDraft {
        resource_id: resource.id.clone(),
        expected_revision: resource.revision,
        operation_revision: operation,
        direction: direction.into(),
        request_id: id.into(),
    }
}

#[test]
fn native_history_pauses_and_rejects_before_writes_then_restores_trusted_human_state() {
    let mut db = database();
    let target = reading(&db);
    let generated = save(
        &mut db,
        &target,
        body("原正文。").to_string(),
        "agent",
        "initial",
    )
    .resource;
    let clean = decision(&mut db, &generated, "keep", None);
    let edited = save(
        &mut db,
        &clean,
        body("人工正文。").to_string(),
        "human",
        "human",
    )
    .resource;
    let repair = save(&mut db, &edited, "{invalid".into(), "human", "repair")
        .draft
        .unwrap();
    let input = history(&edited, edited.revision, "undo", "undo-human");
    let before = snapshot(&db, &edited.id);
    let changes = db.total_changes();
    let mut sources = Sources::new();
    sources.ready = false;
    let BoundaryError::Pause(pause) =
        restore_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap_err()
    else {
        panic!("expected sources pause")
    };
    assert_eq!(pause.outcome(), HbsOutcome::RolledBack);
    assert!(matches!(pause.need(), HbsNeed::Sources));
    sources.ready = true;
    sources.policy.validation_pause = true;
    let BoundaryError::Pause(pause) =
        restore_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap_err()
    else {
        panic!("expected pure validation pause")
    };
    assert!(matches!(pause.need(), HbsNeed::Structured { .. }));
    assert_eq!(snapshot(&db, &edited.id), before);
    assert_eq!(db.total_changes(), changes);
    assert!(db.is_autocommit());
    sources.policy.validation_pause = false;
    sources.policy.reject = true;
    let BoundaryError::Failure(rejected) =
        restore_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap_err()
    else {
        panic!("expected the original domain failure")
    };
    assert_eq!(rejected.failure.error.command_code(), "invalid_input");
    assert_eq!(
        rejected.failure.error.to_string(),
        "历史正文无法通过校验，当前内容已保留"
    );
    let FailureCause::NativeRejection(origin) = rejected.failure.cause else {
        panic!("matched Native decision origin must survive the original message mapping")
    };
    assert_eq!(origin.native_request_id, "synthetic-native-request");
    assert_eq!(rejected.outcome, HbsOutcome::RolledBack);
    assert_eq!(snapshot(&db, &edited.id), before);
    sources.policy.reject = false;
    db.execute_batch("create temp trigger reject_history_receipt before insert on structured_resource_receipts begin select raise(abort,'PRIVATE_DRIVER_MESSAGE'); end;").unwrap();
    let BoundaryError::Failure(failed) =
        restore_with(&mut sources, &mut db, input.clone(), ReviewRoute::Native).unwrap_err()
    else {
        panic!("expected a receipt SQL failure")
    };
    let Error::Database(cause) = &failed.failure.error else {
        panic!("original database error must remain typed")
    };
    assert!(cause.to_string().contains("PRIVATE_DRIVER_MESSAGE"));
    assert!(matches!(
        failed.failure.cause,
        FailureCause::SourceDatabase(_)
    ));
    assert_eq!(failed.phase, "receipt");
    assert_eq!(failed.outcome, HbsOutcome::RolledBack);
    assert_eq!(snapshot(&db, &edited.id), before);
    db.execute_batch("drop trigger reject_history_receipt")
        .unwrap();
    let restored = restore_with(&mut sources, &mut db, input, ReviewRoute::Native).unwrap();
    assert_eq!(restored.content, clean.content);
    assert_eq!(restored.review_baseline, clean.review_baseline);
    let repair_after =
        crate::structured_resources::read_text(&db, &edited.id, None, false).unwrap();
    assert_eq!(repair_after.text, repair.content);
    assert_eq!(
        repair_after.draft.unwrap().content_hash,
        repair.content_hash
    );
    let redone = restore_with(
        &mut sources,
        &mut db,
        history(&restored, edited.revision, "redo", "redo-human"),
        ReviewRoute::Native,
    )
    .unwrap();
    assert_eq!(redone.content, edited.content);
    assert_eq!(redone.review_baseline, edited.review_baseline);
    assert_eq!(
        crate::structured_resources::read_text(&db, &edited.id, None, false)
            .unwrap()
            .draft
            .unwrap()
            .content_hash,
        repair.content_hash
    );
    assert!(sources.policy.selected.is_empty());
}

#[test]
fn native_history_refuses_templates_stale_versions_and_untrusted_agent_history_before_sources() {
    let mut db = database();
    let target = reading(&db);
    let generated = save(
        &mut db,
        &target,
        body("原正文。").to_string(),
        "agent",
        "agent",
    )
    .resource;
    let mut sources = Sources::new();
    for (input, code) in [
        (
            history(&generated, generated.revision, "undo", "untrusted"),
            "invalid_input",
        ),
        (
            HistoryDraft {
                expected_revision: 0,
                ..history(&generated, generated.revision, "undo", "stale")
            },
            "revision_conflict",
        ),
    ] {
        let before = snapshot(&db, &generated.id);
        let BoundaryError::Failure(error) =
            restore_with(&mut sources, &mut db, input, ReviewRoute::Native).unwrap_err()
        else {
            panic!("existing authorization must precede sources")
        };
        assert_eq!(error.failure.error.command_code(), code);
        assert_eq!(snapshot(&db, &generated.id), before);
    }
    let template =
        crate::structured_resources::create_template(&db, "模板", "create-template", None).unwrap();
    let BoundaryError::Failure(error) = restore_with(
        &mut sources,
        &mut db,
        history(&template, 1, "undo", "template-history"),
        ReviewRoute::Native,
    )
    .unwrap_err() else {
        panic!("template history is outside Native scope")
    };
    assert_eq!(error.failure.error.command_code(), "invalid_input");
    assert_eq!(sources.requires, 0);
    assert_eq!(sources.policy.validated, 0);
}
