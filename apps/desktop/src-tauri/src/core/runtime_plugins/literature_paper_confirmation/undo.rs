//! Original whole-batch undo and marker-only reset retain their exact transaction.
use super::{
    ConfirmationPolicy, ConfirmationResources, mismatch, repository::ConfirmationRepository,
    unit_policy,
};
use crate::{
    ai::{confirmation_support::*, error::AiError, structured_resource_validation::Flow},
    runtime_plugins::literature_reading_units::confirmation::{
        self as unit_confirmation, AiReadingUnitUndoDraft,
    },
};
use research_workbench_native_sdk::paper_confirmation::{
    PaperConfirmationRequestV1 as Request,
    wire::{PaperConfirmationDecisionV1 as Decision, PaperConfirmationInputV1 as Input},
};
use rusqlite::Transaction;
use uuid::Uuid;

pub(super) fn undo<P: ConfirmationPolicy>(
    repository: &mut ConfirmationRepository<'_, '_, P>,
    draft: AiOverviewConfirmationUndoDraft,
) -> Result<AiOverviewConfirmationResult, Flow<AiError, P::Pause>> {
    repository.policy.decide(Input::Request {
        request: Request::UndoOverview {
            draft: draft.clone(),
        },
    })?;
    if let Some(field) = draft.field_kind.as_deref() {
        return repository.reset_field(&draft.batch_id, field, &draft.idempotency_key);
    }
    let reset_key_exists = repository
        .facts()
        .reset_key_exists(&draft.idempotency_key)?;
    repository
        .policy
        .decide(Input::UndoBatchKey { reset_key_exists })?;
    let batch = repository.facts().batch(&draft.batch_id)?;
    if !batch.active {
        return Ok(AiOverviewConfirmationResult {
            candidate: repository.facts().candidate(&batch.candidate_id)?,
            batch,
        });
    }
    let existing_batch_id = repository.facts().undo_for_key(&draft.idempotency_key)?;
    repository.policy.decide(Input::UndoReplayKey {
        existing_batch_id,
        requested_batch_id: draft.batch_id.clone(),
        paper: false,
    })?;
    repository.commit_undo(&batch, &draft.idempotency_key)
}

pub(super) fn apply<P: ConfirmationPolicy>(
    tx: &Transaction<'_>,
    resources: &ConfirmationResources,
    policy: &mut P,
    batch: &AiOverviewConfirmationBatch,
    key: &str,
) -> Result<(), Flow<AiError, P::Pause>> {
    let journal = ConfirmationJournal::new(tx);
    for field in &batch.field_confirmations {
        let previous_snapshot = journal
            .facts()
            .previous_snapshot(&field.id)?
            .map(|json| serde_json::from_str::<serde_json::Value>(&json))
            .transpose()
            .map_err(|_| AiError::InvalidInput("确认前快照已损坏".into()))?;
        unit_confirmation::undo(
            &resources.units,
            tx,
            AiReadingUnitUndoDraft {
                reading_unit_id: &field.target_reading_unit_id,
                expected_revision: field.target_applied_revision,
                previous_snapshot: previous_snapshot.as_ref(),
                confirmation_batch_id: &batch.id,
            },
            &mut |input| unit_policy(policy, &field.id, input),
        )?;
    }
    journal.append_undo(&Uuid::new_v4().to_string(), &batch.id, key)?;
    let remaining = journal.facts().active_confirmations(&batch.candidate_id)?;
    let state = if remaining == 0 {
        journal.facts().previous_state(&batch.id)?
    } else {
        "edited".into()
    };
    journal.set_review_state(&batch.candidate_id, &state)?;
    Ok(())
}

pub(super) fn apply_reset<P: ConfirmationPolicy>(
    tx: &Transaction<'_>,
    policy: &mut P,
    batch_id: &str,
    field_kind: &str,
    key: &str,
) -> Result<String, Flow<AiError, P::Pause>> {
    let journal = ConfirmationJournal::new(tx);
    let batch = journal.facts().batch(batch_id)?;
    let decision = policy.decide(Input::ResetField {
        batch: Box::new(batch.clone()),
        field_kind: field_kind.into(),
    })?;
    let Decision::ResetField { field } = decision else {
        return Err(mismatch());
    };
    let existing_reset_id = journal.facts().reset_for_key(key)?;
    let legacy_key_exists = journal.facts().undo_key_exists(key)?;
    let decision = policy.decide(Input::ResetEligibility {
        field: field.clone(),
        existing_reset_id,
        legacy_key_exists,
    })?;
    let Decision::ResetEligibility { append } = decision else {
        return Err(mismatch());
    };
    if append {
        journal.append_reset(&Uuid::new_v4().to_string(), &field.id, key)?;
        let candidate = journal.facts().candidate(&batch.candidate_id)?;
        let decision = policy.decide(Input::ResetState {
            field_active: candidate
                .overview_field_confirmations
                .iter()
                .map(|field| field.active)
                .collect(),
            revision_editor_kinds: candidate
                .revisions
                .iter()
                .map(|revision| revision.editor_kind.clone())
                .collect(),
        })?;
        let Decision::ResetState { state } = decision else {
            return Err(mismatch());
        };
        journal.set_review_state(&batch.candidate_id, &state)?;
    }
    Ok(batch.candidate_id)
}
