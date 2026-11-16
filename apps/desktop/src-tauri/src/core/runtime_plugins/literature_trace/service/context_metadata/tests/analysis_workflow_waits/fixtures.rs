//! Explicit accepted responses for the fixed controlled-wait requests.
use crate::ai::{
    candidate_contracts::{AiOverviewProfileContext, CandidatePayload},
    error::AiError,
    generation_policy::PreparedCandidate,
};
use research_workbench_native_sdk::{
    analysis_workflow::{
        candidate::{CandidateDecisionV1, CandidateInputV1},
        docling::DoclingEvidenceRow,
        overview_task::OverviewTaskInputV1,
    },
    candidate_review::wire::{GenericReviewSemanticsV1, ReviewDecisionV1, ReviewInputV1},
    generation::{AiTaskSpecDraft, evidence::*},
    paper_context::{PaperAiReadingContextItem, PreparedPaperContextV1},
    paper_context_wire::{decode_payload, encode_payload},
};
use serde_json::{Value, json};

fn responses() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/workflow-wait-responses-v1/responses.json"
    )))
    .unwrap()
}

pub(super) fn task(input: &OverviewTaskInputV1) -> AiTaskSpecDraft {
    let source = responses();
    let mut observed = serde_json::to_value(input).unwrap();
    let reference = input.template.as_ref().unwrap();
    observed["template"]["templateResourceId"] = json!("__template_id__");
    assert_eq!(observed, source["taskInput"]);
    let mut draft = source["task"].clone();
    draft["paperTemplateRef"] = serde_json::to_value(reference).unwrap();
    serde_json::from_value(draft).unwrap()
}

pub(super) fn context() -> PreparedPaperContextV1 {
    decode_payload(responses()["contextCbor"].as_str().unwrap()).unwrap()
}

pub(super) fn page_rows(
    attachment: &AttachmentEvidence,
    pages: &[(i64, String)],
) -> Vec<PreparedPageEvidence> {
    let fixture = responses();
    assert_eq!(
        serde_json::to_value(attachment).unwrap(),
        fixture["pageInput"]["attachment"]
    );
    assert_eq!(
        serde_json::to_value(pages).unwrap(),
        fixture["pageInput"]["pages"]
    );
    serde_json::from_value(fixture["pageRows"].clone()).unwrap()
}

pub(super) fn selection_rows(source: Option<AnchorEvidence>) -> Vec<AiEvidencePreview> {
    let fixture = responses();
    assert_eq!(
        serde_json::to_value(source.unwrap()).unwrap(),
        fixture["selectionInput"]
    );
    serde_json::from_value(fixture["selection"].clone()).unwrap()
}

