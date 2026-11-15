//! Core keeps the original reads and archive statement around finite Native projections.
use super::*;
use crate::{
    ai::{error::AiError, structured_resource_validation::DecisionOrigin},
    data_interface::guarded_transaction,
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::{
        literature_context::source_failure,
        literature_reading_history::{
            self as history,
            failure::{native_wire, source_wire, worker_failure},
            resources::ReadingHistoryResources,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    reading_history::{
        ReadingHistoryDecisionV1 as Decision, ReadingHistoryInputV1 as Input,
        ReadingHistoryRequestV1 as Request,
    },
};
use serde_json::{Value, json};

pub(super) struct DecidedHistory {
    pub input: Input,
    pub decision: Box<Decision>,
    pub origin: DecisionOrigin,
}
impl LiteratureTraceService {
    pub(crate) async fn reading_history(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: Request,
    ) -> Result<Value, Value> {
        if let Request::Bootstrap {
            paper_id,
            attachment_ids,
        } = request
        {
            return self
                .prepare_reading_bootstrap(store, key, paper_id, attachment_ids)
                .await;
        }
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.reading_history_owned(store, key, request).await })
            .await
            .map_err(|_| {
                native_wire(
                    Self::error(
                        "plugin_host_source_worker_unsettled",
                        "旧阅读历史操作未确认完成。",
                    )
                    .package(&selected),
                )
            })?
    }
    async fn reading_history_owned(
        &self,
        store: AppStore,
        key: String,
        request: Request,
    ) -> Result<Value, Value> {
        self.available().map_err(native_wire)?;
        let _lifetime = self.doi_operations.read().await;
        self.available().map_err(native_wire)?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_reading_history(&catalog, &selected)?;
            ReadingHistoryResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            native_wire(Self::error(
                "plugin_host_source_worker_unsettled",
                "旧阅读历史资源读取未确认完成。",
            ))
        })?
        .map_err(native_wire)?;
        let resources = Arc::new(resources);
        #[cfg(not(unix))]
        {
            let _ = (resources, store, request);
            Err(native_wire(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            )))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            let result = async {
                let host = self.ensure_host(&mut slot, &key).await?;
                // The original App acquired storage before validating an empty identity.
                let initial = if let Request::Debug { turn_id } = &request {
                    let source = store.clone();
                    match tokio::task::spawn_blocking(move || {
                        source
                            .connection()
                            .map(|_| ())
                            .map_err(|error| source_failure(error.into()))
                    })
                    .await
                    {
                        Ok(Ok(())) => Input::DebugIdentity {
                            turn_id: turn_id.clone(),
                        },
                        Ok(Err(error)) => return Ok(Err(source_wire(*error, &key, None))),
                        Err(_) => {
                            return Err(Self::error(
                                "plugin_host_source_worker_unsettled",
                                "旧阅读存储检查未确认完成。",
                            ));
                        }
                    }
                } else {
                    match capture(store.clone(), Arc::clone(&resources), request.clone()).await {
                        Ok(input) => input,
                        Err(error) => return Ok(Err(source_wire(*error, &key, None))),
                    }
                };
                let mut decided = self.reading_history_stage(host, &key, initial).await?;
                self.available()?;
                if let Decision::DebugIdentity { turn_id } = decided.decision.as_ref() {
                    let input = match capture(
                        store.clone(),
                        Arc::clone(&resources),
                        Request::Debug {
                            turn_id: turn_id.clone(),
                        },
                    )
                    .await
                    {
                        Ok(input) => input,
                        Err(error) => {
                            return Ok(Err(source_wire(*error, &key, Some(&decided.origin))));
                        }
                    };
                    decided = self.reading_history_stage(host, &key, input).await?;
                    self.available()?;
                }
                Ok(match *decided.decision {
                    Decision::List { sessions } => Ok(json!(sessions)),
                    Decision::Session { session } => Ok(json!(session)),
                    Decision::DebugRecord { data } => Ok(json!(data)),
                    Decision::Archive { session_id } => {
                        match archive(store, resources, session_id).await {
                            Ok(changed) => Ok(json!(changed)),
                            Err(error) => Err(source_wire(*error, &key, Some(&decided.origin))),
                        }
                    }
                    Decision::Rejected { message } => {
                        let mut error = Self::error("invalid_input", message).package(&key);
                        error.instance_id = Some(decided.origin.instance_id);
                        error.request_id = Some(decided.origin.native_request_id);
                        Err(native_wire(error))
                    }
                    _ => {
                        return Err(host.error(
                            "plugin_host_response_mismatch",
                            "旧阅读历史阶段种类不匹配。",
                        ));
                    }
                })
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(native_wire)?
        }
    }

    #[cfg(unix)]
    pub(super) async fn reading_history_stage(
        &self,
        host: &mut NativeHost,
        key: &str,
        input: Input,
    ) -> NativeResult<DecidedHistory> {
        self.available()?;
        let expected = input.clone();
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let before = host.request_identity();
        let output = tokio::select! {
            biased;
            _ = &mut stopped => Err(Self::error(
                "plugin_host_stopping", "应用正在关闭旧阅读历史操作。",
            )),
            output = host.catalog_call(
                CatalogInput::ProjectReadingHistory { input: Box::new(input) },
                Duration::from_secs(60),
            ) => output,
        }
        .map_err(|mut error| {
            match host.request_identity() {
                Some((instance, request))
                    if before.as_ref() != Some(&(instance.clone(), request.clone())) =>
                {
                    error.instance_id = Some(instance);
                    error.request_id = Some(request);
                }
                _ => error.request_id = None,
            }
            error
        })?;
        let CatalogOutput::ReadingHistory { input, decision } = output else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "旧阅读历史结果种类不匹配。",
            ));
        };
        if *input != expected || !decision_matches(&input, &decision) {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "旧阅读历史结果与本次输入不匹配。",
            ));
        }
        let Some((instance_id, native_request_id)) = host.request_identity() else {
            return Err(host.error(
                "plugin_host_response_mismatch",
                "旧阅读历史结果缺少本次身份。",
            ));
        };
        Ok(DecidedHistory {
            input: *input,
            decision,
            origin: DecisionOrigin {
                package_key: key.into(),
                instance_id,
                native_request_id,
            },
        })
    }
}
fn decision_matches(input: &Input, decision: &Decision) -> bool {
    match (input, decision) {
        (_, Decision::Rejected { .. }) => true,
        (Input::List { rows }, Decision::List { sessions }) => {
            rows.len() == sessions.len()
                && rows
                    .iter()
                    .zip(sessions)
                    .all(|(row, session)| row.id == session.id && row.paper_id == session.paper_id)
        }
        (
            Input::Session {
                summary: Some(row),
                turns,
            },
            Decision::Session { session },
        ) => {
            row.id == session.summary.id
                && row.paper_id == session.summary.paper_id
                && turns.len() == session.turns.len()
                && turns
                    .iter()
                    .zip(&session.turns)
                    .all(|(row, turn)| row.id == turn.id && row.session_id == turn.session_id)
        }
        (Input::DebugRecord { row: Some(row) }, Decision::DebugRecord { data }) => {
            row.turn_id == data.turn_id
        }
        (
            Input::BootstrapSources { .. },
            Decision::HistoryRequired {} | Decision::Bootstrap { .. },
        )
        | (Input::BootstrapHistory { .. }, Decision::Bootstrap { .. })
        | (Input::BootstrapSnapshot { .. }, Decision::Bootstrap { .. }) => true,
        (Input::DebugIdentity { turn_id: requested }, Decision::DebugIdentity { turn_id }) => {
            requested.trim() == turn_id
        }
        (
            Input::Archive {
                session_id: requested,
            },
            Decision::Archive { session_id },
        ) => requested.trim() == session_id,
        (
            Input::BootstrapHistory {
                workflow: Some(workflow),
            },
            Decision::SnapshotRequired {
                context_snapshot_id,
            },
        ) => workflow.context_snapshot_id == *context_snapshot_id,
        _ => false,
    }
}
async fn capture(
    store: AppStore,
    resources: Arc<ReadingHistoryResources>,
    request: Request,
) -> crate::runtime_plugins::literature_context::ContextResult<Input> {
    tokio::task::spawn_blocking(move || {
        let db = store
            .connection()
            .map_err(|error| source_failure(error.into()))?;
        history::capture(&db, &resources, &request)
    })
    .await
    .map_err(|_| worker_failure(false))?
}
async fn archive(
    store: AppStore,
    resources: Arc<ReadingHistoryResources>,
    id: String,
) -> crate::runtime_plugins::literature_context::ContextResult<bool> {
    tokio::task::spawn_blocking(move || {
        let mut db = store
            .connection()
            .map_err(|error| source_failure(error.into()))?;
        guarded_transaction::run_mut(&mut db, |db| {
            let mut result = history::archive(db, &resources, &id);
            let unknown = !db.is_autocommit();
            if unknown {
                let mut error = result
                    .err()
                    .unwrap_or_else(|| source_failure(AiError::TemporaryUnavailable));
                error.outcome = HbsOutcome::Unknown;
                result = Err(error);
            }
            (result, unknown)
        })
    })
    .await
    .map_err(|_| worker_failure(true))?
}
