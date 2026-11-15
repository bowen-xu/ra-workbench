//! Original algorithm stages retain their distinct commits and the Core operation owner.
use super::{
    knowledge_failure::{self, KnowledgeMutationKind, native_ai},
    knowledge_policy::NativeKnowledgePolicy,
    *,
};
use crate::{
    ai::{error::AiError, structured_resource_validation::Flow},
    data_interface::guarded_transaction,
    hbs_storage::boundary::{self, BoundaryError, HbsOutcome},
    runtime_plugins::{
        literature_knowledge_promotion::{
            KnowledgeSql, PromotionError, algorithm_io::AlgorithmRequest, algorithm_runs, evolution,
        },
        native::{SafeDatabaseError, StorageMutationOutcome},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::wire::{WorkflowDecisionV1, WorkflowInputV1},
    knowledge_promotion::{
        KnowledgeValidationInputV1 as Input,
        algorithms::{AlgorithmIdKindV1, ConceptAlgorithmRun, SharedFoundationsDraft},
    },
};

struct StartedAlgorithm {
    id: String,
    initial: Result<(), PromotionError<(usize, Box<Input>)>>,
}

impl LiteratureTraceService {
    pub(crate) async fn concept_evolution(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        concept_id: String,
    ) -> NativeResult<ConceptAlgorithmRun> {
        self.run_algorithm(store, key, AlgorithmRequest::Evolution(concept_id))
            .await
    }

    pub(crate) async fn shared_foundations(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: SharedFoundationsDraft,
    ) -> NativeResult<ConceptAlgorithmRun> {
        self.run_algorithm(store, key, AlgorithmRequest::Foundations(draft))
            .await
    }

    async fn run_algorithm(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: AlgorithmRequest,
    ) -> NativeResult<ConceptAlgorithmRun> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.algorithm_owned(store, key, request).await })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &selected, None))?
    }

    async fn algorithm_owned(
        &self,
        store: AppStore,
        key: String,
        request: AlgorithmRequest,
    ) -> NativeResult<ConceptAlgorithmRun> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let mut sql = Arc::new(
            self.algorithm_sql(&key, request.needs_foundation_resources())
                .await?,
        );
        let mut policy = NativeKnowledgePolicy::new(None, request.source_message());
        let started = loop {
            self.available()?;
            let source = store.clone();
            let request = request.clone();
            let key_for_start = key.clone();
            let (next_policy, next_sql, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut guard = source
                        .connection()
                        .map_err(AiError::from)
                        .map_err(PromotionError::Failed)?;
                    guarded_transaction::run(&mut guard, |db| {
                        let prepared = match request.prepare_start(db, &sql, &mut policy) {
                            Ok(prepared) => prepared,
                            Err(error) => return (Err(error), false),
                        };
                        let result = request.insert_start(db, &sql, &prepared);
                        let unknown = !db.is_autocommit();
                        match result {
                            Ok(()) => {
                                // Preserve the original single-guard sequence from start through source capture.
                                let initial =
                                    request.prepare_rows(db, &sql, &mut policy).map(|_| ());
                                (
                                    Ok(StartedAlgorithm {
                                        id: prepared.snapshot.id,
                                        initial,
                                    }),
                                    unknown,
                                )
                            }
                            Err(error) => (
                                Err(PromotionError::Failed(native_ai(statement_error(
                                    error.into(),
                                    "start",
                                    &key_for_start,
                                    &prepared.snapshot.id,
                                    unknown,
                                )))),
                                unknown,
                            ),
                        }
                    })
                })();
                (policy, sql, result)
            })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &key, None))?;
            policy = next_policy;
            sql = next_sql;
            match result {
                Ok(started) => break started,
                Err(PromotionError::Failed(error)) => return Err(command_error(error, &key)),
                Err(PromotionError::Need((ordinal, input))) => {
                    self.algorithm_rule(&key, &mut policy, ordinal, input)
                        .await?
                }
            }
        };
        let run_id = started.id.clone();
        let result = self
            .algorithm_results(&store, &key, &request, started, &sql, policy)
            .await;
        let count = match result {
            Ok(count) => count,
            Err(mut failure) => {
                // The original result failure marks the already-persisted run as failed, best effort.
                if !failure
                    .concept_algorithm
                    .as_ref()
                    .is_some_and(|detail| detail.outcome == StorageMutationOutcome::Unknown)
                {
                    let source = store.clone();
                    let id = run_id.clone();
                    let resources = sql.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(guard) = source.connection() {
                            algorithm_runs::fail(&guard, &resources, &id);
                        }
                    })
                    .await;
                }
                attach_run(&mut failure, &run_id, "validation");
                return Err(failure);
            }
        };
        let source = store.clone();
        let selected = key.clone();
        let id = run_id.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = source
                .connection()
                .map_err(AiError::from)
                .map_err(|error| command_error(error, &selected))?;
            guarded_transaction::run(&mut guard, |db| {
                let result = algorithm_runs::finish(db, &sql, &id, count);
                let unknown = !db.is_autocommit();
                (
                    result.map_err(|error| {
                        statement_error(error.into(), "finish", &selected, &id, unknown)
                    }),
                    unknown,
                )
            })?;
            algorithm_runs::get(&guard, &sql, &id).map_err(|error| {
                mutation_error(
                    Some(error.into()),
                    "postcommit-read",
                    HbsOutcome::Committed,
                    &selected,
                    None,
                )
            })
        })
        .await
        .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, &key, None))?
        .map_err(|mut failure| {
            attach_run(&mut failure, &run_id, "finish");
            failure
        })
    }

    async fn algorithm_results(
        &self,
        store: &AppStore,
        key: &str,
        request: &AlgorithmRequest,
        started: StartedAlgorithm,
        sql: &Arc<KnowledgeSql>,
        mut policy: NativeKnowledgePolicy,
    ) -> NativeResult<usize> {
        match started.initial {
            Ok(()) => {}
            Err(PromotionError::Failed(error)) => return Err(command_error(error, key)),
            Err(PromotionError::Need((ordinal, input))) => {
                self.algorithm_rule(key, &mut policy, ordinal, input)
                    .await?
            }
        }
        let run_id = started.id;
        loop {
            self.available()?;
            let source = store.clone();
            let selected = key.to_owned();
            let resources = sql.clone();
            let request = request.clone();
            let run_id = run_id.clone();
            let (next_policy, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut guard = source
                        .connection()
                        .map_err(AiError::from)
                        .map_err(PromotionError::Failed)?;
                    let rows = request.prepare_rows(&guard, &resources, &mut policy)?;
                    guarded_transaction::run_mut(&mut guard, |db| {
                        let tx = match db.transaction() {
                            Ok(tx) => tx,
                            Err(error) => {
                                return (Err(PromotionError::Failed(error.into())), false);
                            }
                        };
                        let result: Result<_, Flow<AiError, std::convert::Infallible>> = rows
                            .write(&tx, &resources, &run_id)
                            .map_err(|error| Flow::original(error.into()));
                        let settled = boundary::settle_with(
                            tx,
                            result,
                            "write",
                            true,
                            true,
                            AiError::from,
                            |_| AiError::TemporaryUnavailable,
                        );
                        match boundary::confirm_with(db, settled, true, |_| {
                            AiError::TemporaryUnavailable
                        }) {
                            Ok(count) => (Ok(count), false),
                            Err(BoundaryError::Pause(pause)) => match pause.need() {},
                            Err(BoundaryError::Failure(failure)) => {
                                let unknown = failure.needs_recovery();
                                (
                                    Err(PromotionError::Failed(native_ai(mutation_error(
                                        Some(failure.failure.error),
                                        failure.phase,
                                        failure.outcome,
                                        &selected,
                                        failure.rollback_error,
                                    )))),
                                    unknown,
                                )
                            }
                        }
                    })
                })();
                (policy, result)
            })
            .await
            .map_err(|_| mutation_error(None, "worker", HbsOutcome::Unknown, key, None))?;
            policy = next_policy;
            match result {
                Ok(count) => return Ok(count),
                Err(PromotionError::Failed(error)) => return Err(command_error(error, key)),
                Err(PromotionError::Need((ordinal, input))) => {
                    self.algorithm_rule(key, &mut policy, ordinal, input)
                        .await?
                }
            }
        }
    }

    pub(crate) async fn concept_algorithm_run(
        &self,
        store: AppStore,
        key: String,
        run_id: String,
    ) -> NativeResult<ConceptAlgorithmRun> {
        self.available()?;
        let _lifetime = self.doi_operations.read().await;
        let sql = self.algorithm_sql(&key, false).await?;
        // The original command acquires its store before ID validation.
        let source = store.clone();
        tokio::task::spawn_blocking(move || source.connection().map(drop))
            .await
            .map_err(|_| Self::error("worker_unavailable", "算法读取工作线程不可用。"))?
            .map_err(|error| command_error(error.into(), &key))?;
        let mut policy = NativeKnowledgePolicy::new(None, "算法运行读取来源已变化。");
        self.algorithm_rule(
            &key,
            &mut policy,
            0,
            Box::new(Input::AlgorithmId {
                purpose: AlgorithmIdKindV1::Run,
                is_empty: run_id.trim().is_empty(),
                bytes: run_id.len(),
            }),
        )
        .await?;
        tokio::task::spawn_blocking(move || {
            let guard = store.connection().map_err(AiError::from)?;
            // Replaying the same owned observation projects the original Native refusal.
            evolution::validate_run_id(&mut policy, &run_id).map_err(|error| match error {
                PromotionError::Failed(error) => error,
                PromotionError::Need(_) => AiError::TemporaryUnavailable,
            })?;
            algorithm_runs::get(&guard, &sql, run_id.trim()).map_err(Into::into)
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "算法读取工作线程不可用。"))?
        .map_err(|error| command_error(error, &key))
    }

    async fn algorithm_sql(&self, key: &str, with_foundations: bool) -> NativeResult<KnowledgeSql> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            package::accept_concepts(&catalog, &selected)?;
            (if with_foundations {
                KnowledgeSql::load_foundations(&catalog, &package)
            } else {
                KnowledgeSql::load_algorithm(&catalog, &package)
            })
            .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| Self::error("worker_unavailable", "算法资源工作线程不可用。"))?
    }

    async fn algorithm_rule(
        &self,
        key: &str,
        policy: &mut NativeKnowledgePolicy,
        ordinal: usize,
        input: Box<Input>,
    ) -> NativeResult<()> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                key,
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
                    "概念算法校验种类不匹配。",
                    &origin,
                ),
                key,
            ));
        };
        policy.remember(*input, decision, origin);
        Ok(())
    }
}

