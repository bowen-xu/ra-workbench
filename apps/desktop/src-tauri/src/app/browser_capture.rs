//! App selects the original static composition or one exact Native capture binding.
use crate::{
    AppStore,
    browser_connector::{BrowserConnector, CaptureHandler, CaptureRequest},
    runtime_plugins::literature_browser_capture::CaptureFailure,
};
use serde_json::Value;
use tauri::AppHandle;

pub(crate) fn start(app: &AppHandle, store: AppStore) -> BrowserConnector {
    let handler = if let Some(binding) = store.native_note_trace() {
        binding
            .service
            .browser_capture_declared(&binding.package_key)
            .map(|_| dispatch as CaptureHandler)
            .map_err(|error| error.to_string())
    } else {
        Err("当前工作台组合未启用浏览器采集。".to_owned())
    };
    BrowserConnector::start(app, store, handler)
}

pub(crate) fn dispatch(
    store: &AppStore,
    scope: &str,
    request: CaptureRequest,
) -> Result<Value, CaptureFailure> {
    if let Some(binding) = store.native_note_trace().cloned() {
        let (input, bytes) = request.into_input(scope);
        return tauri::async_runtime::block_on(binding.service.browser_capture(
            store.clone(),
            binding.package_key,
            scope.to_owned(),
            input,
            bytes,
        ))
        .map_err(CaptureFailure::Native);
    }

    {
        let _ = request;
        Err(CaptureFailure::Unsupported)
    }
}
