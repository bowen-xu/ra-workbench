//! Original manual relations. Preflight finishes before BEGIN; Native waits own no DB guard.
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
        literature_knowledge_promotion::{KnowledgeIo, KnowledgeSql, PromotionError, relation},
        native::SafeDatabaseError,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    knowledge_promotion::{
        KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input,
        models::RelationNodeSummary,
        relation::{RelationAssertion, RelationAssertionDraft},
    },
};
impl LiteratureTraceService {
    pub(crate) async fn save_relation(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: RelationAssertionDraft,
    ) -> NativeResult<RelationAssertion> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.relation_owned(store, key, draft).await })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &selected, None))?
    }
    async fn relation_owned(
        &self,
        store: AppStore,
        key: String,
        draft: RelationAssertionDraft,
    ) -> NativeResult<RelationAssertion> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let mut sql = self.relation_sql(key.clone()).await?;
        let mut policy = NativeKnowledgePolicy::new(None, "关系来源已变化，请重新载入。");
        loop {
            self.available()?;
            let source = store.clone();
            let request = draft.clone();
            let selected = key.clone();
            let (next_policy, next_sql, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut guard = source
                        .connection()
                        .map_err(AiError::from)
                        .map_err(PromotionError::Failed)?;
                    // Preserve all original field/node/project/anchor/version reads before BEGIN.
                    let prepared = relation::prepare_relation(&guard, &sql, &mut policy, &request)?;
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
                            .write_relation(&request, &prepared)
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
                            Ok(id) => {
                                relation::read_relation::<std::convert::Infallible>(db, &sql, &id)
                                    .map_err(|error| match error {
                                        PromotionError::Failed(error) => {
                                            PromotionError::Failed(native_ai(mutation_error(
                                                Some(error),
                                                "postcommit-read",
                                                HbsOutcome::Committed,
                                                &selected,
                                                None,
                                            )))
                                        }
                                        PromotionError::Need(never) => match never {},
                                    })
                            }
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
                                            | AiError::Knowledge(KnowledgeError::Database(_),)
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
                Err(PromotionError::Failed(error)) => return Err(command_error(error, &key)),
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
                        return Err(command_error(
                            super::concept_capture::identified(
                                "plugin_host_response_mismatch",
                                "关系校验种类不匹配。",
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
    pub(crate) async fn search_relation_nodes(
        &self,
        store: AppStore,
        key: String,
        query: String,
    ) -> NativeResult<Vec<RelationNodeSummary>> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let sql = self.relation_sql(key.clone()).await?;
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::KnowledgeValidation {
                    ordinal: 0,
                    input: Box::new(Input::RelationSearch {
                        query_characters: query.trim().chars().count(),
                    }),
                },
            )
            .await?;
        let plan = match decision {
            WorkflowDecisionV1::KnowledgeValidation {
                decision: Decision::RelationSearch { result: Ok(plan) },
            } => plan,
            WorkflowDecisionV1::KnowledgeValidation {
                decision:
                    Decision::RelationSearch {
                        result: Err(message),
                    },
            } => {
                return Err(command_error(
                    super::concept_capture::identified("invalid_input", &message, &origin),
                    &key,
                ));
            }
            _ => {
                return Err(command_error(
                    super::concept_capture::identified(
                        "plugin_host_response_mismatch",
                        "关系检索校验种类不匹配。",
                        &origin,
                    ),
                    &key,
                ));
            }
        };
        let selected = key.clone();
        tokio::task::spawn_blocking(move || {
            let db = store
                .connection()
                .map_err(AiError::from)
                .map_err(|e| command_error(e, &selected))?;
            relation::search_nodes::<std::convert::Infallible>(&db, &sql, &query, &plan)
                .map_err(|e| promotion_error(e, &selected))
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "关系检索工作线程不可用。").package(&key))?
    }
    pub(crate) async fn list_relations(
        &self,
        store: AppStore,
        key: String,
        project: Option<String>,
    ) -> NativeResult<Vec<RelationAssertion>> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let sql = self.relation_sql(key.clone()).await?;
        let selected = key.clone();
        tokio::task::spawn_blocking(move || {
            let db = store
                .connection()
                .map_err(AiError::from)
                .map_err(|e| command_error(e, &selected))?;
            relation::list_relations::<std::convert::Infallible>(&db, &sql, project.as_deref())
                .map_err(|e| promotion_error(e, &selected))
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "关系列表工作线程不可用。").package(&key))?
    }
    async fn relation_sql(&self, key: String) -> NativeResult<KnowledgeSql> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            package::accept_concepts(&catalog, &selected)?;
            KnowledgeSql::load_relation(&catalog, &package)
                .map_err(|e| Self::error(e.code, e.message).package(&selected))
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "关系资源工作线程不可用。").package(&key))?
    }
}
fn promotion_error(
    error: PromotionError<std::convert::Infallible>,
    key: &str,
) -> Box<NativeHostError> {
    match error {
        PromotionError::Failed(error) => command_error(error, key),
        PromotionError::Need(never) => match never {},
    }
}
fn command_error(error: AiError, key: &str) -> Box<NativeHostError> {
    knowledge_failure::command_error(KnowledgeMutationKind::Relation, error, key)
}
fn mutation_error(
    error: Option<AiError>,
    phase: &'static str,
    outcome: HbsOutcome,
    key: &str,
    rollback: Option<Box<SafeDatabaseError>>,
) -> Box<NativeHostError> {
    knowledge_failure::mutation_error(
        KnowledgeMutationKind::Relation,
        error,
        phase,
        outcome,
        key,
        rollback,
    )
}
