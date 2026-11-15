//! Human capture reuses finite knowledge rules and the original Core transaction.
use super::knowledge_policy::NativeKnowledgePolicy;
use super::*;
use crate::{
    ai::{
        error::AiError,
        structured_resource_validation::{DecisionOrigin, Flow},
    },
    data_interface::guarded_transaction,
    hbs_storage::boundary::{self, BoundaryError},
    runtime_plugins::{
        literature_concepts::{ConceptFailure, ConceptOutcome, ConceptProblem, ConceptResources},
        literature_knowledge_promotion::{
            KnowledgeIo, KnowledgeSql, PromotionError,
            capture::{CaptureIds, read_capture},
            manual::read_definition,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    knowledge_promotion::{
        capture::{ConceptCaptureDraft, ConceptCaptureResult},
        manual::DefinitionClaimDraft,
        models::DefinitionClaim,
    },
};

#[derive(Clone)]
enum KnowledgeWrite {
    Capture(ConceptCaptureDraft),
    Definition(DefinitionClaimDraft),
}
enum SavedTarget {
    Capture(CaptureIds),
    Definition(String),
}
enum SavedKnowledge {
    Capture(Box<ConceptCaptureResult>),
    Definition(DefinitionClaim),
}
impl SavedTarget {
    fn read(
        self,
        db: &rusqlite::Connection,
        sql: &KnowledgeSql,
    ) -> Result<SavedKnowledge, PromotionError<std::convert::Infallible>> {
        match self {
            Self::Capture(ids) => {
                read_capture(db, sql, &ids).map(|value| SavedKnowledge::Capture(Box::new(value)))
            }
            Self::Definition(id) => read_definition(db, sql, &id).map(SavedKnowledge::Definition),
        }
    }
}

impl LiteratureTraceService {
    pub(crate) async fn capture_concept(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: ConceptCaptureDraft,
    ) -> Result<ConceptCaptureResult, AiError> {
        match self
            .run_knowledge_write(store, key, KnowledgeWrite::Capture(draft))
            .await?
        {
            SavedKnowledge::Capture(value) => Ok(*value),
            _ => Err(AiError::TemporaryUnavailable),
        }
    }
    pub(crate) async fn save_definition(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: DefinitionClaimDraft,
    ) -> Result<DefinitionClaim, AiError> {
        match self
            .run_knowledge_write(store, key, KnowledgeWrite::Definition(draft))
            .await?
        {
            SavedKnowledge::Definition(value) => Ok(value),
            _ => Err(AiError::TemporaryUnavailable),
        }
    }
    async fn run_knowledge_write(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: KnowledgeWrite,
    ) -> Result<SavedKnowledge, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.knowledge_write_owned(store, key, request).await })
            .await
            .map_err(|_| unknown(&selected))?
    }
    async fn knowledge_write_owned(
        &self,
        store: AppStore,
        key: String,
        request: KnowledgeWrite,
    ) -> Result<SavedKnowledge, AiError> {
        self.available().map_err(technical)?;
        let _lifetime = self.doi_operations.read().await;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let manual = matches!(&request, KnowledgeWrite::Definition(_));
        let (mut sql, semantic) = tokio::task::spawn_blocking(move || {
            let package =
                package::accept_analysis_workflow(&catalog, &selected).map_err(technical)?;
            package::accept_concepts(&catalog, &selected).map_err(technical)?;
            let map = |cause: crate::runtime_plugins::RuntimePluginError| {
                technical(Self::error(cause.code, cause.message).package(&selected))
            };
            Ok::<_, AiError>((
                if manual {
                    KnowledgeSql::load_manual(&catalog, &package)
                } else {
                    KnowledgeSql::load_capture(&catalog, &package)
                }
                .map_err(map)?,
                ConceptResources::load(&catalog, &package).map_err(map)?,
            ))
        })
        .await
        .map_err(|_| unknown(&key))??;
        let mut policy =
            NativeKnowledgePolicy::new(Some(semantic), "概念捕获来源已变化，请重新载入。");
        loop {
            self.available().map_err(technical)?;
            let source = store.clone();
            let request = request.clone();
            let selected = key.clone();
            let (next_policy, next_sql, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut guard = source
                        .connection()
                        .map_err(|e| Flow::original(AiError::from(e)))?;
                    guarded_transaction::run_mut(&mut guard, |db| {
                        let tx = match db.transaction() {
                            Ok(tx) => tx,
                            Err(error) => {
                                return (Err(Flow::original(AiError::from(error))), false);
                            }
                        };
                        let mut io = KnowledgeIo::new(&tx, &sql, &mut policy);
                        let result = match &request {
                            KnowledgeWrite::Capture(draft) => {
                                io.capture(draft).map(SavedTarget::Capture)
                            }
                            KnowledgeWrite::Definition(draft) => {
                                io.save_definition(draft).map(SavedTarget::Definition)
                            }
                        }
                        .map_err(flow);
                        let settled = boundary::settle_with(
                            tx,
                            result,
                            "write",
                            true,
                            true,
                            AiError::from,
                            |_| AiError::TemporaryUnavailable,
                        );
                        let result = match boundary::confirm_with(db, settled, true, |_| {
                            AiError::TemporaryUnavailable
                        }) {
                            Ok(ids) => ids.read(db, &sql).map_err(|error| match error {
                                PromotionError::Failed(error) => Flow::original(storage_failure(
                                    error,
                                    "postcommit-read",
                                    ConceptOutcome::Committed,
                                    &selected,
                                )),
                                PromotionError::Need(never) => match never {},
                            }),
                            Err(BoundaryError::Pause(pause)) => Err(Flow::Need(pause.need())),
                            Err(BoundaryError::Failure(failure)) if failure.needs_recovery() => {
                                let mut detail = ConceptFailure::new(
                                    failure.phase,
                                    storage_problem(failure.failure.error),
                                );
                                detail.outcome = ConceptOutcome::Unknown;
                                detail.rollback_error = failure.rollback_error;
                                return (
                                    Err(Flow::original(technical(
                                        detail.native().package(&selected),
                                    ))),
                                    true,
                                );
                            }
                            Err(BoundaryError::Failure(failure)) => match failure.failure.error {
                                error @ (AiError::Database(_)
                                | AiError::Knowledge(
                                    crate::ai::knowledge_contracts::KnowledgeError::Database(_),
                                )) => Err(Flow::original(storage_failure(
                                    error,
                                    failure.phase,
                                    ConceptOutcome::RolledBack,
                                    &selected,
                                ))),
                                error => Err(Flow::original(error)),
                            },
                        };
                        let unknown = !db.is_autocommit();
                        (result, unknown)
                    })
                })();
                (policy, sql, result)
            })
            .await
            .map_err(|_| unknown(&key))?;
            policy = next_policy;
            sql = next_sql;
            match result {
                Ok(value) => return Ok(value),
                Err(Flow::Fail(failure)) => return Err(failure.error),
                Err(Flow::Need((ordinal, input))) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(
                            &key,
                            WorkflowInputV1::KnowledgeValidation {
                                ordinal,
                                input: input.clone(),
                            },
                        )
                        .await
                        .map_err(technical)?;
                    let WorkflowDecisionV1::KnowledgeValidation { decision } = decision else {
                        return Err(identified(
                            "plugin_host_response_mismatch",
                            "概念捕获校验种类不匹配。",
                            &origin,
                        ));
                    };
                    policy.remember(*input, decision, origin);
                }
            }
        }
    }
}
fn flow<P>(error: PromotionError<P>) -> Flow<AiError, P> {
    match error {
        PromotionError::Failed(error) => Flow::original(error),
        PromotionError::Need(need) => Flow::Need(need),
    }
}
pub(super) fn technical(mut failure: Box<NativeHostError>) -> AiError {
    let code = match failure.concepts.as_ref().map(|detail| detail.outcome) {
        Some(ConceptOutcome::Unknown) => "data_operation_outcome_unknown",
        Some(_) => "storage_error",
        None => "native_workflow_failed",
    };
    if let Some(detail) = &failure.concepts {
        if let ConceptProblem::Database(cause) = detail.problem.as_ref() {
            failure.message.push_str(&format!("（{cause}）"));
        }
        if let Some(cause) = &detail.rollback_error {
            failure.message.push_str(&format!("（{cause}）"));
        }
    }
    if code == "data_operation_outcome_unknown" {
        failure.code = code.into();
    }
    AiError::NativeWorkflow { code, failure }
}
pub(super) fn identified(code: &'static str, message: &str, origin: &DecisionOrigin) -> AiError {
    let mut failure = LiteratureTraceService::error(code, message).package(&origin.package_key);
    failure.instance_id = Some(origin.instance_id.clone());
    failure.request_id = Some(origin.native_request_id.clone());
    AiError::NativeWorkflow { code, failure }
}
fn unknown(key: &str) -> AiError {
    technical(ConceptFailure::unknown().native().package(key))
}

fn storage_problem(error: AiError) -> ConceptProblem {
    match error {
        AiError::Database(cause)
        | AiError::Knowledge(crate::ai::knowledge_contracts::KnowledgeError::Database(cause)) => {
            ConceptProblem::Database(Box::new(
                crate::runtime_plugins::native::SafeDatabaseError::sql("capture", &cause),
            ))
        }
        _ => ConceptProblem::Codec,
    }
}
fn storage_failure(
    error: AiError,
    phase: &'static str,
    outcome: ConceptOutcome,
    key: &str,
) -> AiError {
    let mut detail = ConceptFailure::new(phase, storage_problem(error));
    detail.outcome = outcome;
    technical(detail.native().package(key))
}
