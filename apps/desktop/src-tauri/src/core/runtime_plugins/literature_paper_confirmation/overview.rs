//! Original overview reads and atomic adoption, with finite policy suspension.
use super::{
    ConfirmationPolicy, ConfirmationResources, mismatch, repository::ConfirmationRepository,
    unit_policy,
};
use crate::ai::{
    candidate_contracts::{PaperOverviewField, PaperOverviewPayload},
    confirmation_support::*,
    error::AiError,
    structured_resource_validation::Flow,
};
use crate::runtime_plugins::literature_reading_units::confirmation::{
    self as unit_confirmation, AiConfirmedReadingUnitDraft,
};
use research_workbench_native_sdk::paper_confirmation::{
    PaperConfirmationRequestV1 as Request,
    wire::{PaperConfirmationDecisionV1 as Decision, PaperConfirmationInputV1 as Input},
};
use uuid::Uuid;

pub(super) struct Plan<'a> {
    pub candidate: &'a AiCandidate,
    pub overview: &'a PaperOverviewPayload,
    pub fields: Vec<PaperOverviewField>,
    pub draft: &'a AiOverviewFieldConfirmationDraft,
    pub paper_id: String,
    pub stored_review_state: String,
    pub batch_id: String,
    pub populated_field_count: usize,
}
pub(super) fn confirm<P: ConfirmationPolicy>(
    repository: &mut ConfirmationRepository<'_, '_, P>,
    draft: AiOverviewFieldConfirmationDraft,
) -> Result<AiOverviewConfirmationResult, Flow<AiError, P::Pause>> {
    let decision = repository.policy.decide(Input::Request {
        request: Request::ConfirmOverview {
            draft: draft.clone(),
        },
    })?;
    let Decision::Request {
        request: Request::ConfirmOverview { .. },
    } = decision
    else {
        return Err(mismatch());
    };
    if let Some(batch_id) = repository.facts().batch_for_key(&draft.idempotency_key)? {
        let batch = repository.facts().batch(&batch_id)?;
        repository.policy.decide(Input::OverviewReplay {
            requested_candidate_id: draft.candidate_id.clone(),
            batch_candidate_id: batch.candidate_id.clone(),
        })?;
        return Ok(AiOverviewConfirmationResult {
            candidate: repository.facts().candidate(&draft.candidate_id)?,
            batch,
        });
    }

    let candidate = repository.facts().candidate(&draft.candidate_id)?;
    repository.facts().require_completed_turn(&candidate.id)?;
    let stored_review_state = repository
        .facts()
        .stored_review_state(&draft.candidate_id)?;
    let decision = repository.policy.decide(Input::OverviewCandidate {
        stored_review_state: stored_review_state.clone(),
        candidate: Box::new(candidate.clone()),
        draft: draft.clone(),
    })?;
    let Decision::OverviewCandidate { qualified } = decision else {
        return Err(mismatch());
    };
    let paper_id = qualified.paper_id;
    let overview = &qualified.overview;
    let active_fields = repository.facts().fields(&candidate.id)?;
    let decision = repository.policy.decide(Input::OverviewFields {
        overview: Box::new(overview.clone()),
        requested_fields: draft.field_kinds.clone(),
        confirmed_fields: active_fields,
    })?;
    let Decision::OverviewFields { fields } = decision else {
        return Err(mismatch());
    };
    // Optional human review marks adoption of reading content, not citation verification.
    // Evidence and semantic assessments remain independently recorded.

    repository.commit_fields(Plan {
        candidate: &candidate,
        overview,
        fields,
        draft: &draft,
        paper_id,
        stored_review_state,
        batch_id: Uuid::new_v4().to_string(),
        populated_field_count: qualified.populated_field_count,
    })
}
pub(super) fn apply<P: ConfirmationPolicy>(
    tx: &rusqlite::Transaction<'_>,
    resources: &ConfirmationResources,
    policy: &mut P,
    plan: &Plan<'_>,
) -> Result<(), Flow<AiError, P::Pause>> {
    let journal = ConfirmationJournal::new(tx);
    journal.append_batch(BatchInsert {
        id: &plan.batch_id,
        candidate: &plan.candidate.id,
        revision: plan.candidate.current_revision,
        scope: &plan.draft.scope_kind,
        paper: &plan.paper_id,
        project: None,
        previous_state: &plan.stored_review_state,
        key: &plan.draft.idempotency_key,
    })?;
    for field in &plan.fields {
        let decision = policy.decide(Input::OverviewField {
            candidate: Box::new(plan.candidate.clone()),
            overview: Box::new(plan.overview.clone()),
            field: Box::new(field.clone()),
        })?;
        let Decision::OverviewField { plan: prepared } = decision else {
            return Err(mismatch());
        };
        let anchor_ids = resources.anchors.apply(tx, prepared.anchors)?;
        let unit = prepared.unit.map_err(AiError::from)?;
        let confirmation = unit_confirmation::confirm(
            &resources.units,
            tx,
            AiConfirmedReadingUnitDraft {
                template_field: unit
                    .template_field
                    .as_ref()
                    .map(|(portrait, field)| (portrait.as_str(), field.as_str())),
                project_id: None,
                paper_id: &plan.paper_id,
                unit_kind: &unit.unit_kind,
                title: &unit.title,
                body: &unit.body,
                provenance_kind: &unit.provenance_kind,
                epistemic_role: &unit.epistemic_role,
                anchor_ids: &anchor_ids,
                candidate_id: &plan.candidate.id,
                candidate_revision: plan.candidate.current_revision,
            },
            &mut |input| unit_policy(policy, &field.field, input),
        )?;
        journal.append_field(FieldInsert {
            id: &Uuid::new_v4().to_string(),
            batch: &plan.batch_id,
            candidate: &plan.candidate.id,
            revision: plan.candidate.current_revision,
            field: &field.field,
            hash: &prepared.hash.map_err(AiError::from)?,
            decision: &unit.decision_kind,
            target: &confirmation.reading_unit_id,
            applied_revision: confirmation.applied_revision,
            previous_snapshot: confirmation
                .previous_snapshot
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| AiError::InvalidInput("无法保存确认前快照".to_owned()))?,
        })?;
    }
    let confirmed_count = journal.facts().distinct_active_fields(&plan.candidate.id)?;
    if usize::try_from(confirmed_count).ok() == Some(plan.populated_field_count) {
        let has_human = journal.facts().has_human_edit(&plan.candidate.id)?;
        journal.set_review_state(
            &plan.candidate.id,
            if has_human {
                "accepted_after_edit"
            } else {
                "accepted_unchanged"
            },
        )?;
    }
    Ok(())
}
