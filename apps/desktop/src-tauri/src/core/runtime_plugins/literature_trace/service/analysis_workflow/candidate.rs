//! Original generated-candidate validation, followed by Core-owned publication.
use super::*;
use crate::{
    ai::{
        candidate_contracts::AiEvidenceBundle,
        error::AiError,
        generation_support::{
            GeneratedCandidateCommit, GenerationStorageUnknown, save_prepared_candidate_guarded,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_workflow::candidate::*;
mod capture;
mod revision;
use capture::{CandidatePolicy, CaptureError};

impl LiteratureTraceService {
    pub(crate) async fn save_analysis_candidate(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        run: String,
        bundle: AiEvidenceBundle,
        content: String,
    ) -> Result<GeneratedCandidateCommit, AiError> {
        let owner = Arc::clone(self);
        let identity = run.clone();
        tokio::spawn(async move {
            owner
                .save_candidate_owned(store, key, run, bundle, content)
                .await
        })
        .await
        .map_err(|_| unsettled(&identity))?
    }
    async fn save_candidate_owned(
        &self,
        store: AppStore,
        key: String,
        run: String,
        bundle: AiEvidenceBundle,
        content: String,
    ) -> Result<GeneratedCandidateCommit, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let mut policy = CandidatePolicy::default();
        let source_bundle = Arc::new(bundle);
        let source_content = Arc::new(content);
        // Decode, kind/quotes, profile, matrix/canonical projection, then publication.
        for _ in 0..5 {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let bundle = Arc::clone(&source_bundle);
            let content = Arc::clone(&source_content);
            let id = run.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut db = source.connection().map_err(AiError::from)?;
                    let prepared = policy.capture(&db, &id, &bundle, &content)?;
                    save_prepared_candidate_guarded(&mut db, &id, &bundle, prepared)
                        .map_err(CaptureError::from)
                })();
                (policy, result)
            })
            .await
            .map_err(|_| unsettled(&run))?;
            policy = next;
            match result {
                Ok(saved) => return Ok(saved),
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
                            "AI 候选决定结果种类不匹配。",
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
                "AI 候选准备未在原有限步骤内完成。",
            )
            .package(&key),
        ))
    }
}
fn unsettled(run: &str) -> AiError {
    AiError::GenerationStorageUnknown(Box::new(GenerationStorageUnknown::unsettled(
        run,
        "candidate",
        "execution",
        "sql_worker_unsettled",
    )))
}
