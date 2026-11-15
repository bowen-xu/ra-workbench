//! Replay the original lazy SQL prefix against matching owned Native observations.
use super::*;
use crate::ai::generation_policy::{CandidateValidationFacts, PreparedCandidate};
use research_workbench_native_sdk::{
    candidate_contracts::{CandidatePayload, EvidenceQuote},
    paper_context_wire::decode_payload,
};
use rusqlite::Connection;

struct Observation {
    input: CandidateInputV1,
    decision: CandidateDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(super) struct CandidatePolicy {
    observations: Vec<Observation>,
}
pub(super) enum CaptureError {
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

impl CandidatePolicy {
    pub(super) fn record(
        &mut self,
        input: CandidateInputV1,
        decision: CandidateDecisionV1,
        origin: DecisionOrigin,
    ) {
        self.observations.push(Observation {
            input,
            decision,
            origin,
        });
    }
    pub(super) fn capture(
        &self,
        db: &Connection,
        run: &str,
        bundle: &AiEvidenceBundle,
        content: &str,
    ) -> Result<PreparedCandidate, CaptureError> {
        let (payload, payload_cbor_base64) = self.decode(bundle, content)?;
        let facts = CandidateValidationFacts::new(db);
        let (task_kind, suggestion_kind) = facts.run_output(run)?;
        self.validated(
            1,
            CandidateInputV1::KindAndQuotes {
                payload_cbor_base64: payload_cbor_base64.clone(),
                task_kind,
                suggestion_kind,
                evidence: bundle
                    .items
                    .iter()
                    .map(|item| EvidenceQuote {
                        citation_key: item.citation_key.clone(),
                        quote_exact: item.quote_exact.clone(),
                    })
                    .collect(),
            },
        )?;
        let profile = facts.overview_profile(run)?;
        self.validated(
            2,
            CandidateInputV1::Profile {
                payload_cbor_base64: payload_cbor_base64.clone(),
                profile,
            },
        )?;
        // Preserve the original matrix read even for an ordinary overview payload.
        let matrix = facts.matrix_context(run)?;
        let final_state = self.observed(
            3,
            CandidateInputV1::Finalize {
                payload_cbor_base64: payload_cbor_base64.clone(),
                matrix,
            },
        )?;
        let CandidateDecisionV1::Prepared { canonical_json } = &final_state.decision else {
            return Err(mismatch(&final_state.origin));
        };
        Ok(PreparedCandidate {
            kind: payload.kind(),
            canonical_json: canonical_json.clone(),
        })
    }
    pub(super) fn capture_revision(
        &self,
        db: &Connection,
        candidate: &research_workbench_native_sdk::candidate_review::contracts::AiCandidate,
        value: serde_json::Value,
    ) -> Result<PreparedCandidate, CaptureError> {
        let (payload, encoded) = self.decode(&candidate.evidence_bundle, &value.to_string())?;
        self.validated(
            1,
            CandidateInputV1::RevisionQuotesAndKind {
                payload_cbor_base64: encoded.clone(),
                candidate_kind: candidate.candidate_kind.clone(),
                evidence: candidate
                    .evidence_bundle
                    .items
                    .iter()
                    .map(|item| EvidenceQuote {
                        citation_key: item.citation_key.clone(),
                        quote_exact: item.quote_exact.clone(),
                    })
                    .collect(),
            },
        )?;
        let facts = CandidateValidationFacts::new(db);
        let profile = facts.overview_profile(&candidate.run_id)?;
        self.validated(
            2,
            CandidateInputV1::Profile {
                payload_cbor_base64: encoded.clone(),
                profile,
            },
        )?;
        let matrix = facts.matrix_context(&candidate.run_id)?;
        self.validated(
            3,
            CandidateInputV1::RevisionMatrix {
                payload_cbor_base64: encoded.clone(),
                matrix,
            },
        )?;
        let active_fields = if matches!(payload, CandidatePayload::PaperOverview(_)) {
            facts.active_confirmed_fields(&candidate.id)?
        } else {
            Vec::new()
        };
        let finalized = self.observed(
            4,
            CandidateInputV1::RevisionConfirmedFields {
                payload_cbor_base64: encoded,
                active_fields,
            },
        )?;
        let CandidateDecisionV1::Prepared { canonical_json } = &finalized.decision else {
            return Err(mismatch(&finalized.origin));
        };
        Ok(PreparedCandidate {
            kind: payload.kind(),
            canonical_json: canonical_json.clone(),
        })
    }
    fn decode(
        &self,
        bundle: &AiEvidenceBundle,
        content: &str,
    ) -> Result<(CandidatePayload, String), CaptureError> {
        let decoded = self.observed(
            0,
            CandidateInputV1::Decode {
                content: content.into(),
                evidence_keys: bundle
                    .items
                    .iter()
                    .map(|item| item.citation_key.clone())
                    .collect(),
            },
        )?;
        let CandidateDecisionV1::Decoded {
            payload_cbor_base64,
        } = &decoded.decision
        else {
            return Err(mismatch(&decoded.origin));
        };
        let payload: CandidatePayload =
            decode_payload(payload_cbor_base64).map_err(|_| mismatch(&decoded.origin))?;
        Ok((payload, payload_cbor_base64.clone()))
    }
    fn observed(
        &self,
        index: usize,
        input: CandidateInputV1,
    ) -> Result<&Observation, CaptureError> {
        let Some(observed) = self.observations.get(index) else {
            return Err(CaptureError::Need(Box::new(input)));
        };
        if observed.input != input {
            return Err(template::identified(
                "plugin_host_source_changed",
                "AI 候选来源已变化，请重新准备。",
                &observed.origin,
            )
            .into());
        }
        if let CandidateDecisionV1::Rejected { error } = &observed.decision {
            return Err(template::rejected(error.clone(), &observed.origin).into());
        }
        Ok(observed)
    }
    fn validated(&self, index: usize, input: CandidateInputV1) -> Result<(), CaptureError> {
        let observed = self.observed(index, input)?;
        if matches!(observed.decision, CandidateDecisionV1::Validated {}) {
            Ok(())
        } else {
            Err(mismatch(&observed.origin))
        }
    }
}
fn mismatch(origin: &DecisionOrigin) -> CaptureError {
    template::identified(
        "plugin_host_response_mismatch",
        "AI 候选决定结果与本次阶段不符。",
        origin,
    )
    .into()
}
