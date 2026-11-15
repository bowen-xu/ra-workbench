//! Original partial module commits retain their in-flight loop position across Native waits.
use super::{
    ConfirmationPolicy, mismatch, overview,
    repository::{ConfirmationRepository, PendingMetadataUndo},
    undo,
};
use crate::{
    ai::{confirmation_support::*, error::AiError, structured_resource_validation::Flow},
    runtime_plugins::native::{NativeHostError, NativeProfile},
};
use research_workbench_native_sdk::paper_confirmation::{
    PaperConfirmationRequestV1 as Request,
    wire::{PaperConfirmationDecisionV1 as Decision, PaperConfirmationInputV1 as Input},
};
use uuid::Uuid;

pub(super) struct ConfirmProgress {
    draft: AiPaperReviewConfirmationDraft,
    request_json: String,
    batch_id: String,
    metadata_index: usize,
    overview_index: usize,
    failures: Vec<String>,
}
pub(super) struct UndoProgress {
    draft: AiPaperReviewConfirmationUndoDraft,
    batch: AiPaperReviewConfirmationBatch,
    overview_index: usize,
    pending_metadata: Option<Vec<PendingMetadataUndo>>,
    metadata_index: usize,
    failures: Vec<String>,
}

pub(super) fn confirm<P: ConfirmationPolicy>(
    repository: &mut ConfirmationRepository<'_, '_, P>,
    draft: AiPaperReviewConfirmationDraft,
    progress: &mut Option<ConfirmProgress>,
) -> Result<AiPaperReviewConfirmationResult, Flow<AiError, P::Pause>> {
    let resumed = progress.is_some();
    if progress.is_none() {
        let decision = repository.policy.decide(Input::Request {
            request: Request::ConfirmPaper { draft },
        })?;
        let Decision::Request {
            request: Request::ConfirmPaper { draft },
        } = decision
        else {
            return Err(mismatch());
        };
        let request_json = serde_json::to_string(&draft)
            .map_err(|_| AiError::InvalidInput("无法保存文献确认范围".into()))?;
        let batch_id = match repository.paper_batch_for_key(&draft.idempotency_key)? {
            Some((id, stored_paper_id, stored_request_json)) => {
                repository.policy.decide(Input::PaperReplay {
                    paper_id: draft.paper_id.clone(),
                    request_json: request_json.clone(),
                    stored_paper_id,
                    stored_request_json,
                })?;
                id
            }
            None => {
                let exists = repository.paper_exists(&draft.paper_id)?;
                repository.policy.decide(Input::PaperExists { exists })?;
                let id = Uuid::new_v4().to_string();
                repository.insert_paper_batch(&id, &draft, &request_json)?;
                id
            }
        };
        *progress = Some(ConfirmProgress {
            draft,
            request_json,
            batch_id,
            metadata_index: 0,
            overview_index: 0,
            failures: vec![],
        });
    }
    let progress = progress
        .as_mut()
        .expect("initialized original paper confirmation loop");
    if resumed {
        let binding = repository.paper_batch_for_key(&progress.draft.idempotency_key)?;
        if binding.as_ref()
            != Some(&(
                progress.batch_id.clone(),
                progress.draft.paper_id.clone(),
                progress.request_json.clone(),
            ))
        {
            return Err(changed());
        }
    }
    while let Some(field) = progress
        .draft
        .metadata_field_names
        .get(progress.metadata_index)
    {
        let result =
            repository.confirm_metadata_field(&progress.batch_id, &progress.draft.paper_id, field);
        record(
            result,
            &mut progress.failures,
            &format!("基础信息 {field}"),
            repository.policy.explicit_settlement(),
        )?;
        progress.metadata_index += 1;
        repository.policy.complete_module();
    }
    while let Some(target) = progress.draft.overview_targets.get(progress.overview_index) {
        let result = confirm_target(repository, &progress.batch_id, &progress.draft, target);
        record(
            result,
            &mut progress.failures,
            &format!("结构化阅读候选 {}", target.candidate_id),
            repository.policy.explicit_settlement(),
        )?;
        progress.overview_index += 1;
        repository.policy.complete_module();
    }
    repository
        .paper_result(&progress.batch_id, progress.failures.clone())
        .map_err(Flow::original)
}

