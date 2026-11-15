//! App selects the original static adapter or one explicit Native reading binding.
use crate::storage::AppStore;
use research_workbench_native_sdk::reading_history::ReadingHistoryRequestV1 as Request;
use serde_json::Value;
use tauri::State;

async fn execute(store: &AppStore, request: Request) -> Result<Value, Value> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .reading_history(store.clone(), binding.package_key, request)
            .await;
    }

    {
        let _ = request;
        Err(
            serde_json::json!({"code":"capability_unavailable","message":"当前工作台组合未提供旧阅读历史。"}),
        )
    }
}
#[tauri::command]
pub async fn prepare_paper_reading_bootstrap(
    store: State<'_, AppStore>,
    paper_id: String,
    attachment_ids: Vec<String>,
) -> Result<Value, Value> {
    execute(
        &store,
        Request::Bootstrap {
            paper_id,
            attachment_ids,
        },
    )
    .await
}
#[tauri::command]
pub async fn list_paper_ai_reading_sessions(
    store: State<'_, AppStore>,
    paper_id: String,
) -> Result<Value, Value> {
    execute(&store, Request::List { paper_id }).await
}
#[tauri::command]
pub async fn get_paper_ai_reading_session(
    store: State<'_, AppStore>,
    session_id: String,
) -> Result<Value, Value> {
    execute(&store, Request::Session { session_id }).await
}
#[tauri::command]
pub async fn get_paper_ai_reading_turn_debug_data(
    store: State<'_, AppStore>,
    turn_id: String,
) -> Result<Value, Value> {
    execute(&store, Request::Debug { turn_id }).await
}
#[tauri::command]
pub async fn delete_paper_ai_reading_session(
    store: State<'_, AppStore>,
    session_id: String,
) -> Result<Value, Value> {
    execute(&store, Request::Archive { session_id }).await
}
