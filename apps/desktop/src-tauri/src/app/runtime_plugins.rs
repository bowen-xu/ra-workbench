//! Compose the controlled package catalog with Tauri's original local origin.
//!
//! Tauri 2.11.5 installs a user-registered `tauri` protocol before its default
//! asset protocol. Core resources still use its public asset resolver so HTML
//! CSP hashes/nonces and IPC initialization retain their original owners.

use std::{path::PathBuf, sync::Arc};

use tauri::{
    Manager, Runtime,
    http::{Method, Request, Response, StatusCode, header},
    utils::config::HeaderAddition,
};

use crate::runtime_plugins::{
    RuntimePluginCatalog,
    literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    native::{NativePreviewService, NativeProfile},
};

const DIRECTORY_VARIABLE: &str = "RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY";

pub(crate) fn configure<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    let directory = std::env::var_os(DIRECTORY_VARIABLE).map(PathBuf::from);
    let pilot =
        std::env::var("RESEARCH_ASSISTANT_NATIVE_WRITING_PREVIEW").is_ok_and(|value| value == "1");
    let catalog = Arc::new(RuntimePluginCatalog::new(directory).with_native_writing_preview(pilot));
    let builder = builder
        .manage(Arc::new(NativePreviewService::new(Arc::clone(&catalog))))
        .manage(Arc::new(LiteratureTraceService::new(Arc::clone(&catalog))))
        .manage(Arc::clone(&catalog));
    builder.register_asynchronous_uri_scheme_protocol(
        "tauri",
        move |context, request, responder| {
            let app = context.app_handle().clone();
            let catalog = Arc::clone(&catalog);
            // The protocol request owns this bounded read; no filesystem work runs
            // on the WebView event loop or the async executor's worker thread.
            let _response_task = tauri::async_runtime::spawn_blocking(move || {
                responder.respond(response(&app, &catalog, request));
            });
        },
    )
}

/// Resolve bundle resources before AppStore captures its original sources.
/// The catalog and transport owners were already managed before WebView creation.
pub(crate) fn attach_bundled_packages(app: &tauri::AppHandle) -> tauri::Result<()> {
    if std::env::var_os(DIRECTORY_VARIABLE).is_none() {
        let directory = app.path().resource_dir()?.join("plugins");
        if directory.exists() {
            app.state::<Arc<RuntimePluginCatalog>>()
                .attach_bundled_root(directory);
        }
    }
    Ok(())
}

/// Explicit development selection retains precedence over the deployed package.
/// A supplied invalid/unavailable key stays native and fails on the first call;
/// it must not silently select another package or fall back to static decisions.
pub(crate) fn attach_note_trace(
    app: &tauri::AppHandle,
    sources: crate::storage::AppStoreSources,
) -> crate::storage::AppStoreSources {
    let Some(package_key) = binding_package_key(
        app,
        "RESEARCH_ASSISTANT_NATIVE_NOTE_TRACE_PACKAGE_KEY",
        NativeProfile::NoteTraceDecision,
    ) else {
        return sources;
    };
    let binding = NoteTraceBinding {
        service: Arc::clone(app.state::<Arc<LiteratureTraceService>>().inner()),
        package_key,
    };
    crate::storage::AppStoreSources {
        native_note_trace: Some(binding),
        ..sources
    }
}

/// Bind the deployed writing package, with an explicit development override.
/// Core retains the Agent and its original submission transaction.
pub(crate) fn attach_writing_context(
    app: &tauri::AppHandle,
    store: crate::storage::AppStore,
) -> crate::storage::AppStore {
    let Some(package_key) = binding_package_key(
        app,
        "RESEARCH_ASSISTANT_NATIVE_WRITING_PACKAGE_KEY",
        NativeProfile::WritingProjects,
    ) else {
        return store;
    };
    store.with_native_writing_context(crate::runtime_plugins::writing_projects::context::Binding {
        service: Arc::clone(app.state::<Arc<NativePreviewService>>().inner()),
        catalog: Arc::clone(app.state::<Arc<RuntimePluginCatalog>>().inner()),
        package_key,
    })
}