pub(super) fn undo<P: ConfirmationPolicy>(
    repository: &mut ConfirmationRepository<'_, '_, P>,
    draft: AiPaperReviewConfirmationUndoDraft,
    progress: &mut Option<UndoProgress>,
) -> Result<AiPaperReviewConfirmationResult, Flow<AiError, P::Pause>> {
    if progress.is_none() {
        repository.policy.decide(Input::Request {
            request: Request::UndoPaper {
                draft: draft.clone(),
            },
        })?;
        let batch = repository.paper_batch(&draft.batch_id)?;
        if !batch.active {
            return repository
                .paper_result(&batch.id, vec![])
                .map_err(Flow::original);
        }
        let existing_batch_id = repository.paper_undo_for_key(&draft.idempotency_key)?;
        repository.policy.decide(Input::UndoReplayKey {
            existing_batch_id,
            requested_batch_id: draft.batch_id.clone(),
            paper: true,
        })?;
        *progress = Some(UndoProgress {
            draft,
            batch,
            overview_index: 0,
            pending_metadata: None,
            metadata_index: 0,
            failures: vec![],
        });
    }
    let progress = progress
        .as_mut()
        .expect("initialized original paper undo loop");
    while let Some(batch_id) = progress
        .batch
        .overview_batch_ids
        .get(progress.overview_index)
    {
        let active = repository.facts().batch_is_active(batch_id)?;
        if active {
            let result = undo::undo(
                repository,
                AiOverviewConfirmationUndoDraft {
                    field_kind: None,
                    batch_id: batch_id.clone(),
                    idempotency_key: format!("paper-review-undo:{}:{batch_id}", progress.batch.id),
                },
            )
            .map(|_| ());
            record(
                result,
                &mut progress.failures,
                "结构化阅读",
                repository.policy.explicit_settlement(),
            )?;
        }
        progress.overview_index += 1;
        repository.policy.complete_module();
    }
    if progress.pending_metadata.is_none() {
        progress.pending_metadata =
            Some(repository.load_pending_metadata_undo(&progress.batch.id)?);
    }
    let pending = progress
        .pending_metadata
        .as_ref()
        .expect("captured original pending metadata");
    while let Some(confirmation) = pending.get(progress.metadata_index) {
        let result = repository.undo_metadata_field(&progress.batch, confirmation);
        record(
            result,
            &mut progress.failures,
            &format!("基础信息 {}", confirmation.field_name),
            repository.policy.explicit_settlement(),
        )?;
        progress.metadata_index += 1;
        repository.policy.complete_module();
    }
    let remaining_metadata = repository.remaining_metadata(&progress.batch.id)?;
    let remaining_overviews = repository.remaining_overviews(&progress.batch.id)?;
    if remaining_metadata == 0 && remaining_overviews == 0 {
        repository.append_paper_undo(
            &Uuid::new_v4().to_string(),
            &progress.batch.id,
            &progress.draft.idempotency_key,
        )?;
    }
    repository
        .paper_result(&progress.batch.id, progress.failures.clone())
        .map_err(Flow::original)
}

fn confirm_target<P: ConfirmationPolicy>(
    repository: &mut ConfirmationRepository<'_, '_, P>,
    batch: &str,
    draft: &AiPaperReviewConfirmationDraft,
    target: &AiPaperReviewOverviewTargetDraft,
) -> Result<(), Flow<AiError, P::Pause>> {
    let candidate = repository.facts().candidate(&target.candidate_id)?;
    repository.facts().require_completed_turn(&candidate.id)?;
    repository.policy.decide(Input::PaperTarget {
        candidate: Box::new(candidate),
        paper_id: draft.paper_id.clone(),
    })?;
    let result = overview::confirm(
        repository,
        AiOverviewFieldConfirmationDraft {
            candidate_id: target.candidate_id.clone(),
            candidate_revision: target.candidate_revision,
            field_kinds: target.field_kinds.clone(),
            scope_kind: "paper".into(),
            project_id: draft.project_id.clone(),
            idempotency_key: format!("paper-review:{batch}:{}", target.candidate_id),
        },
    )?;
    repository.link_overview(batch, &result.batch.id)?;
    Ok(())
}

fn record<P>(
    result: Result<(), Flow<AiError, P>>,
    failures: &mut Vec<String>,
    label: &str,
    explicit: bool,
) -> Result<(), Flow<AiError, P>> {
    match result {
        Ok(()) => Ok(()),
        Err(Flow::Fail(failure))
            if !explicit
                || matches!(
                    &failure.error,
                    AiError::NativeWorkflow {
                        code: "invalid_input" | "storage_error",
                        ..
                    }
                )
                || !matches!(
                    &failure.error,
                    AiError::NativeWorkflow { .. } | AiError::GenerationStorageUnknown(_)
                ) =>
        {
            failures.push(format!("{label}：{}", failure.error));
            Ok(())
        }
        Err(error) => Err(error),
    }
}
fn changed<P>() -> Flow<AiError, P> {
    Flow::original(AiError::NativeWorkflow {
        code: "plugin_host_source_changed",
        failure: NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_source_changed",
            "文献确认批次来源已变化，请重新载入。",
        ),
    })
}
