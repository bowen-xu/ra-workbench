//! One finite structured owner: database attempts settle before pure Native waits.
use super::*;
use crate::{
    ai::structured_resource_validation::DecisionOrigin,
    hbs_storage::boundary::{BoundaryError, HbsNeed, HbsOutcome},
    runtime_plugins::literature_hbs::{
        HbsOperation, HbsOperationError, OperationError, ResourceOperationError, RuntimeHbsSources,
        StructuredOperation, StructuredOutput, operation_error,
    },
    storage::AppStore,
    structured_resources::{
        AdoptionDraft, HistoryDraft, Resource, ReviewDraft, WriteDraft, WriteResult,
    },
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    structured_validation::{StructuredValidationInputV1, StructuredValidationOutcomeV1},
};
use serde_json::Value;
#[cfg(unix)]
mod calls;
mod selection;
#[cfg(all(test, unix))]
mod tests;
mod worker;

impl LiteratureTraceService {
    pub(crate) async fn bind_resource_template(
        self: &Arc<Self>,
        store: AppStore,
        input: AdoptionDraft,
    ) -> Result<Resource, ResourceOperationError> {
        let request = input.request_id.clone();
        self.resource_operation(
            store,
            input.template_id.clone(),
            StructuredOperation::BindTemplate(input),
            &request,
        )
        .await
    }
    /// The original text backend publishes after its creation scope is saved.
    pub(crate) async fn write_text_resource(
        self: &Arc<Self>,
        store: AppStore,
        input: WriteDraft,
    ) -> Result<WriteResult, ResourceOperationError> {
        let request = input.request_id.clone();
        match self
            .structured_operation(
                store,
                input.resource_id.clone(),
                StructuredOperation::TextWrite(input),
            )
            .await
            .map_err(|error| error.into_write(&request))?
        {
            StructuredOutput::Write(result) => Ok(*result),
            _ => Err(OperationError::unknown().into_write(&request)),
        }
    }
    pub(crate) async fn write_resource(
        self: &Arc<Self>,
        store: AppStore,
        input: WriteDraft,
    ) -> Result<WriteResult, ResourceOperationError> {
        let request = input.request_id.clone();
        match self
            .structured_operation(
                store,
                input.resource_id.clone(),
                StructuredOperation::Write(input),
            )
            .await
            .map_err(|error| error.into_write(&request))?
        {
            StructuredOutput::Write(result) => Ok(*result),
            _ => Err(OperationError::unknown().into_write(&request)),
        }
    }
    pub(crate) async fn structured_hbs(
        self: &Arc<Self>,
        store: AppStore,
        id: String,
        operation: HbsOperation,
    ) -> Result<Value, HbsOperationError> {
        match self
            .structured_operation(store, id, StructuredOperation::Hbs(Arc::new(operation)))
            .await
            .map_err(OperationError::into_hbs)?
        {
            StructuredOutput::Hbs(value) => Ok(value),
            StructuredOutput::Resource(_) | StructuredOutput::Write(_) => {
                Err(HbsOperationError::unknown())
            }
        }
    }
    pub(crate) async fn review_resource(
        self: &Arc<Self>,
        store: AppStore,
        input: ReviewDraft,
    ) -> Result<Resource, ResourceOperationError> {
        let request = input.request_id.clone();
        self.resource_operation(
            store,
            input.resource_id.clone(),
            StructuredOperation::Review(input),
            &request,
        )
        .await
    }
    pub(crate) async fn restore_resource_history(
        self: &Arc<Self>,
        store: AppStore,
        input: HistoryDraft,
    ) -> Result<Resource, ResourceOperationError> {
        let request = input.request_id.clone();
        self.resource_operation(
            store,
            input.resource_id.clone(),
            StructuredOperation::History(input),
            &request,
        )
        .await
    }
    async fn resource_operation(
        self: &Arc<Self>,
        store: AppStore,
        id: String,
        operation: StructuredOperation,
        request: &str,
    ) -> Result<Resource, ResourceOperationError> {
        match self
            .structured_operation(store, id, operation)
            .await
            .map_err(|error| error.into_resource(request))?
        {
            StructuredOutput::Resource(resource) => Ok(*resource),
            StructuredOutput::Hbs(_) | StructuredOutput::Write(_) => {
                Err(OperationError::unknown().into_resource(request))
            }
        }
    }
    async fn structured_operation(
        self: &Arc<Self>,
        store: AppStore,
        id: String,
        operation: StructuredOperation,
    ) -> Result<StructuredOutput, OperationError> {
        let owner = Arc::clone(self);
        // One accepted owner covers final SQL, postcommit reads and notify;
        // dropping the IPC waiter cannot detach that work from shutdown's gate.
        tokio::spawn(async move { owner.structured_operation_owned(store, id, operation).await })
            .await
            .map_err(|_| OperationError::unknown())?
    }
    async fn structured_operation_owned(
        &self,
        store: AppStore,
        id: String,
        operation: StructuredOperation,
    ) -> Result<StructuredOutput, OperationError> {
        self.available().map_err(|error| {
            HbsOperationError::native(error, "execution", HbsOutcome::NotStarted)
        })?;
        let _lifetime = self.doi_operations.read().await;
        self.available().map_err(|error| {
            HbsOperationError::native(error, "execution", HbsOutcome::NotStarted)
        })?;
        let operation = Arc::new(operation);
        let mut sources = RuntimeHbsSources::default();
        let mut provider_key: Option<String> = None;
        let mut selection_accepted = false;
        let mut template_accepted = false;
        #[cfg(unix)]
        let mut slot = None;
        loop {
            sources.policy.restart();
            let (next, result) = worker::attempt_operation(
                store.clone(),
                id.clone(),
                Arc::clone(&operation),
                sources,
            )
            .await?;
            sources = next;
            let pause = match result {
                Ok(value) => return Ok(value),
                Err(BoundaryError::Failure(error)) => return Err(operation_error(error)),
                Err(BoundaryError::Pause(pause)) => pause,
            };
            let outcome = pause.outcome();
            match pause.need() {
                HbsNeed::EditableTemplate { slot: index, input } => {
                    if !template_accepted {
                        self.available().map_err(|error| {
                            HbsOperationError::native(error, "sources", outcome)
                        })?;
                        let catalog = Arc::clone(&self.catalog);
                        let explicit = provider_key.clone().or_else(|| {
                            store
                                .native_note_trace()
                                .map(|binding| binding.package_key.clone())
                        });
                        provider_key = Some(
                            tokio::task::spawn_blocking(move || {
                                selection::select_editable_template(&catalog, explicit.as_deref())
                            })
                            .await
                            .map_err(|_| OperationError::unknown())?
                            .map_err(|error| {
                                HbsOperationError::native(error, "sources", outcome)
                            })?,
                        );
                        template_accepted = true;
                    }
                    let key = provider_key
                        .as_deref()
                        .ok_or_else(OperationError::unknown)?;
                    #[cfg(not(unix))]
                    {
                        let _ = (index, input, key);
                        return Err(unsupported(outcome));
                    }
                    #[cfg(unix)]
                    {
                        if slot.is_none() {
                            slot = Some(self.host.lock().await);
                        }
                        let current = slot.as_mut().ok_or_else(OperationError::unknown)?;
                        let mut before = None;
                        let (decision, origin) = match self
                            .editable_template_decision(current, key, &input, &mut before)
                            .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                return Err(self
                                    .finish_structured_failure(current, key, before, error, outcome)
                                    .await);
                            }
                        };
                        if !sources
                            .policy
                            .store_template(index, input, decision, origin)
                        {
                            return Err(OperationError::unknown());
                        }
                    }
                }
                HbsNeed::Sources => {
                    self.available()
                        .map_err(|error| HbsOperationError::native(error, "sources", outcome))?;
                    let catalog = Arc::clone(&self.catalog);
                    let key = provider_key.clone().or_else(|| {
                        store
                            .native_note_trace()
                            .map(|binding| binding.package_key.clone())
                    });
                    let reading = tokio::task::spawn_blocking(move || {
                        selection::select(&catalog, key.as_deref())
                    })
                    .await
                    .map_err(|_| OperationError::unknown())?
                    .map_err(|error| HbsOperationError::native(error, "sources", outcome))?;
                    provider_key = Some(reading.package_key().to_owned());
                    sources.reading = Some(reading);
                }
                HbsNeed::Structured { slot: index, input } => {
                    let key = sources
                        .reading
                        .as_ref()
                        .map(|reading| reading.package_key().to_owned())
                        .ok_or_else(OperationError::unknown)?;
                    #[cfg(not(unix))]
                    {
                        let _ = (index, input, key);
                        return Err(unsupported(outcome));
                    }
                    #[cfg(unix)]
                    {
                        if slot.is_none() {
                            slot = Some(self.host.lock().await);
                        }
                        let current = slot.as_mut().ok_or_else(OperationError::unknown)?;
                        let mut before = None;
                        let (decision, origin) =
                            match self.hbs_decision(current, &key, &input, &mut before).await {
                                Ok(value) => value,
                                Err(error) => {
                                    return Err(self
                                        .finish_structured_failure(
                                            current, &key, before, error, outcome,
                                        )
                                        .await);
                                }
                            };
                        if !sources.policy.store(index, input, decision, origin) {
                            return Err(OperationError::unknown());
                        }
                    }
                }
                HbsNeed::Unreviewed { slot: index, input } => {
                    if !selection_accepted {
                        self.available().map_err(|error| {
                            HbsOperationError::native(error, "sources", outcome)
                        })?;
                        let catalog = Arc::clone(&self.catalog);
                        let explicit = provider_key.clone().or_else(|| {
                            store
                                .native_note_trace()
                                .map(|binding| binding.package_key.clone())
                        });
                        provider_key = Some(
                            tokio::task::spawn_blocking(move || {
                                selection::select_unreviewed(&catalog, explicit.as_deref())
                            })
                            .await
                            .map_err(|_| OperationError::unknown())?
                            .map_err(|error| {
                                HbsOperationError::native(error, "sources", outcome)
                            })?,
                        );
                        selection_accepted = true;
                    }
                    let key = provider_key
                        .as_deref()
                        .ok_or_else(OperationError::unknown)?;
                    #[cfg(not(unix))]
                    {
                        let _ = (index, input, key);
                        return Err(unsupported(outcome));
                    }
                    #[cfg(unix)]
                    {
                        if slot.is_none() {
                            slot = Some(self.host.lock().await);
                        }
                        let current = slot.as_mut().ok_or_else(OperationError::unknown)?;
                        let mut before = None;
                        let selected = match self
                            .unreviewed_selection(current, key, &input, &mut before)
                            .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                return Err(self
                                    .finish_structured_failure(current, key, before, error, outcome)
                                    .await);
                            }
                        };
                        if !sources.policy.store_selection(index, input, selected) {
                            return Err(OperationError::unknown());
                        }
                    }
                }
            }
        }
    }
}
#[cfg(not(unix))]
fn unsupported(outcome: HbsOutcome) -> OperationError {
    HbsOperationError::native(
        LiteratureTraceService::error(
            "plugin_host_platform_unsupported",
            "此平台尚未提供文献宿主。",
        ),
        "validation",
        outcome,
    )
    .into()
}