pub(super) fn docling_row(
    run: &str,
    item: &PaperAiReadingContextItem,
) -> Result<Option<DoclingEvidenceRow>, AiError> {
    let fixtures = responses();
    let fixture = fixtures["doclingRows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fixture| fixture["item"]["id"] == item.id)
        .expect("known controlled Docling item");
    let expected: PaperAiReadingContextItem =
        decode_payload(fixture["itemCbor"].as_str().unwrap()).unwrap();
    assert_eq!(item, &expected);
    let mut row: DoclingEvidenceRow = serde_json::from_value(fixture["row"].clone()).unwrap();
    assert_eq!(row.anchor_id, format!("docling:__run_id__:{}", item.id));
    row.anchor_id = format!("docling:{run}:{}", item.id);
    Ok(Some(row))
}

pub(super) fn candidate(input: CandidateInputV1) -> Result<CandidateDecisionV1, AiError> {
    Ok(match input {
        CandidateInputV1::Decode {
            content,
            evidence_keys,
        } => {
            let payload: CandidatePayload = serde_json::from_str(&content).unwrap();
            assert_eq!(payload.kind(), "reference_record_candidate_v1");
            assert!(evidence_keys.contains(&"ev-quote".into()));
            CandidateDecisionV1::Decoded {
                payload_cbor_base64: encode_payload(&payload).unwrap(),
            }
        }
        CandidateInputV1::RevisionQuotesAndKind { candidate_kind, .. } => {
            assert_eq!(candidate_kind, "reference_record_candidate_v1");
            CandidateDecisionV1::Validated {}
        }
        CandidateInputV1::Profile { profile, .. } => {
            assert!(profile.is_none());
            CandidateDecisionV1::Validated {}
        }
        CandidateInputV1::RevisionMatrix { matrix, .. } => {
            assert!(matrix.is_none());
            CandidateDecisionV1::Validated {}
        }
        CandidateInputV1::RevisionConfirmedFields {
            payload_cbor_base64,
            active_fields,
        } => {
            assert!(active_fields.is_empty());
            let payload: CandidatePayload = decode_payload(&payload_cbor_base64).unwrap();
            CandidateDecisionV1::Prepared {
                canonical_json: serde_json::to_string(&payload).unwrap(),
            }
        }
        _ => panic!("unrecorded controlled candidate policy request"),
    })
}

pub(super) fn review(input: ReviewInputV1) -> ReviewDecisionV1 {
    match input {
        ReviewInputV1::ValidateDraft { draft } | ReviewInputV1::Replay { draft, .. } => {
            assert_eq!(draft.action, "defer");
            ReviewDecisionV1::Validated { rejection: None }
        }
        ReviewInputV1::Candidate { draft, .. } => {
            assert_eq!(draft.action, "defer");
            ReviewDecisionV1::Candidate {
                result: Ok(GenericReviewSemanticsV1 {
                    review_state: "deferred".into(),
                    provenance: None,
                    requires_inspection: false,
                    requires_citations: false,
                }),
            }
        }
        _ => panic!("unrecorded controlled review policy request"),
    }
}

pub(super) fn prepared_reference(content: &str) -> PreparedCandidate {
    let payload: CandidatePayload = serde_json::from_str(content).unwrap();
    assert_eq!(payload.kind(), "reference_record_candidate_v1");
    PreparedCandidate {
        kind: payload.kind(),
        canonical_json: serde_json::to_string(&payload).unwrap(),
    }
}

pub(super) fn preview(
    db: &rusqlite::Connection,
    credentials: &crate::ai::JsonCredentialStore,
    draft: &AiTaskSpecDraft,
) -> Result<crate::ai::generation_support::AiTaskPreview, AiError> {
    use crate::ai::{
        generation_support::{
            resolve_task_scope, scope_hash, validate_fixed_task_scope, validate_task_before_policy,
        },
        task_evidence_support::TaskEvidenceFacts,
    };
    validate_task_before_policy(draft)?;
    assert!(matches!(
        draft.task_kind.as_str(),
        "paper_overview" | "paper_suggestions" | "reference_extraction"
    ));
    let scope = resolve_task_scope(db, credentials, draft)?;
    if draft.task_kind == "paper_overview" {
        validate_fixed_task_scope(&draft.task_kind, &scope.items)?;
    } else {
        assert_eq!(scope.items.len(), 1);
        let (kind, id) = if draft.task_kind == "reference_extraction" {
            ("anchor", "reference-anchor")
        } else {
            ("attachment", "selected")
        };
        assert_eq!(scope.items[0].item_kind, kind);
        assert_eq!(scope.items[0].object_id, id);
    }
    let facts = TaskEvidenceFacts::new(db);
    let evidence = if draft.task_kind == "reference_extraction" {
        selection_rows(facts.anchor("reference-anchor")?)
    } else {
        page_rows(
            &facts.attachment("selected")?.unwrap(),
            &facts.page_texts("selected")?,
        );
        serde_json::from_value(responses()["pagePreview"].clone()).unwrap()
    };
    let profile = if draft.task_kind == "paper_overview" {
        let reference = draft.paper_template_ref.as_ref().unwrap();
        let (revision, binding) =
            crate::ai::template_facts::TemplateFacts::new(db).reading_template("paper")?;
        assert_eq!(binding, Some((reference.id().into(), reference.revision())));
        let mut fixed = responses()["profile"].clone();
        fixed["selectionRevision"] = json!(revision);
        fixed["paperTemplate"]["templateResourceId"] = json!(reference.id());
        Some(serde_json::from_value::<AiOverviewProfileContext>(fixed).unwrap())
    } else {
        None
    };
    let hash = scope_hash(draft, &scope.items, None, profile.as_ref(), None)?;
    Ok(scope.into_preview(draft, hash, profile, None, evidence))
}

pub(super) fn defer_candidate(
    db: &mut rusqlite::Connection,
    draft: research_workbench_native_sdk::candidate_review::contracts::AiReviewDecisionDraft,
) -> Result<research_workbench_native_sdk::candidate_review::contracts::AiCandidate, AiError> {
    use crate::ai::{
        review_support::{self, GenericReviewAdapter, ReviewPlan, ReviewPromotion},
        structured_resource_validation::Flow,
    };
    type Failure = Flow<AiError, std::convert::Infallible>;
    struct Deferred;
    impl GenericReviewAdapter for Deferred {
        type Pause = std::convert::Infallible;
        fn review(&mut self, input: ReviewInputV1) -> Result<ReviewDecisionV1, Failure> {
            Ok(review(input))
        }
        fn decode(
            &mut self,
            _: &research_workbench_native_sdk::candidate_review::contracts::AiCandidate,
            value: Value,
        ) -> Result<CandidatePayload, Failure> {
            Ok(serde_json::from_value(value).unwrap())
        }
        fn project_contains_paper(
            &mut self,
            _: &rusqlite::Connection,
            _: &str,
            _: &str,
        ) -> Result<bool, Failure> {
            panic!("deferred fixture has no project")
        }
        fn promote(
            &mut self,
            _: &rusqlite::Transaction<'_>,
            _: &ReviewPlan<'_>,
        ) -> ReviewPromotion<Self::Pause> {
            panic!("deferred fixture has no promotion")
        }
    }
    assert_eq!(draft.action, "defer");
    assert!(draft.project_id.is_none());
    review_support::decide_with(db, draft, true, &mut Deferred).map_err(Flow::into_static)
}

pub(crate) const EVIDENCE_WRITE_SQL: [&str; 7] = [
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-claim.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-revision.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-update.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-insert.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-history.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-link-upsert.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-write-link-lookup.v1.sql"
    )),
];

pub(crate) const EVIDENCE_READ_SQL: [&str; 4] = [
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-read-claims.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-read-source.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-read-sources.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-read-links.v1.sql"
    )),
];

pub(crate) const MATRIX_SQL: [&str; 14] = [
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-ids.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell-header.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell-keys.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-insert.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-header.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-update.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-revision.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell-insert.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell-update.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell-revision.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-matrix.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-cell.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-project-belongs.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/evidence-matrix-anchor-belongs.v1.sql"
    )),
];

pub(crate) const DERIVED_SQL: [&str; 7] = [
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-ids.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-insert.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-header.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-update.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-revision.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-view.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/derived-view-input-snapshot.v1.sql"
    )),
];

pub(crate) const MATRIX_CONTEXT_SQL: [&str; 2] = [
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/matrix-synthesis-matches.v1.sql"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/matrix-synthesis-cell.v1.sql"
    )),
];