fn binding_package_key(
    app: &tauri::AppHandle,
    variable: &str,
    profile: NativeProfile,
) -> Option<String> {
    if let Some(key) = std::env::var_os(variable) {
        let key = key.into_string().unwrap_or_default();
        // Do not reflect arbitrary environment text in a package diagnostic.
        return Some(
            if key.len() == 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                key
            } else {
                String::new()
            },
        );
    }
    let catalog = app.state::<Arc<RuntimePluginCatalog>>();
    let selected = catalog
        .discover()
        .map_err(|error| {
            let diagnostic = crate::runtime_plugins::native::NativeHostError::for_profile(
                profile,
                error.code,
                error.message,
            );
            match error.package_key {
                Some(key) => diagnostic.package(&key),
                None => diagnostic,
            }
        })
        .and_then(|listing| select_native_package(&listing.packages, profile));
    match selected {
        Ok(key) => key,
        Err(error) => {
            // The existing loader reports discovery/compatibility failures to
            // the user. Do not enable a Core domain source on failed discovery.
            eprintln!("原生插件绑定不可用 [{}]：{}", error.code, error.message);
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartupTrashCleanup {
    NoProvider,
    Completed { package_key: String },
}

/// Core-only setup calls this at the original synchronous purge position.
/// Its caller retains the original failure log and subsequent GC scheduling.

pub(crate) fn purge_expired_trash_on_startup(
    app: &tauri::AppHandle,
    store: &crate::storage::AppStore,
) -> crate::runtime_plugins::native::NativeResult<StartupTrashCleanup> {
    let catalog = Arc::clone(app.state::<Arc<RuntimePluginCatalog>>().inner());
    let service = Arc::clone(app.state::<Arc<LiteratureTraceService>>().inner());
    let database = store.shared_agent_connection();
    tauri::async_runtime::block_on(run_startup_purge(catalog, move |package_key| async move {
        service
            .purge_expired_trash(database, package_key)
            .await
            .map(|warnings| {
                for warning in warnings {
                    eprintln!("回收站到期清理警告：{warning}");
                }
            })
    }))
}

fn startup_catalog_error(
    error: crate::runtime_plugins::RuntimePluginError,
) -> Box<crate::runtime_plugins::native::NativeHostError> {
    use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
    let diagnostic =
        NativeHostError::for_profile(NativeProfile::NoteTraceDecision, error.code, error.message);
    match error.package_key {
        Some(key) => diagnostic.package(&key),
        None => diagnostic,
    }
}

fn select_startup_package(
    packages: &[crate::runtime_plugins::RuntimePluginPackage],
) -> crate::runtime_plugins::native::NativeResult<Option<String>> {
    select_native_package(packages, NativeProfile::NoteTraceDecision)
}

fn select_native_package(
    packages: &[crate::runtime_plugins::RuntimePluginPackage],
    profile: NativeProfile,
) -> crate::runtime_plugins::native::NativeResult<Option<String>> {
    use crate::runtime_plugins::native::NativeHostError;
    let providers = packages
        .iter()
        .filter(|package| {
            package
                .manifest
                .get("id")
                .and_then(serde_json::Value::as_str)
                == Some(profile.plugin_id())
        })
        .collect::<Vec<_>>();
    match providers.as_slice() {
        [] => Ok(None),
        [package] => Ok(Some(package.package_key.clone())),
        _ => Err(NativeHostError::for_profile(
            profile,
            "plugin_host_package_ambiguous",
            "受控目录存在同一插件的多个包，未选择提供者。",
        )),
    }
}

async fn run_startup_purge<F, P>(
    catalog: Arc<RuntimePluginCatalog>,
    purge: F,
) -> crate::runtime_plugins::native::NativeResult<StartupTrashCleanup>
where
    F: FnOnce(String) -> P,
    P: std::future::Future<Output = crate::runtime_plugins::native::NativeResult<()>>,
{
    use crate::runtime_plugins::native::{
        NativeHostError, NativeProfile, NativeResult, accept_trash_cleanup,
    };
    let selected = tauri::async_runtime::spawn_blocking(move || -> NativeResult<Option<String>> {
        let listing = catalog.discover().map_err(startup_catalog_error)?;
        let Some(package_key) = select_startup_package(&listing.packages)? else {
            return Ok(None);
        };
        accept_trash_cleanup(&catalog, &package_key)?;
        Ok(Some(package_key))
    })
    .await
    .map_err(|_| {
        NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_accept_failed",
            "启动清理的插件包核对任务未完成。",
        )
    })??;
    let Some(package_key) = selected else {
        return Ok(StartupTrashCleanup::NoProvider);
    };
    purge(package_key.clone()).await?;
    Ok(StartupTrashCleanup::Completed { package_key })
}

fn response<R: Runtime>(
    app: &tauri::AppHandle<R>,
    catalog: &RuntimePluginCatalog,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let Some(origin) = local_origin(request.uri()) else {
        return error_response(StatusCode::FORBIDDEN, "runtime_plugin_origin_invalid");
    };
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "runtime_plugin_method_invalid",
        );
    }
    let path = request.uri().path();
    let (bytes, mime_type, csp_header) = if path == "/plugins" || path.starts_with("/plugins/") {
        if request.uri().query().is_some() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "runtime_plugin_resource_path_invalid",
            );
        }
        let Some((package_key, resource)) = package_resource(path) else {
            return error_response(
                StatusCode::BAD_REQUEST,
                "runtime_plugin_resource_path_invalid",
            );
        };
        match catalog.resource(package_key, resource) {
            Ok(resource) => (resource.bytes, resource.mime_type, None),
            Err(error) => {
                // Keep the catalog's stable diagnostic; missing package assets
                // are never sent through the SPA's index.html fallback.
                let status = StatusCode::from_u16(error.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let mut response = error_response(status, error.code);
                if let Ok(bytes) = serde_json::to_vec(&error) {
                    *response.body_mut() = bytes;
                    response.headers_mut().insert(
                        header::CONTENT_TYPE,
                        header::HeaderValue::from_static("application/json"),
                    );
                }
                return response;
            }
        }
    } else {
        match app
            .asset_resolver()
            .get_for_scheme(path.to_owned(), request.uri().scheme_str() == Some("https"))
        {
            Some(asset) => (asset.bytes, asset.mime_type, asset.csp_header),
            None => return error_response(StatusCode::NOT_FOUND, "runtime_core_asset_missing"),
        }
    };
    let mut builder = Response::builder()
        .add_configured_headers(app.config().app.security.headers.as_ref())
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(csp) = csp_header {
        builder = builder.header(header::CONTENT_SECURITY_POLICY, csp);
    }
    let body = if request.method() == Method::HEAD {
        Vec::new()
    } else {
        bytes
    };
    builder.body(body).unwrap_or_else(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "runtime_plugin_response_invalid",
        )
    })
}

