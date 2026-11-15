//! Application-selected paper policy dispatch on the existing Core runtime.
use super::{
    credential::JsonCredentialStore, error::AiError, model::AiRun, provider::CancellationSignal,
};
use crate::storage::AppStore;
use std::sync::atomic::AtomicU64;
pub(crate) async fn execute_overview(
    app: &tauri::AppHandle,
    store: &AppStore,
    credentials: &JsonCredentialStore,
    run: &AiRun,
    signal: &CancellationSignal,
    sequence: &AtomicU64,
) -> Result<(), AiError> {
    if run.task_kind == "paper_profile_classification"
        && let Some(binding) = store.native_note_trace().cloned()
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
        return super::overview_execution::execute_overview(&host, run, signal).await;
    }
    (store.require_generation_policy_source()?.execute_overview)(
        app,
        store,
        credentials,
        run,
        signal,
        sequence,
    )
    .await
}
