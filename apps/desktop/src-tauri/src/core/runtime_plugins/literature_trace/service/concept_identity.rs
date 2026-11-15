//! Original identity chain and the existing Core operation/settlement owners.
use super::{
    knowledge_failure::{self, KnowledgeMutationKind, native_ai},
    knowledge_policy::NativeKnowledgePolicy,
    *,
};
use crate::{
    ai::{
        error::AiError, knowledge_contracts::KnowledgeError, structured_resource_validation::Flow,
    },
    data_interface::guarded_transaction,
    hbs_storage::boundary::{self, BoundaryError, HbsOutcome},
    runtime_plugins::{
        literature_concepts::ConceptResources,
        literature_knowledge_promotion::{KnowledgeIo, KnowledgeSql, PromotionError, identity},
        native::SafeDatabaseError,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    knowledge_promotion::identity::{IdentityOutputV1, IdentityRequestV1},
};
impl LiteratureTraceService {
    pub(crate) async fn concept_identity(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: IdentityRequestV1,
    ) -> NativeResult<IdentityOutputV1> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.identity_owned(store, key, request).await })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &selected, None))?
    }
    async fn identity_owned(
        &self,
        store: AppStore,
        key: String,
        request: IdentityRequestV1,
    ) -> NativeResult<IdentityOutputV1> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let writing = matches!(
            &request,
            IdentityRequestV1::Merge { .. } | IdentityRequestV1::Split { .. }
        );
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let (mut sql, semantic) = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            package::accept_concepts(&catalog, &selected)?;
            let sql = KnowledgeSql::load_identity(&catalog, &package)
                .map_err(|e| Self::error(e.code, e.message).package(&selected))?;
            let semantic = if writing {
                Some(
                    ConceptResources::load(&catalog, &package)
                        .map_err(|e| Self::error(e.code, e.message).package(&selected))?,
                )
            } else {
                None
            };
            Ok::<_, Box<NativeHostError>>((sql, semantic))
        })
        .await
        .map_err(|_| {
            Self::error("worker_unavailable", "身份资源工作线程不可用。").package(&key)
        })??;
        let mut policy = NativeKnowledgePolicy::new(semantic, "概念身份来源已变化，请重新预览。");
        loop {
            self.available()?;
            let source = store.clone();
            let selected = key.clone();
            let request = request.clone();
            let (next_policy, next_sql, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut guard = source
                        .connection()
                        .map_err(AiError::from)
                        .map_err(PromotionError::Failed)?;
                    match &request {
                        IdentityRequestV1::Preview { draft } => {
                            return identity::preview_merge(&guard, &sql, &mut policy, draft)
                                .map(|impact| IdentityOutputV1::Impact { impact });
                        }
                        IdentityRequestV1::List { .. } => {
                            return identity::list_events::<std::convert::Infallible>(&guard, &sql)
                                .map(|events| IdentityOutputV1::Events { events })
                                .map_err(|error| match error {
                                    PromotionError::Failed(error) => PromotionError::Failed(error),
                                    PromotionError::Need(never) => match never {},
                                });
                        }
                        _ => {}
                    }
                    identity::precheck(&mut policy, &request)?;
                    guarded_transaction::run_mut(&mut guard, |db| {
                        let tx = match db.transaction() {
                            Ok(tx) => tx,
                            Err(error) => {
                                return (
                                    Err(PromotionError::Failed(AiError::Database(error))),
                                    false,
                                );
                            }
                        };
                        let result = KnowledgeIo::new(&tx, &sql, &mut policy)
                            .write_identity(&request)
                            .map_err(|error| match error {
                                PromotionError::Failed(error) => Flow::original(error),
                                PromotionError::Need(need) => Flow::Need(need),
                            });
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
                            Ok(id) => identity::identity_event_from::<std::convert::Infallible>(
                                db, &sql, &id,
                            )
                            .map(|event| IdentityOutputV1::Event { event })
                            .map_err(|error| match error {
                                PromotionError::Failed(error) => {
                                    let mut failure = mutation_error(
                                        Some(error),
                                        "postcommit-read",
                                        HbsOutcome::Committed,
                                        &selected,
                                        None,
                                    );
                                    if let Some(detail) = failure.concept_identity.as_mut() {
                                        detail.object_id = Some(id);
                                    }
                                    PromotionError::Failed(native_ai(failure))
                                }
                                PromotionError::Need(never) => match never {},
                            }),
                            Err(BoundaryError::Pause(pause)) => {
                                Err(PromotionError::Need(pause.need()))
                            }
                            Err(BoundaryError::Failure(failure)) => {
                                let unknown = failure.needs_recovery();
                                let error = failure.failure.error;
                                let result = if unknown
                                    || matches!(
                                        &error,
                                        AiError::Database(_)
                                            | AiError::Knowledge(KnowledgeError::Database(_))
                                            | AiError::NativeWorkflow { .. },
                                    ) {
                                    native_ai(mutation_error(
                                        Some(error),
                                        failure.phase,
                                        failure.outcome,
                                        &selected,
                                        failure.rollback_error,
                                    ))
                                } else {
                                    error
                                };
                                return (Err(PromotionError::Failed(result)), unknown);
                            }
                        };
                        (result, !db.is_autocommit())
                    })
                })();
                (policy, sql, result)
            })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &key, None))?;
            policy = next_policy;
            sql = next_sql;
            match result {
                Ok(value) => return Ok(value),
                Err(PromotionError::Failed(error)) => {
                    return Err(knowledge_failure::command_error(
                        KnowledgeMutationKind::ConceptIdentity,
                        error,
                        &key,
                    ));
                }
                Err(PromotionError::Need((ordinal, input))) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(
                            &key,
                            WorkflowInputV1::KnowledgeValidation {
                                ordinal,
                                input: input.clone(),
                            },
                        )
                        .await?;
                    let WorkflowDecisionV1::KnowledgeValidation { decision } = decision else {
                        return Err(knowledge_failure::command_error(
                            KnowledgeMutationKind::ConceptIdentity,
                            super::concept_capture::identified(
                                "plugin_host_response_mismatch",
                                "概念身份校验种类不匹配。",
                                &origin,
                            ),
                            &key,
                        ));
                    };
                    policy.remember(*input, decision, origin);
                }
            }
        }
    }
}
fn mutation_error(
    error: Option<AiError>,
    phase: &'static str,
    outcome: HbsOutcome,
    key: &str,
    rollback: Option<Box<SafeDatabaseError>>,
) -> Box<NativeHostError> {
    knowledge_failure::mutation_error(
        KnowledgeMutationKind::ConceptIdentity,
        error,
        phase,
        outcome,
        key,
        rollback,
    )
}