fn local_origin(uri: &tauri::http::Uri) -> Option<&'static str> {
    match (
        uri.scheme_str(),
        uri.authority().map(|value| value.as_str()),
    ) {
        (Some("tauri"), Some("localhost")) => Some("tauri://localhost"),
        (Some("http"), Some("tauri.localhost")) => Some("http://tauri.localhost"),
        (Some("https"), Some("tauri.localhost")) => Some("https://tauri.localhost"),
        _ => None,
    }
}

fn package_resource(path: &str) -> Option<(&str, &str)> {
    let (key, resource) = path.strip_prefix("/plugins/")?.split_once('/')?;
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || resource.is_empty()
    {
        return None;
    }
    Some((key, resource))
}

fn error_response(status: StatusCode, code: &str) -> Response<Vec<u8>> {
    let mut response = Response::new(code.as_bytes().to_vec());
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
#[path = "runtime_plugins/startup_tests.rs"]
mod startup_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_is_restricted_to_tauris_existing_local_origins() {
        for url in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/sdk/v1/react.js",
            "https://tauri.localhost/plugins/test",
        ] {
            assert!(local_origin(&url.parse().unwrap()).is_some());
        }
        for url in [
            "https://example.com/index.html",
            "tauri://other/index.html",
            "http://localhost/index.html",
            "tauri://localhost:80/index.html",
        ] {
            assert!(local_origin(&url.parse().unwrap()).is_none());
        }
    }

    #[test]
    fn only_content_keyed_package_routes_reach_the_catalog() {
        let key = "a".repeat(64);
        let path = format!("/plugins/{key}/entry.js");
        assert_eq!(package_resource(&path), Some((key.as_str(), "entry.js")));
        for path in [
            "/plugins",
            "/plugins/",
            "/plugins/entry.js",
            "/plugins/no/entry.js",
            "/plugins//entry.js",
            "/assets/entry.js",
        ] {
            assert!(package_resource(path).is_none());
        }
        assert!(package_resource(&format!("/plugins/{key}/")).is_none());
        assert!(package_resource(&format!("/plugins/{}/entry.js", "A".repeat(64))).is_none());
    }
}
