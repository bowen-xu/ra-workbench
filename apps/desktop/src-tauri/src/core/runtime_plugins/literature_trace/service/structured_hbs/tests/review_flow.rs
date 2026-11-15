use super::*;
use crate::structured_resources::{
    self as resources, HistoryDraft, Resource, ReviewDraft, WriteDraft,
};
use research_workbench_native_sdk::review_selection::{
    UnreviewedSelectionInputV1, UnreviewedSelectionV1,
};

fn candidate(fixture: &Fixture) -> Resource {
    let mut db = fixture.store.connection().unwrap();
    let target = resources::get(&db, &fixture.id).unwrap();
    let result = crate::hbs_storage::test_sources::write(
        &mut db,
        WriteDraft {
            resource_id: target.id,
            expected_revision: target.revision,
            content: portrait(9).to_string(),
            request_id: "service-candidate:1".into(),
            conversation_id: None,
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: None,
            editor_kind: "agent".into(),
        },
    )
    .unwrap();
    assert_eq!(
        result.status, "applied",
        "original static candidate fixture must be valid"
    );
    result.resource
}
fn selection(resource: &Resource) -> UnreviewedSelectionInputV1 {
    UnreviewedSelectionInputV1 {
        resource_id: resource.id.clone(),
        resource_revision: resource.revision,
        pending_field_ids: resource.pending_field_ids.clone(),
        protected_field_ids: vec![],
        reviewed_field_ids: vec![],
    }
}
fn keep(resource: &Resource) -> ReviewDraft {
    ReviewDraft {
        resource_id: resource.id.clone(),
        expected_revision: resource.revision,
        request_id: "service-keep:1".into(),
        action: "keep".into(),
        field_ids: None,
        scope: Some("unreviewed".into()),
    }
}
fn validation(resource: &Resource) -> CatalogOutput {
    let value = resource.content.as_ref().unwrap();
    CatalogOutput::StructuredValidation {
        outcome: Box::new(StructuredValidationOutcomeV1::Validated {
            input: Box::new(input(value)),
            canonical_json: value.to_string(),
        }),
    }
}

fn prepared(validation_calls: usize) -> (Fixture, Resource) {
    let mut fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    let original = candidate(&fixture);
    let selected = selection(&original);
    let mut responses = vec![CatalogOutput::UnreviewedFields {
        selection: Box::new(UnreviewedSelectionV1 {
            selected_field_ids: selected.pending_field_ids.clone(),
            input: Box::new(selected),
        }),
    }];
    responses.extend((0..validation_calls).map(|_| validation(&original)));
    fixture.responses(&responses);
    (fixture, original)
}

#[tokio::test]
async fn selection_needs_no_unused_reading_sql_and_receipt_replay_does_not_call_native() {
    let (fixture, original) = prepared(0);
    let directory = fixture._package.path().join("writing/data");
    for name in ["snapshot-hash", "snapshot-rows", "validation-latest"] {
        let file = directory.join(format!("reading-evidence-{name}.v1.sql"));
        std::fs::remove_file(&file).unwrap();
    }
    let request = keep(&original);
    let kept = fixture
        .service
        .review_resource(fixture.store.clone(), request.clone())
        .await
        .unwrap();
    assert_eq!(kept.revision, original.revision + 1);
    assert!(kept.pending_field_ids.is_empty());
    assert_eq!(kept.content, kept.review_baseline);
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        1
    );
    let replay = fixture
        .service
        .review_resource(fixture.store.clone(), request)
        .await
        .unwrap();
    assert_eq!(replay.revision, kept.revision);
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        1
    );
    fixture.service.shutdown().await.unwrap();
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
}

#[tokio::test]
async fn trusted_keep_undo_redo_keeps_one_immutable_package_and_the_original_owner() {
    let (fixture, original) = prepared(2);
    let kept = fixture
        .service
        .review_resource(fixture.store.clone(), keep(&original))
        .await
        .unwrap();
    let restored = fixture
        .service
        .restore_resource_history(
            fixture.store.clone(),
            HistoryDraft {
                resource_id: kept.id.clone(),
                expected_revision: kept.revision,
                operation_revision: kept.revision,
                direction: "undo".into(),
                request_id: "service-history:1".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.content, original.content);
    assert_eq!(restored.review_baseline, original.review_baseline);
    let redone = fixture
        .service
        .restore_resource_history(
            fixture.store.clone(),
            HistoryDraft {
                resource_id: kept.id.clone(),
                expected_revision: restored.revision,
                operation_revision: kept.revision,
                direction: "redo".into(),
                request_id: "service-history:2".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(redone.content, kept.content);
    assert_eq!(redone.review_baseline, kept.review_baseline);
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        3
    );
    fixture.service.shutdown().await.unwrap();
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
}

#[tokio::test]
async fn out_of_scope_selection_is_a_matched_technical_failure_before_any_review_write() {
    let mut fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    let original = candidate(&fixture);
    fixture.responses(&[CatalogOutput::UnreviewedFields {
        selection: Box::new(UnreviewedSelectionV1 {
            input: Box::new(selection(&original)),
            selected_field_ids: vec!["outside-pending".into()],
        }),
    }]);
    let request = keep(&original);
    let error = fixture
        .service
        .review_resource(fixture.store.clone(), request.clone())
        .await
        .unwrap_err();
    assert_eq!(
        error.request_id.as_deref(),
        Some(request.request_id.as_str())
    );
    let diagnostic = error.native_failure.unwrap();
    assert_eq!(diagnostic.kind, "technical");
    assert_eq!(diagnostic.outcome, HbsOutcome::RolledBack);
    assert_eq!(diagnostic.code, "plugin_host_response_mismatch");
    assert!(diagnostic.instance_id.is_some());
    assert!(diagnostic.native_request_id.is_some());
    let db = fixture.store.connection().unwrap();
    let after = resources::get(&db, &original.id).unwrap();
    assert_eq!(after.revision, original.revision);
    assert_eq!(after.content, original.content);
    assert_eq!(after.review_baseline, original.review_baseline);
    let writes: i64 = db
        .query_row(
            "select count(*) from structured_resource_receipts where request_id=?1",
            [request.request_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(writes, 0);
    drop(db);
    fixture.service.shutdown().await.unwrap();
}