fn command_error(error: AiError, key: &str) -> Box<NativeHostError> {
    knowledge_failure::command_error(KnowledgeMutationKind::ConceptAlgorithm, error, key)
}
fn mutation_error(
    error: Option<AiError>,
    phase: &'static str,
    outcome: HbsOutcome,
    key: &str,
    rollback: Option<Box<SafeDatabaseError>>,
) -> Box<NativeHostError> {
    knowledge_failure::mutation_error(
        KnowledgeMutationKind::ConceptAlgorithm,
        error,
        phase,
        outcome,
        key,
        rollback,
    )
}
fn attach_run(failure: &mut NativeHostError, run_id: &str, phase: &'static str) {
    let detail = failure.concept_algorithm.get_or_insert_with(|| {
        Box::new(crate::runtime_plugins::native::StorageMutationDiagnostic {
            phase,
            outcome: StorageMutationOutcome::NotStarted,
            object_id: None,
            revision: None,
            database_error: None,
            rollback_error: None,
        })
    });
    detail.object_id = Some(run_id.to_owned());
}

fn statement_error(
    error: AiError,
    phase: &'static str,
    key: &str,
    run_id: &str,
    unknown: bool,
) -> Box<NativeHostError> {
    let outcome = if unknown {
        HbsOutcome::Unknown
    } else {
        HbsOutcome::NotStarted
    };
    let mut failure = mutation_error(Some(error), phase, outcome, key, None);
    if !unknown {
        failure
            .concept_algorithm
            .as_mut()
            .expect("algorithm mutation diagnostics are always attached")
            .outcome = StorageMutationOutcome::StatementFailed;
        failure.message = "概念算法语句执行失败，请按原运行ID核对已保存记录和结果。".into();
    }
    attach_run(&mut failure, run_id, phase);
    failure
}
