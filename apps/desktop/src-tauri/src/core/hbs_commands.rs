//! Narrow production IPC. Only real notes/resources are addressable; the
//! isolated preview's seed/reset/text/inspection commands are never exposed.
use crate::runtime_plugins::{
    literature_hbs::{HbsOperation, HbsOperationError, NativeFailure},
    literature_trace::service::LiteratureTraceService,
};
use crate::{hbs_storage, storage::AppStore};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
#[path = "hbs_commands/static_operations.rs"]
mod static_operations;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_failure: Option<Box<NativeFailure>>,
}
impl CommandError {
    fn from_cause(cause: String, request: Option<&str>) -> Self {
        let code = cause
            .split('：')
            .next()
            .filter(|code| {
                code.starts_with("MODEL_")
                    && code
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            })
            .unwrap_or("MODEL_STORAGE_ERROR");
        Self {
            code: code.into(),
            message: cause,
            request_id: request.map(str::to_owned),
            native_failure: None,
        }
    }
    fn from_operation(cause: HbsOperationError, request: Option<&str>) -> Self {
        let (message, native_failure) = cause.into_wire();
        let mut error = Self::from_cause(message, request);
        error.native_failure = native_failure.map(Box::new);
        error
    }
}
fn identity(kind: &str, resource: &str) -> Result<(), String> {
    if !matches!(kind, "note" | "structured") {
        return Err("MODEL_INVALID_KIND：仅允许笔记与结构化资源".into());
    }
    if uuid::Uuid::parse_str(resource).is_err() {
        return Err("MODEL_INVALID_RESOURCE：资源身份无效".into());
    }
    Ok(())
}
fn bounded(value: &impl Serialize) -> Result<(), String> {
    // A checkpoint contains several independently validated document snapshots.
    // Keep the envelope bounded without reducing the domain's 8 MiB text limit.
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("MODEL_REQUEST_TOO_LARGE：请求超过容量限制".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn hbs_read_document(
    store: tauri::State<'_, AppStore>,
    kind: String,
    resource_id: String,
) -> Result<Value, CommandError> {
    let result = (|| {
        identity(&kind, &resource_id)?;
        crate::notes::session_assets::read_hbs_for_session(
            &store,
            &kind,
            &resource_id,
            crate::notes::session_assets::HbsRead::Document,
        )
    })();
    result.map_err(|cause| CommandError::from_cause(cause, None))
}

#[tauri::command]
pub(crate) async fn hbs_commit_document(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    kind: String,
    resource_id: String,
    request: hbs_storage::Commit,
) -> Result<Value, CommandError> {
    let request_id = request.id.clone();
    identity(&kind, &resource_id)
        .and_then(|()| bounded(&request))
        .map_err(|cause| CommandError::from_cause(cause, Some(&request_id)))?;
    let store = store.inner().clone();
    if static_path(&store, &kind) {
        return tokio::task::spawn_blocking(move || {
            static_operations::commit(store, kind, resource_id, request)
        })
        .await
        .map_err(|_| static_worker_failure(Some(&request_id)))?
        .map_err(|cause| CommandError::from_cause(cause, Some(&request_id)));
    }
    service
        .structured_hbs(store, resource_id, HbsOperation::Commit(request))
        .await
        .map_err(|cause| CommandError::from_operation(cause, Some(&request_id)))
}

#[tauri::command]
pub(crate) async fn hbs_read_preservation(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    kind: String,
    resource_id: String,
) -> Result<Value, CommandError> {
    identity(&kind, &resource_id).map_err(|cause| CommandError::from_cause(cause, None))?;
    let store = store.inner().clone();
    if static_path(&store, &kind) {
        return tokio::task::spawn_blocking(move || {
            crate::notes::session_assets::read_hbs_for_session(
                &store,
                &kind,
                &resource_id,
                crate::notes::session_assets::HbsRead::Preservation,
            )
        })
        .await
        .map_err(|_| static_worker_failure(None))?
        .map_err(|cause| CommandError::from_cause(cause, None));
    }
    service
        .structured_hbs(store, resource_id, HbsOperation::ReadPreservation)
        .await
        .map_err(|cause| CommandError::from_operation(cause, None))
}

#[tauri::command]
pub(crate) async fn hbs_save_preservation(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    kind: String,
    resource_id: String,
    checkpoint: Value,
) -> Result<Value, CommandError> {
    identity(&kind, &resource_id)
        .and_then(|()| bounded(&checkpoint))
        .map_err(|cause| CommandError::from_cause(cause, None))?;
    let store = store.inner().clone();
    if static_path(&store, &kind) {
        return tokio::task::spawn_blocking(move || {
            let mut db = store.connection().map_err(|error| error.to_string())?;
            hbs_storage::preservation::save(
                store.generation_policy_source(),
                store.paper_reading_source(),
                &mut db,
                &kind,
                &resource_id,
                checkpoint,
            )
        })
        .await
        .map_err(|_| static_worker_failure(None))?
        .map_err(|cause| CommandError::from_cause(cause, None));
    }
    service
        .structured_hbs(
            store,
            resource_id,
            HbsOperation::SavePreservation(checkpoint),
        )
        .await
        .map_err(|cause| CommandError::from_operation(cause, None))
}

fn static_path(store: &AppStore, kind: &str) -> bool {
    kind != "structured"
        || store.generation_policy_source().is_some()
        || store.paper_reading_source().is_some()
}

fn static_worker_failure(request: Option<&str>) -> CommandError {
    CommandError::from_cause(
        "MODEL_STORAGE_ERROR：Core HBS 工作线程未能确认结算；原请求与内容已保留。".into(),
        request,
    )
}

#[cfg(test)]
#[path = "hbs_commands/native_tests.rs"]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_boundary_is_closed_and_diagnostics_keep_request_identity() {
        let id = "00000000-0000-4000-8000-000000000186";
        assert!(identity("note", id).is_ok());
        assert!(identity("structured", id).is_ok());
        assert!(identity("text", id).is_err());
        assert!(identity("note", "../../outside").is_err());
        let error = CommandError::from_cause("MODEL_DRAFT_PENDING：草稿保留".into(), Some("req:1"));
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({"code":"MODEL_DRAFT_PENDING","message":"MODEL_DRAFT_PENDING：草稿保留","requestId":"req:1"})
        );
        assert!(serde_json::from_value::<hbs_storage::Commit>(serde_json::json!({"id":"req","expectedVersion":1,"saved":null,"baseline":null,"sql":"unexpected"})).is_err());
    }
}
