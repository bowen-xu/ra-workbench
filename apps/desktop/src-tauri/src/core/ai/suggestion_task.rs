//! Selected domain task on the existing Core runtime.
use super::{CancellationSignal, credential::JsonCredentialStore, error::AiError, model::AiRun};
use crate::storage::AppStore;
use std::sync::atomic::AtomicU64;
pub(crate) async fn execute_suggestions(
    app: &tauri::AppHandle,
    store: &AppStore,
    credentials: &JsonCredentialStore,
    run: &AiRun,
    signal: &CancellationSignal,
    sequence: &AtomicU64,
) -> Result<(), AiError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        let host = super::overview_execution::native::NativeOverviewHost {
            execution: super::generation_support::OverviewExecution::new(
                app,
                store,
                credentials,
                run,
                signal,
                sequence,
            ),
            store,
            binding,
        };
        return super::suggestion_execution::execute_suggestions(&host, run, signal).await;
    }
    (store
        .require_generation_policy_source()?
        .execute_suggestions)(app, store, credentials, run, signal, sequence)
    .await
}

pub(super) async fn start(
    app: tauri::AppHandle,
    store: AppStore,
    credentials: JsonCredentialStore,
    manager: super::AiRunManager,
    draft: super::generation_support::AiTaskSpecDraft,
) -> Result<AiRun, AiError> {
    // Retain accepted admission through manager dispatch if the IPC waiter leaves.
    tokio::spawn(async move {
        let binding = store
            .native_note_trace()
            .cloned()
            .ok_or(AiError::CapabilityUnavailable(
                "literature_library.generation_policy_source",
            ))?;
        let run = binding
            .service
            .admit_analysis_overview(
                store.clone(),
                credentials.clone(),
                binding.package_key,
                draft,
                None,
            )
            .await?;
        super::commands::register_and_spawn(app, store, credentials, manager, run).await
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}
