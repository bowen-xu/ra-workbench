//! Development API transport over the same registered Tauri commands.
//! Uses Tauri's public Webview::on_message and AppHandle::invoke_key APIs.
//! The upstream get_ipc_response pattern is documented at
//! https://github.com/tauri-apps/tauri/blob/7cd71369c00978a3783b6ae3e9972358abbe4ae6/crates/tauri/src/test/mod.rs#L296-L312

use research_agent_core::mcp::{FrameDecoder, Request};
use research_workbench_native_sdk::protocol::MAX_FRAME_BYTES;
use serde_json::{Value, json};
use std::io::{Read, Write};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeError, InvokeResponse};
use tauri::{AppHandle, Manager, Webview};

pub(crate) fn configure_context(
    mut context: tauri::Context<tauri::Wry>,
) -> Result<tauri::Context<tauri::Wry>, String> {
    if !std::env::args_os().any(|argument| argument == "--workbench-api-test-host") {
        return Ok(context);
    }
    let directory = std::env::var_os("RESEARCH_ASSISTANT_DEVELOPMENT_DATA_DIRECTORY")
        .ok_or("workbench_api_test_host_requires_development_data_directory")?;
    let directory = std::path::PathBuf::from(directory);
    if !directory.is_absolute() || directory.parent().is_none() {
        return Err("workbench_api_test_host_data_directory_invalid".into());
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("workbench_api_test_host_data_directory_failed: {error}"))?;
    let directory = directory
        .canonicalize()
        .map_err(|error| format!("workbench_api_test_host_data_directory_failed: {error}"))?;
    let window = context
        .config_mut()
        .app
        .windows
        .iter_mut()
        .find(|window| window.label == "main")
        .ok_or("workbench_api_test_host_window_configuration_unavailable")?;
    // The locked Tauri runtime omits profile fields when converting WindowConfig.
    // Create this window through the explicit public builder in setup instead.
    window.create = false;
    window.data_directory = Some(directory);
    Ok(context)
}

pub(crate) fn create_window(app: &tauri::App) -> Result<(), String> {
    if !std::env::args_os().any(|argument| argument == "--workbench-api-test-host") {
        return Ok(());
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .ok_or("workbench_api_test_host_window_configuration_unavailable")?;
    let directory = config
        .data_directory
        .as_ref()
        .ok_or("workbench_api_test_host_requires_development_data_directory")?;
    let builder = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|error| format!("workbench_api_test_host_window_failed: {error}"))?;
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(
        *uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            directory.as_os_str().as_encoded_bytes(),
        )
        .as_bytes(),
    );
    #[cfg(not(target_os = "macos"))]
    let builder = builder.data_directory(directory.join("webview"));
    builder
        .build()
        .map_err(|error| format!("workbench_api_test_host_window_failed: {error}"))?;
    Ok(())
}

pub(crate) fn start(app: &AppHandle) -> Result<(), String> {
    if !std::env::args_os().any(|argument| argument == "--workbench-api-test-host") {
        return Ok(());
    }
    if std::env::var_os("RESEARCH_ASSISTANT_DEVELOPMENT_DATA_DIRECTORY").is_none() {
        return Err("workbench_api_test_host_requires_development_data_directory".into());
    }
    let window = app
        .get_webview_window("main")
        .ok_or("workbench_api_test_host_window_unavailable")?;
    let webview: Webview = window.as_ref().clone();
    let app = app.clone();
    std::thread::Builder::new()
        .name("workbench-api-test-host".into())
        .spawn(move || {
            if let Err(error) = serve(&app, &webview) {
                eprintln!("workbench_api_test_host_failed: {error}");
            }
            if let Err(error) =
                tauri::async_runtime::block_on(crate::desktop_lifecycle::confirm_app_quit(app))
            {
                eprintln!("workbench_api_test_host_shutdown_failed: {error}");
            }
        })
        .map_err(|error| format!("workbench_api_test_host_start_failed: {error}"))?;
    Ok(())
}

fn write_frame(value: Value) -> Result<(), std::io::Error> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &value)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn serve(app: &AppHandle, webview: &Webview) -> Result<(), Box<dyn std::error::Error>> {
    write_frame(json!({"jsonrpc":"2.0","method":"workbench.ready","params":{
        "protocolVersion":1,"version":env!("CARGO_PKG_VERSION")
    }}))?;
    let mut input = std::io::stdin().lock();
    let mut decoder = FrameDecoder::with_max_frame_bytes(MAX_FRAME_BYTES);
    let mut buffer = [0; 8192];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            if !decoder.is_empty() {
                return Err("workbench_api_test_host_incomplete_frame".into());
            }
            return Ok(());
        }
        for frame in decoder.push(&buffer[..count])? {
            invoke(app, webview, &frame)?;
        }
    }
}

fn invoke(app: &AppHandle, webview: &Webview, frame: &[u8]) -> Result<(), std::io::Error> {
    let Ok(request) = serde_json::from_slice::<Request>(frame) else {
        return write_frame(json!({"jsonrpc":"2.0","id":null,"error":{
            "code":-32600,"message":"workbench_api_test_host_invalid_request"
        }}));
    };
    if request.jsonrpc != "2.0" || request.method.is_empty() || !request.params.is_object() {
        return write_frame(json!({"jsonrpc":"2.0","id":request.id,"error":{
            "code":-32600,"message":"workbench_api_test_host_invalid_request"
        }}));
    }
    let url = match webview.url() {
        Ok(url) => url,
        Err(error) => {
            return write_frame(response_error(request.id, Value::String(error.to_string())));
        }
    };
    webview.clone().on_message(
        tauri::webview::InvokeRequest {
            cmd: request.method,
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url,
            body: InvokeBody::Json(request.params),
            headers: Default::default(),
            invoke_key: app.invoke_key().to_owned(),
        },
        Box::new(move |_, _, response, _, _| {
            let reply = match response {
                InvokeResponse::Ok(body) => match body.deserialize::<Value>() {
                    Ok(value) => json!({"jsonrpc":"2.0","id":request.id,"result":value}),
                    Err(error) => response_error(request.id, Value::String(error.to_string())),
                },
                InvokeResponse::Err(InvokeError(error)) => response_error(request.id, error),
            };
            if let Err(error) = write_frame(reply) {
                eprintln!(
                    "workbench_api_test_host_response_failed: requestId={}: {error}",
                    request.id
                );
            }
        }),
    );
    Ok(())
}

fn response_error(id: u64, cause: Value) -> Value {
    let message = cause
        .as_str()
        .or_else(|| cause.get("message").and_then(Value::as_str))
        .unwrap_or("workbench_api_test_host_command_failed");
    json!({"jsonrpc":"2.0","id":id,"error":{
        "code":-32000,"message":message,"data":cause
    }})
}
