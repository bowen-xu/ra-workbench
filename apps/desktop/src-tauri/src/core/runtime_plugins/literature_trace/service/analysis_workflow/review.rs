//! Original Core citation reads and write pause only at the existing payload decoder.
use super::*;
mod acceptance;
use crate::{
    ai::{
        candidate_contracts::CandidatePayload,
        error::AiError,
        review_support::{current_payload_source, record_citation_assessment_with},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::candidate::{CandidateDecisionV1, CandidateInputV1},
    candidate_review::contracts::{AiCandidate, AiCitationAssessmentDraft},
};

#[derive(Default)]
struct CitationDecoder {
    observed: Option<(CandidateInputV1, CandidateDecisionV1, DecisionOrigin)>,
}
enum CaptureError {
    Original(Box<AiError>),
    Need(Box<CandidateInputV1>),
}
impl From<AiError> for CaptureError {
    fn from(error: AiError) -> Self {
        Self::Original(Box::new(error))
    }
}
impl From<rusqlite::Error> for CaptureError {
    fn from(error: rusqlite::Error) -> Self {
        AiError::from(error).into()
    }
}
impl CitationDecoder {
    fn decode(&self, candidate: &AiCandidate) -> Result<CandidatePayload, CaptureError> {
        let (value, keys) = current_payload_source(candidate)?;
        let input = CandidateInputV1::Decode {
            content: value.to_string(),
            evidence_keys: keys,
        };
        let Some((previous, decision, origin)) = &self.observed else {
            return Err(CaptureError::Need(Box::new(input)));
        };
        if previous != &input {
            return Err(template::identified(
                "plugin_host_source_changed",
                "引用判断的候选来源已变化，请重新载入。",
                origin,
            )
            .into());
        }
        match decision {
            CandidateDecisionV1::Decoded {
                payload_cbor_base64,
            } => decode_payload(payload_cbor_base64).map_err(|_| {
                template::identified(
                    "plugin_host_response_mismatch",
                    "引用判断候选解码结果格式不匹配。",
                    origin,
                )
                .into()
            }),
            CandidateDecisionV1::Rejected { error } => {
                Err(template::rejected(error.clone(), origin).into())
            }
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "引用判断候选结果种类不匹配。",
                origin,
            )
            .into()),
        }
    }
}
impl LiteratureTraceService {
    pub(crate) async fn record_candidate_citation_assessment(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: AiCitationAssessmentDraft,
    ) -> Result<AiCandidate, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let candidate = draft.candidate_id.clone();
        tokio::spawn(async move { owner.record_citation_owned(store, key, draft).await })
            .await
            .map_err(|_| unsettled(&selected, &candidate))?
    }
    async fn record_citation_owned(
        &self,
        store: AppStore,
        key: String,
        draft: AiCitationAssessmentDraft,
    ) -> Result<AiCandidate, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let mut decoder = CitationDecoder::default();
        // The original entry consumes one current payload, then returns its original read-back.
        for _ in 0..2 {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let request = draft.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let db = source.connection().map_err(AiError::from)?;
                    record_citation_assessment_with(&db, request, |candidate| {
                        decoder.decode(candidate)
                    })
                })();
                (decoder, result)
            })
            .await
            .map_err(|_| unsettled(&key, &draft.candidate_id))?;
            decoder = next;
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
                            "引用判断候选结果种类不匹配。",
                            &origin,
                        ));
                    };
                    decoder.observed = Some((*input, decision, origin));
                }
            }
        }
        Err(unsettled(&key, &draft.candidate_id))
    }
}
fn unsettled(key: &str, candidate: &str) -> AiError {
    template::technical(
        LiteratureTraceService::error(
            "plugin_host_source_worker_unsettled",
            format!("引用判断操作结果尚未确认，请核对原候选 {candidate} 与幂等请求。"),
        )
        .package(key),
    )
}
