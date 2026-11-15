//! Existing accepted-operation lifetime and transaction replay for the four original commands.
use super::super::analysis_workflow::template;
use super::*;
use crate::{
    ai::{
        error::AiError,
        structured_resource_validation::{Failure, FailureCause, Flow},
    },
    data_interface::guarded_transaction,
    runtime_plugins::literature_paper_confirmation::{
        ConfirmationPolicy, ConfirmationRepository, ConfirmationResources, ConfirmationResult,
        ConfirmationState,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::paper_confirmation::{
    PaperConfirmationRequestV1 as Request,
    wire::{PaperConfirmationDecisionV1 as Decision, PaperConfirmationInputV1 as Input},
};

struct Observation {
    input: Input,
    encoded: String,
    result: PaperConfirmationResultV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
struct Policy {
    observations: Vec<Observation>,
    last_origin: Option<DecisionOrigin>,
}
impl ConfirmationPolicy for Policy {
    type Pause = Box<Input>;
    fn decide(&mut self, input: Input) -> Result<Decision, Flow<AiError, Self::Pause>> {
        let Some(previous) = self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &input))
        else {
            return Err(Flow::Need(Box::new(input)));
        };
        let encoded = encode_payload(&input).map_err(|_| {
            Flow::original(template::identified(
                "plugin_host_source_input_invalid",
                "确认来源无法编码。",
                &previous.origin,
            ))
        })?;
        if encoded != previous.encoded {
            return Err(Flow::original(template::identified(
                "plugin_host_source_changed",
                &format!("确认的 {} 来源已变化，请重新载入。", stage_name(&input)),
                &previous.origin,
            )));
        }
        previous.result.clone().map_err(|error| {
            Flow::Fail(Failure {
                error: error.into(),
                cause: FailureCause::NativeRejection(Box::new(previous.origin.clone())),
            })
        })
    }
    fn complete_module(&mut self) {
        self.observations.clear();
        self.last_origin = None;
    }
}
fn same_rule(left: &Input, right: &Input) -> bool {
    match (left, right) {
        (Input::Request { request: a }, Input::Request { request: b }) => {
            std::mem::discriminant(a) == std::mem::discriminant(b)
        }
        (Input::UndoReplayKey { paper: a, .. }, Input::UndoReplayKey { paper: b, .. }) => a == b,
        (Input::OverviewField { field: a, .. }, Input::OverviewField { field: b, .. }) => {
            a.field == b.field
        }
        (
            Input::Unit {
                scope_key: a,
                input: x,
            },
            Input::Unit {
                scope_key: b,
                input: y,
            },
        ) => a == b && std::mem::discriminant(x) == std::mem::discriminant(y),
        (
            Input::MetadataSource { field_name: a, .. },
            Input::MetadataSource { field_name: b, .. },
        ) => a == b,
        _ => std::mem::discriminant(left) == std::mem::discriminant(right),
    }
}

impl LiteratureTraceService {
    pub(crate) async fn paper_confirmation(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: Request,
    ) -> Result<ConfirmationResult, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move { owner.confirmation_owned(store, key, request).await })
            .await
            .map_err(|_| unsettled(&selected))?
    }
    async fn confirmation_owned(
        &self,
        store: AppStore,
        key: String,
        request: Request,
    ) -> Result<ConfirmationResult, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        self.available().map_err(template::technical)?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let source = store.clone();
        let resources = tokio::task::spawn_blocking(move || {
            // Original App admission obtained storage before parameter validation.
            drop(source.connection().map_err(AiError::from)?);
            let package = package::accept_paper_confirmation(&catalog, &selected)
                .map_err(template::technical)?;
            ConfirmationResources::load(&catalog, &package)
                .map(Arc::new)
                .map_err(|error| template::technical(resource_error(error, &selected)))
        })
        .await
        .map_err(|_| unsettled(&key))??;
        #[cfg(not(unix))]
        {
            let _ = (resources, request);
            Err(template::technical(
                Self::error(
                    "plugin_host_platform_unsupported",
                    "此平台尚未提供文献宿主。",
                )
                .package(&key),
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available().map_err(template::technical)?;
            let result = async {
                let host = self.ensure_host(&mut slot, &key).await?;
                let mut policy = Policy::default();
                let mut state = ConfirmationState::default();
                loop {
                    self.available()?;
                    let source = store.clone();
                    let resources = Arc::clone(&resources);
                    let request = request.clone();
                    let joined = tokio::task::spawn_blocking(move || {
                        let result = (|| {
                            let mut db = source
                                .connection()
                                .map_err(AiError::from)
                                .map_err(Flow::original)?;
                            guarded_transaction::run_mut(&mut db, |db| {
                                let result = ConfirmationRepository::new(db, &resources, &mut policy)
                                    .execute(request, &mut state);
                                let unknown = !db.is_autocommit()
                                    || matches!(
                                        &result,
                                        Err(Flow::Fail(failure))
                                            if failure.error.code() == "data_operation_outcome_unknown"
                                    );
                                (result, unknown)
                            })
                        })();
                        (policy, state, result)
                    })
                    .await;
                    let (next_policy, next_state, result) = match joined {
                        Ok(value) => value,
                        Err(_) => return Ok(Err(unsettled(&key))),
                    };
                    policy = next_policy;
                    state = next_state;
                    match result {
                        Ok(result) => return Ok(Ok(result)),
                        Err(Flow::Fail(failure)) => {
                            return Ok(Err(failed(failure, &key, policy.last_origin.as_ref())));
                        }
                        Err(Flow::Need(input)) => {
                            let encoded = encode_payload(input.as_ref()).map_err(|_| {
                                host.error("plugin_host_source_input_invalid", "确认来源无法编码。")
                            })?;
                            let (result, origin) = self
                                .paper_confirmation_stage(host, &key, *input.clone())
                                .await?;
                            policy.last_origin = Some(origin.clone());
                            policy.observations.push(Observation {
                                input: *input,
                                encoded,
                                result,
                                origin,
                            });
                        }
                    }
                }
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(template::technical)?
        }
    }
}

fn resource_error(
    error: crate::runtime_plugins::RuntimePluginError,
    key: &str,
) -> Box<NativeHostError> {
    let mut native = LiteratureTraceService::error(error.code, error.message).package(key);
    // These are the literal, digest-pinned resources requested by ConfirmationResources.
    if let Some(resource) = error
        .resource
        .as_deref()
        .and_then(|path| path.strip_prefix("data/"))
        .and_then(|name| name.strip_suffix(".v1.sql"))
        && (resource.starts_with("paper-confirmation-")
            || matches!(
                resource,
                "reading-unit-accepted-insert"
                    | "reading-unit-accepted-evidence"
                    | "nal-bootstrap-research-revision-insert"
            ))
    {
        native.data_error = Some(Box::new(
            crate::data_interface::plugin_resources::PluginDataError {
                code: "plugin_data_resource_unavailable",
                message: "确认操作的固定存储资源不可用。",
                package_key: key.into(),
                resource_id: format!("literature.{resource}"),
                resource_version: 1,
                phase: "resource",
                safe_cause: error.code.into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            },
        ));
    }
    native
}

fn failed(failure: Failure<AiError>, key: &str, last: Option<&DecisionOrigin>) -> AiError {
    if let FailureCause::NativeRejection(origin) = failure.cause {
        return template::rejected(failure.error, &origin);
    }
    if let FailureCause::SourceDatabase(database) = failure.cause {
        let code = failure.error.code();
        let mut native =
            LiteratureTraceService::error(code, failure.error.user_message()).package(key);
        if let Some(origin) = last {
            native.instance_id = Some(origin.instance_id.clone());
            native.request_id = Some(origin.native_request_id.clone());
        }
        native.paper_confirmation = Some(Box::new(
            crate::runtime_plugins::native::StorageMutationDiagnostic {
                phase: database.phase,
                outcome: crate::hbs_storage::boundary::HbsOutcome::NotStarted,
                object_id: None,
                revision: None,
                database_error: Some(database),
                rollback_error: None,
            },
        ));
        return AiError::NativeWorkflow {
            code,
            failure: native,
        };
    }
    let mut error = failure.error;
    if let AiError::NativeWorkflow { failure, .. } = &mut error {
        if failure.package_key.is_none() {
            failure.package_key = Some(key.into())
        }
        if let Some(origin) = last {
            if failure.instance_id.is_none() {
                failure.instance_id = Some(origin.instance_id.clone())
            }
            if failure.request_id.is_none() {
                failure.request_id = Some(origin.native_request_id.clone())
            }
        }
    }
    if !matches!(&error, AiError::NativeWorkflow { .. })
        && let Some(origin) = last
    {
        return template::rejected(error, origin);
    }
    error
}
fn unsettled(key: &str) -> AiError {
    let mut failure = LiteratureTraceService::error(
        "plugin_host_source_worker_unsettled",
        "确认操作未确认完成，请核对原批次和幂等键。",
    )
    .package(key);
    failure.paper_confirmation = Some(Box::new(
        crate::runtime_plugins::native::StorageMutationDiagnostic {
            phase: "execution",
            outcome: crate::hbs_storage::boundary::HbsOutcome::Unknown,
            object_id: None,
            revision: None,
            database_error: None,
            rollback_error: None,
        },
    ));
    AiError::NativeWorkflow {
        code: "data_operation_outcome_unknown",
        failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_failure_retains_unknown_outcome_and_only_known_identity() {
        let error = unsettled("selected-package");
        assert_eq!(error.code(), "data_operation_outcome_unknown");
        let value = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
        assert_eq!(
            value["nativeFailure"]["code"],
            "plugin_host_source_worker_unsettled"
        );
        assert_eq!(
            value["nativeFailure"]["paperConfirmation"]["outcome"],
            "unknown"
        );
        assert_eq!(value["nativeFailure"]["packageKey"], "selected-package");
        assert!(value["nativeFailure"]["instanceId"].is_null());
        assert!(value["nativeFailure"]["requestId"].is_null());
    }

    #[test]
    fn paper_and_nested_overview_undo_have_distinct_rule_identity() {
        let input = |paper| Input::UndoReplayKey {
            existing_batch_id: None,
            requested_batch_id: "batch".into(),
            paper,
        };
        assert!(!same_rule(&input(true), &input(false)));
        assert!(same_rule(&input(false), &input(false)));
    }

    #[test]
    fn changed_input_keeps_the_actual_observed_rule_identity() {
        let input = Input::UndoReplayKey {
            existing_batch_id: None,
            requested_batch_id: "original-batch".into(),
            paper: false,
        };
        let origin = DecisionOrigin {
            package_key: "a".repeat(64),
            instance_id: "instance".into(),
            native_request_id: "observed-request".into(),
        };
        let mut policy = Policy {
            observations: vec![Observation {
                encoded: encode_payload(&input).unwrap(),
                input,
                result: Ok(Decision::UndoReplayKey {}),
                origin: origin.clone(),
            }],
            last_origin: Some(origin),
        };
        let changed = Input::UndoReplayKey {
            existing_batch_id: None,
            requested_batch_id: "changed-batch".into(),
            paper: false,
        };
        let Err(Flow::Fail(failure)) = policy.decide(changed) else {
            panic!("changed rule source")
        };
        assert!(
            matches!(failure.error,AiError::NativeWorkflow{failure,..} if failure.code=="plugin_host_source_changed" && failure.request_id.as_deref()==Some("observed-request"))
        );
        policy.complete_module();
        assert!(policy.observations.is_empty());
        assert!(policy.last_origin.is_none());
    }
}
