//! Operation-local exact-input prefix, reusing the original owned payload codec.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin, runtime_plugins::native::DecidedProject,
};
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
use research_workbench_native_sdk::streaming_payload::{self, PayloadDigest};
enum Response {
    Writing(Arc<Decision>),
    Library(Arc<CatalogOutput>),
}
struct Cached {
    digest: PayloadDigest,
    decision: Response,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(super) struct Reader {
    cursor: usize,
    cache: Vec<Cached>,
    last: Option<DecisionOrigin>,
}
impl Reader {
    pub(super) fn begin(&mut self) {
        self.cursor = 0;
        self.last = None;
    }
    pub(super) fn attach(&self, error: &mut Box<NativeHostError>) {
        if let Some(origin) = &self.last
            && error.package_key.as_deref() == Some(origin.package_key.as_str())
        {
            error.instance_id = Some(origin.instance_id.clone());
            error.request_id = Some(origin.native_request_id.clone());
        }
    }
    pub(super) fn library_digest(input: &CatalogInput) -> Read<PayloadDigest> {
        streaming_payload::write_json(std::io::sink(), input)
            .map(|(_, digest)| digest)
            .map_err(|cause| original(WritingError::Io(std::io::Error::other(cause))))
    }
    pub(super) fn resolve_library(
        &mut self,
        digest: PayloadDigest,
        decided: crate::runtime_plugins::literature_trace::service::DecidedBibliography,
        key: &str,
    ) {
        self.cache.push(Cached {
            digest,
            decision: Response::Library(Arc::new(decided.output)),
            origin: DecisionOrigin {
                package_key: key.into(),
                instance_id: decided.instance_id,
                native_request_id: decided.request_id,
            },
        });
    }
    pub(super) fn library(&mut self, input: CatalogInput, key: &str) -> Read<Arc<CatalogOutput>> {
        let digest = Self::library_digest(&input)?;
        let index = self.cursor;
        self.cursor += 1;
        if let Some(cached) = self.cache.get(index)
            && cached.digest.byte_size == digest.byte_size
            && cached.digest.sha256 == digest.sha256
            && cached.origin.package_key == key
            && let Response::Library(output) = &cached.decision
        {
            self.last = Some(cached.origin.clone());
            return Ok(Arc::clone(output));
        }
        self.cache.truncate(index);
        Err(Flow::Need(Need::Library(Box::new(input))))
    }
    pub(super) fn rejected<T>(&self, error: WritingError) -> Read<T> {
        let cause = self
            .last
            .clone()
            .map(|origin| FailureCause::NativeRejection(Box::new(origin)))
            .unwrap_or(FailureCause::SourceReadUnavailable);
        Err(Flow::Fail(Failure { error, cause }))
    }
    pub(super) fn resolve(&mut self, decided: DecidedProject, key: &str) -> NativeResult<()> {
        let digest = streaming_payload::write_json(std::io::sink(), &decided.output.input)
            .map_err(|cause| {
                super::super::super::decided_failure(
                    "storage_error",
                    cause.to_string(),
                    key,
                    &decided.instance_id,
                    &decided.request_id,
                )
            })?
            .1;
        self.cache.push(Cached {
            digest,
            decision: Response::Writing(Arc::new(decided.output.decision)),
            origin: DecisionOrigin {
                package_key: key.into(),
                instance_id: decided.instance_id,
                native_request_id: decided.request_id,
            },
        });
        Ok(())
    }
    pub(super) fn call(&mut self, input: Input) -> Read<Arc<Decision>> {
        let digest = streaming_payload::write_json(std::io::sink(), &input)
            .map_err(|cause| original(WritingError::Io(std::io::Error::other(cause))))?
            .1;
        let index = self.cursor;
        self.cursor += 1;
        if let Some(cached) = self.cache.get(index)
            && cached.digest.byte_size == digest.byte_size
            && cached.digest.sha256 == digest.sha256
            && let Response::Writing(decision) = &cached.decision
        {
            self.last = Some(cached.origin.clone());
            let reason = match decision.as_ref() {
                Decision::BibliographyMutation { decision } => match decision.as_ref() {
                    research_workbench_native_sdk::writing_bibliography::mutation::Decision::Rejected { reason } => {
                        return Err(Flow::Fail(Failure {
                            error: reason.clone().into(),
                            cause: FailureCause::NativeRejection(Box::new(cached.origin.clone())),
                        }));
                    }
                    _ => None,
                },
                Decision::BibliographyRekey { decision } => match decision.as_ref() {
                    research_workbench_native_sdk::writing_bibliography::rekey::Decision::Rejected { reason } => {
                        return Err(Flow::Fail(Failure {
                            error: super::super::bibliography_rekey::rejection(reason.clone()),
                            cause: FailureCause::NativeRejection(Box::new(cached.origin.clone())),
                        }));
                    }
                    _ => None,
                },
                Decision::BibliographyEntries { decision } => match decision.as_ref() {
                    EntryDecision::Rejected { reason } => Some(reason.clone()),
                    _ => None,
                },
                Decision::BibliographyAdoption { decision } => match decision.as_ref() {
                    AdoptionDecision::Rejected { reason } => Some(reason.clone()),
                    _ => None,
                },
                Decision::BibliographyPublication { decision } => match decision.as_ref() {
                    PublicationDecision::Rejected { reason } => Some(reason.clone()),
                    _ => None,
                },
                Decision::Rejected { reason } => {
                    return Err(Flow::Fail(Failure {
                        error: project_rejection(reason.clone()),
                        cause: FailureCause::NativeRejection(Box::new(cached.origin.clone())),
                    }));
                }
                _ => None,
            };
            if let Some(reason) = reason {
                return Err(Flow::Fail(Failure {
                    error: super::super::bibliography_entries::bibliography_error(reason),
                    cause: FailureCause::NativeRejection(Box::new(cached.origin.clone())),
                }));
            }
            return Ok(Arc::clone(decision));
        }
        self.cache.truncate(index);
        Err(Flow::Need(Need::Native(Box::new(input))))
    }
    pub(super) fn mismatch<T>(&self) -> Read<T> {
        Err(Flow::Fail(Failure {
            error: WritingError::InvalidBibtexEntry,
            cause: FailureCause::SourceReadUnavailable,
        }))
    }
    pub(super) fn adoption(&mut self, input: AdoptionInput) -> Read<AdoptionDecision> {
        match self
            .call(Input::BibliographyAdoption {
                input: Box::new(input),
            })?
            .as_ref()
        {
            Decision::BibliographyAdoption { decision } => Ok(*decision.clone()),
            _ => self.mismatch(),
        }
    }
    pub(super) fn rekey(
        &mut self,
        input: research_workbench_native_sdk::writing_bibliography::rekey::Input,
    ) -> Read<research_workbench_native_sdk::writing_bibliography::rekey::Decision> {
        match self
            .call(Input::BibliographyRekey {
                input: Box::new(input),
            })?
            .as_ref()
        {
            Decision::BibliographyRekey { decision } => Ok(*decision.clone()),
            _ => self.mismatch(),
        }
    }
    pub(super) fn mutation(
        &mut self,
        input: research_workbench_native_sdk::writing_bibliography::mutation::Input,
    ) -> Read<research_workbench_native_sdk::writing_bibliography::mutation::Decision> {
        match self
            .call(Input::BibliographyMutation {
                input: Box::new(input),
            })?
            .as_ref()
        {
            Decision::BibliographyMutation { decision } => Ok(*decision.clone()),
            _ => self.mismatch(),
        }
    }
    pub(super) fn publication(&mut self, input: PublicationInput) -> Read<PublicationDecision> {
        match self
            .call(Input::BibliographyPublication {
                input: Box::new(input),
            })?
            .as_ref()
        {
            Decision::BibliographyPublication { decision } => Ok(*decision.clone()),
            _ => self.mismatch(),
        }
    }
    pub(super) fn adoption_reply(
        decided: DecidedProject,
        key: &str,
    ) -> NativeResult<AdoptionDecision> {
        match decided.output.decision {
            Decision::BibliographyAdoption { decision } => match *decision {
                AdoptionDecision::Rejected { reason } => Err(super::super::super::decided_failure(
                    super::super::bibliography_entries::bibliography_error(reason.clone()).code(),
                    reason.to_string(),
                    key,
                    &decided.instance_id,
                    &decided.request_id,
                )),
                decision => Ok(decision),
            },
            _ => Err(super::super::super::decided_failure(
                "plugin_host_response_mismatch",
                "写作书目接纳阶段不匹配。",
                key,
                &decided.instance_id,
                &decided.request_id,
            )),
        }
    }
    pub(super) fn publication_reply(
        decided: DecidedProject,
        key: &str,
    ) -> NativeResult<PublicationDecision> {
        match decided.output.decision {
            Decision::BibliographyPublication { decision } => match *decision {
                PublicationDecision::Rejected { reason } => {
                    Err(super::super::super::decided_failure(
                        super::super::bibliography_entries::bibliography_error(reason.clone())
                            .code(),
                        reason.to_string(),
                        key,
                        &decided.instance_id,
                        &decided.request_id,
                    ))
                }
                decision => Ok(decision),
            },
            _ => Err(super::super::super::decided_failure(
                "plugin_host_response_mismatch",
                "写作书目发布阶段不匹配。",
                key,
                &decided.instance_id,
                &decided.request_id,
            )),
        }
    }
}
fn project_rejection(
    reason: research_workbench_native_sdk::writing_project::Rejection,
) -> WritingError {
    reason.into()
}
