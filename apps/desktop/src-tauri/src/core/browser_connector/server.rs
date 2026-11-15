use std::{
    convert::Infallible,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tauri::Emitter;
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    task::JoinSet,
    time::timeout,
};

use super::CaptureRequest;
use super::{
    Shared,
    protocol::{self, API_VERSION, INSTANCE_HEADER, MAX_JSON_BYTES, MAX_PDF_BYTES, TOKEN_HEADER},
};

type HttpResponse = Response<Full<Bytes>>;

#[cfg(test)]
#[path = "server/connection_permit_tests.rs"]
mod connection_permit_tests;

pub(super) async fn run(
    listener: std::net::TcpListener,
    shared: Arc<Shared>,
    mut stopped: oneshot::Receiver<()>,
) {
    let Ok(listener) = TcpListener::from_std(listener) else {
        shared.active.store(false, Ordering::Release);
        return;
    };
    // Limits the total memory held by bounded PDF requests and rejects excess
    // connections instead of accumulating an unbounded task or socket queue.
    let slots = Arc::new(Semaphore::new(4));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { break; };
                if !peer.ip().is_loopback() { continue; }
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue; };
                let slot = Arc::new(slot);
                let shared = shared.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| serve(request, shared.clone(), slot.clone()));
                    let mut builder = http1::Builder::new();
                    builder.keep_alive(false).max_headers(32).max_buf_size(32768)
                        .timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(5));
                    let connection = builder.serve_connection(TokioIo::new(stream), service);
                    // Clients may disappear after a durable write. Storage owns
                    // its receipt, so cancelling this waiter never undoes it.
                    let _ = timeout(Duration::from_secs(120), connection).await;
                });
            }
        }
    }
    shared.active.store(false, Ordering::Release);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

async fn serve(
    request: Request<Incoming>,
    shared: Arc<Shared>,
    slot: Arc<OwnedSemaphorePermit>,
) -> Result<HttpResponse, Infallible> {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut response = match handle(request, shared.clone(), slot).await {
        Ok(response) => response,
        Err(status) => error(
            status,
            "request_rejected",
            status.canonical_reason().unwrap_or("Request rejected"),
        ),
    };
    if let Ok(instance) = shared.instance_id.parse() {
        response.headers_mut().insert(INSTANCE_HEADER, instance);
    }
    Ok(with_origin(response, origin.as_deref()))
}

async fn handle(
    request: Request<Incoming>,
    shared: Arc<Shared>,
    slot: Arc<OwnedSemaphorePermit>,
) -> Result<HttpResponse, StatusCode> {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if request.method() == Method::OPTIONS {
        return preflight(&request, &shared.host);
    }
    protocol::validate_headers(request.headers(), &shared.host)?;
    if !shared.active.load(Ordering::Acquire) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let route = request
        .uri()
        .path()
        .strip_prefix("/connector/")
        .ok_or(StatusCode::NOT_FOUND)?
        .to_owned();
    // Discovery is read-only and accepts no capture payload or native operation.
    // Every other endpoint belongs to this exact running receiver instance.
    if route == "ping" && matches!(*request.method(), Method::GET | Method::POST) {
        return Ok(with_origin(
            json_response(StatusCode::OK, protocol::ping(&shared)),
            origin.as_deref(),
        ));
    }
    if request.headers().get_all(INSTANCE_HEADER).iter().count() != 1
        || request
            .headers()
            .get(INSTANCE_HEADER)
            .and_then(|value| value.to_str().ok())
            != Some(shared.instance_id.as_str())
    {
        return Err(StatusCode::CONFLICT);
    }
    if request.headers().get_all(TOKEN_HEADER).iter().count() != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let token = request
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if token.len() != 64 || !bool::from(shared.connection_token.as_bytes().ct_eq(token.as_bytes()))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let binary = matches!(
        route.as_str(),
        "saveAttachment" | "saveStandaloneAttachment"
    );
    let session = if binary {
        Some(protocol::session_id(request.uri().query())?)
    } else {
        None
    };
    let metadata = if binary {
        Some(protocol::attachment_metadata(request.headers())?)
    } else {
        None
    };
    if request.method() != Method::POST
        && !(request.method() == Method::GET
            && matches!(route.as_str(), "ping" | "getClientHostnames"))
    {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    let maximum = if binary {
        MAX_PDF_BYTES
    } else {
        MAX_JSON_BYTES
    };
    if request
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > maximum)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let body = timeout(
        Duration::from_secs(60),
        Limited::new(request.into_body(), maximum).collect(),
    )
    .await
    .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
    .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?
    .to_bytes();
    let payload = if binary || body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let shared_for_import = shared.clone();
    let result = spawn_import(slot, move || {
        shared_for_import
            .identity
            .with_scope(|scope| {
                if !shared_for_import.active.load(Ordering::Acquire) {
                    return error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "stopping",
                        "Application is stopping",
                    );
                }
                let result = (shared_for_import.capture)(
                    &shared_for_import.store,
                    scope,
                    CaptureRequest {
                        route: route.clone(),
                        payload,
                        session_id: session,
                        metadata,
                        bytes: body,
                    },
                );
                match result {
                    Ok(value) => {
                        let kind = match route.as_str() {
                            "saveItems" | "saveSnapshot" => Some("item"),
                            "saveAttachment" | "saveStandaloneAttachment" => Some("attachment"),
                            _ => None,
                        };
                        if let Some(kind) = kind
                            && let Some(app) = &shared_for_import.app
                        {
                            let _ = app
                                .emit("research-assistant:browser-capture", json!({"kind":kind}));
                        }
                        let status = if matches!(
                            route.as_str(),
                            "saveItems"
                                | "saveSnapshot"
                                | "saveAttachment"
                                | "saveStandaloneAttachment"
                        ) {
                            StatusCode::CREATED
                        } else {
                            StatusCode::OK
                        };
                        json_response(status, value)
                    }
                    Err(error_value) => json_response(
                        StatusCode::from_u16(error_value.status())
                            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                        error_value.response(),
                    ),
                }
            })
            .unwrap_or_else(|| {
                error(
                    StatusCode::UNAUTHORIZED,
                    "unavailable",
                    "Receiver identity is unavailable",
                )
            })
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(with_origin(result, origin.as_deref()))
}

/// A disconnected or timed-out HTTP waiter cannot cancel a blocking import.
/// Keep its admission slot until both its bytes and storage operation are gone.
fn spawn_import<T: Send + 'static>(
    slot: Arc<OwnedSemaphorePermit>,
    operation: impl FnOnce() -> T + Send + 'static,
) -> tauri::async_runtime::JoinHandle<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let _slot = slot;
        operation()
    })
}

