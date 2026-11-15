//! Static IPC composition: literature chooses portrait policy, while Core owns
//! run history, original response decoding and the application's SQL guard.

use crate::{
    ai::{
        error::AiError,
        run_history::{AiCommandError, AiRun},
    },
    storage::AppStore,
};

use tauri::State;

/// Production bridge shared by the IPC worker and existing integration tests.

#[tauri::command]
pub async fn get_latest_paper_portrait_run(
    store: State<'_, AppStore>,
    paper_id: String,
) -> Result<Option<AiRun>, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .latest_portrait_run(store, binding.package_key, paper_id)
            .await;
    }

    {
        let _ = paper_id;
        Err(AiError::CapabilityUnavailable("literature_library.paper_portrait_history").into())
    }
}
