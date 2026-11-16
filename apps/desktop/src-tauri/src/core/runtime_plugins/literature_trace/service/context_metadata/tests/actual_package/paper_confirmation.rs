//! Actual Core transport and original four-command lifecycle acceptance.
mod failures;
mod lifecycle;
mod variants;
mod waits;
use super::*;
use research_workbench_native_sdk::{
    paper_confirmation::{PaperConfirmationRequestV1 as Request, contracts::*, wire::*},
    paper_context_wire::encode_payload,
};

async fn stage(f: &Fixture, input: PaperConfirmationInputV1) -> PaperConfirmationResultV1 {
    crate::runtime_plugins::native::package::accept_paper_confirmation(&f.service.catalog, &f.key)
        .unwrap();
    let mut slot = f.service.host.lock().await;
    let host = f.service.ensure_host(&mut slot, &f.key).await.unwrap();
    let (result, origin) = f
        .service
        .paper_confirmation_stage(host, &f.key, input)
        .await
        .unwrap();
    assert_eq!(origin.package_key, f.key);
    assert!(!origin.native_request_id.is_empty());
    result
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_paper_confirmation_normalizes_and_qualifies_original_candidate() {
    let mut f = Fixture::new();
    let value = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,
    "profileId":"reviewed_profile_composition_v1",
    "profileRefs":[{"profileId":"legacy","profileVersion":1}],"fields":[
        {"field":"approach","status":"filled","statements":[
            {"id":"s1","text":"Synthetic method Ω","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-quote"]}
        ]}
    ]});
    let candidate = generic_reading::seed(&f, value);
    let (catalog, key) = actual_package();
    f.bind_actual_package(catalog, key);
    let draft = AiPaperReviewConfirmationDraft {
        paper_id: "paper".into(),
        project_id: Some("original-context".into()),
        metadata_field_names: vec!["title".into(), "doi".into(), "title".into()],
        overview_targets: vec![],
        idempotency_key: " original-key ".into(),
    };
    let PaperConfirmationDecisionV1::Request {
        request: Request::ConfirmPaper { draft },
    } = stage(
        &f,
        PaperConfirmationInputV1::Request {
            request: Request::ConfirmPaper { draft },
        },
    )
    .await
    .unwrap()
    else {
        panic!("original request decision")
    };
    assert_eq!(draft.metadata_field_names, ["doi", "title"]);
    assert!(draft.project_id.is_none());
    assert_eq!(draft.idempotency_key, " original-key ");
    let field_draft = AiOverviewFieldConfirmationDraft {
        candidate_id: candidate.id.clone(),
        candidate_revision: candidate.current_revision,
        field_kinds: vec!["approach".into()],
        scope_kind: "field".into(),
        project_id: None,
        idempotency_key: "field-key".into(),
    };
    let expected = protocol::qualify(&candidate, &field_draft).unwrap();
    let PaperConfirmationDecisionV1::OverviewCandidate { qualified } = stage(
        &f,
        PaperConfirmationInputV1::OverviewCandidate {
            stored_review_state: candidate.review_state.clone(),
            candidate: Box::new(candidate.clone()),
            draft: field_draft,
        },
    )
    .await
    .unwrap() else {
        panic!("qualified original candidate")
    };
    assert_eq!(
        encode_payload(&qualified).unwrap(),
        encode_payload(&expected).unwrap()
    );
    let overview = qualified.overview.clone();
    let PaperConfirmationDecisionV1::OverviewFields { fields } = stage(
        &f,
        PaperConfirmationInputV1::OverviewFields {
            overview: Box::new(qualified.overview),
            requested_fields: vec!["approach".into()],
            confirmed_fields: vec![],
        },
    )
    .await
    .unwrap() else {
        panic!("original selected fields")
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].statements[0].text, "Synthetic method Ω");
    let PaperConfirmationDecisionV1::OverviewField { plan } = stage(
        &f,
        PaperConfirmationInputV1::OverviewField {
            candidate: Box::new(candidate),
            overview: Box::new(overview),
            field: Box::new(fields[0].clone()),
        },
    )
    .await
    .unwrap() else {
        panic!("original field interpretation")
    };
    assert_eq!(plan.anchors.len(), 1);
    let unit = plan.unit.unwrap();
    assert_eq!(unit.body, "Synthetic method Ω");
    assert_eq!(unit.provenance_kind, "ai_accepted_unchanged");
    assert_eq!(
        stage(
            &f,
            PaperConfirmationInputV1::UndoBatchKey {
                reset_key_exists: true
            }
        )
        .await
        .unwrap_err()
        .to_string(),
        "撤销幂等键已用于字段恢复"
    );
    let PaperConfirmationDecisionV1::Unit { decision } = stage(
        &f,
        PaperConfirmationInputV1::Unit {
            scope_key: "original-unit".into(),
            input: research_workbench_native_sdk::paper_confirmation::unit::UnitInputV1::Revision {
                previous_snapshot: None,
                current_revision: None,
                expected_revision: 1,
            },
        },
    )
    .await
    .unwrap() else {
        panic!("original unit revision")
    };
    assert_eq!(
        decision,
        research_workbench_native_sdk::paper_confirmation::unit::UnitDecisionV1::Revision {
            result: Err(
                research_workbench_native_sdk::paper_confirmation::unit::UnitProblemV1::NotFound {}
            ),
        }
    );
    assert_eq!(
        stage(
            &f,
            PaperConfirmationInputV1::OverviewReplay {
                requested_candidate_id: "original".into(),
                batch_candidate_id: "other".into(),
            }
        )
        .await
        .unwrap_err()
        .to_string(),
        "确认幂等键已用于其他候选"
    );
    let counts: (i64, i64) = f.store.connection().unwrap().query_row(
        "select (select count(*) from ai_overview_confirmation_batches), (select count(*) from ai_paper_review_confirmation_batches)",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(counts, (0, 0));
    f.service.shutdown().await.unwrap();
}

fn seed(f: &Fixture) -> research_workbench_native_sdk::candidate_review::contracts::AiCandidate {
    generic_reading::seed(
        f,
        json!({
            "kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"reviewed_profile_composition_v1",
            "profileRefs":[{"profileId":"legacy","profileVersion":1}],"fields":[
                {"field":"approach","status":"filled","statements":[{"id":"s1","text":"Original method Ω","epistemicRole":"source_paraphrase","evidenceItemIds":["ev-quote"],"evidenceQuotes":[{"evidenceItemId":"ev-quote","quote":"evidence Ω"}]}]},
                {"field":"conclusion","status":"filled","statements":[{"id":"s2","text":"Original conclusion Ω","epistemicRole":"inference","evidenceItemIds":["ev-quote"]}]}
            ]
        }),
    )
}
