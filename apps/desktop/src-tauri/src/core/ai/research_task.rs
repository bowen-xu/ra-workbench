//! Selected domain task on the existing Core runtime.
use super::{CancellationSignal, credential::JsonCredentialStore, error::AiError, model::AiRun};
use crate::storage::AppStore;
use std::sync::atomic::AtomicU64;
pub(crate) async fn execute_research_task(
    app: &tauri::AppHandle,
    store: &AppStore,
    credentials: &JsonCredentialStore,
    run: &AiRun,
    signal: &CancellationSignal,
    sequence: &AtomicU64,
) -> Result<(), AiError> {
    if matches!(
        run.task_kind.as_str(),
        "reference_extraction" | "matrix_synthesis"
    ) && let Some(binding) = store.native_note_trace().cloned()
    {
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
        return super::research_execution::execute_research_task(&host, run, signal).await;
    }
    (store
        .require_generation_policy_source()?
        .execute_research_task)(app, store, credentials, run, signal, sequence)
    .await
}