fn preflight(request: &Request<Incoming>, host: &str) -> Result<HttpResponse, StatusCode> {
    if request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        != Some(host)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .filter(|value| protocol::extension_origin(value))
        .ok_or(StatusCode::FORBIDDEN)?;
    let method = request
        .headers()
        .get("access-control-request-method")
        .and_then(|value| value.to_str().ok());
    if !matches!(method, Some("GET" | "POST")) {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    let mut response = with_origin(
        json_response(StatusCode::NO_CONTENT, Value::Null),
        Some(origin),
    );
    let headers = response.headers_mut();
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS"
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    headers.insert("access-control-allow-headers", "Content-Type, X-Metadata, X-Zotero-Version, X-Zotero-Connector-API-Version, X-Research-Assistant-Token, X-Research-Assistant-Instance-ID, X-Research-Assistant-Connector-API-Version".parse().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    headers.insert(
        "x-research-assistant-application",
        hyper::header::HeaderValue::from_static("research-assistant"),
    );
    headers.insert(
        "x-research-assistant-connector-api-version",
        hyper::header::HeaderValue::from_static(API_VERSION),
    );
    headers.insert(
        "x-zotero-connector-api-version",
        hyper::header::HeaderValue::from_static("3"),
    );
    headers.insert(
        "access-control-allow-private-network",
        "true"
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}

fn json_response(status: StatusCode, value: Value) -> HttpResponse {
    let mut response = Response::new(Full::new(Bytes::from(
        if status == StatusCode::NO_CONTENT {
            String::new()
        } else {
            value.to_string()
        },
    )));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        "content-type",
        hyper::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-research-assistant-application",
        hyper::header::HeaderValue::from_static("research-assistant"),
    );
    headers.insert(
        "x-research-assistant-connector-api-version",
        hyper::header::HeaderValue::from_static(API_VERSION),
    );
    headers.insert(
        "x-zotero-connector-api-version",
        hyper::header::HeaderValue::from_static("3"),
    );
    headers.insert(
        "cache-control",
        hyper::header::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        "connection",
        hyper::header::HeaderValue::from_static("close"),
    );
    headers.insert(
        "x-research-assistant-version",
        hyper::header::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
    // Compatibility capability marker required by the retained Connector v3
    // response decoder. The application identity is separately explicit above.
    headers.insert(
        "x-zotero-version",
        hyper::header::HeaderValue::from_static("9.0.0"),
    );
    response
}

fn error(status: StatusCode, code: &str, message: &str) -> HttpResponse {
    json_response(status, json!({"error":code,"message":message}))
}

fn with_origin(mut response: HttpResponse, origin: Option<&str>) -> HttpResponse {
    if let Some(origin) = origin.filter(|origin| protocol::extension_origin(origin))
        && let Ok(value) = origin.parse()
    {
        response
            .headers_mut()
            .insert("access-control-allow-origin", value);
        response
            .headers_mut()
            .insert("vary", hyper::header::HeaderValue::from_static("Origin"));
    }
    response
}
