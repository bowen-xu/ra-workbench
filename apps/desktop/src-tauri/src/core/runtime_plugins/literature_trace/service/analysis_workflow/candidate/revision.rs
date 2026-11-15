//! Original revision checks and transaction consume the same selected Native policy.
use super::*;
use crate::{ai::review_support::revise_candidate_with, data_interface::guarded_transaction};
use research_workbench_native_sdk::candidate_review::contracts::{
    AiCandidate, AiCandidateRevisionDraft,
};

impl LiteratureTraceService {
    pub(crate) async fn revise_analysis_candidate(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: AiCandidateRevisionDraft,
    ) -> Result<AiCandidate, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let candidate = draft.candidate_id.clone();
        tokio::spawn(async move { owner.revise_owned(store, key, draft).await })
            .await
            .map_err(|_| unsettled(&selected, &candidate))?
    }
    async fn revise_owned(
        &self,
        store: AppStore,
        key: String,
        draft: AiCandidateRevisionDraft,
    ) -> Result<AiCandidate, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let mut policy = CandidatePolicy::default();
        // Decode, quotes/kind, profile, matrix, confirmed fields/canonical bytes, then write.
        for _ in 0..6 {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let request = draft.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut db = source.connection().map_err(AiError::from)?;
                    guarded_transaction::run_mut(&mut db, |db| {
                        let result = revise_candidate_with(db, request, true, |db, candidate, value| {
                            policy.capture_revision(db, candidate, value)
                        });
                        let unknown = !db.is_autocommit()
                            || matches!(
                                &result,
                                Err(CaptureError::Original(error))
                                    if matches!(error.as_ref(), AiError::GenerationStorageUnknown(_))
                            );
                        (result, unknown)
                    })
                })();
                (policy, result)
            })
            .await
            .map_err(|_| unsettled(&key, &draft.candidate_id))?;
            policy = next;
            match result {
                Ok(candidate) => return Ok(candidate),
                Err(CaptureError::Original(error)) => return Err(*error),
                Err(CaptureError::Need(input)) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(
                            &key,
                            WorkflowInputV1::Candidate {
                                input: input.clone(),
                            },
                        )
                        .await
                        .map_err(template::technical)?;
                    let WorkflowDecisionV1::Candidate { decision } = decision else {
                        return Err(template::identified(
                            "plugin_host_response_mismatch",
                            "候选修订结果种类不匹配。",
                            &origin,
                        ));
                    };
                    policy.record(*input, decision, origin);
                }
            }
        }
        Err(template::technical(
            Self::error(
                "plugin_host_response_mismatch",
                "候选修订未在原有限步骤内完成。",
            )
            .package(&key),
        ))
    }
}
fn unsettled(key: &str, candidate: &str) -> AiError {
    template::technical(
        LiteratureTraceService::error(
            "plugin_host_source_worker_unsettled",
            format!("候选修订结果尚未确认，请核对原候选 {candidate}。"),
        )
        .package(key),
    )
}
