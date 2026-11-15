use super::{Lease, State};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, State as AppState, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use research_docling_runtime::{ENGINE_VERSION, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) fn router(state: Arc<State>) -> Router {
    Router::new()
        .route("/local-service/v1/status", get(status))
        .route("/local-service/v1/lease", get(lease))
        .route("/local-service/v1/shutdown-idle", post(shutdown_idle))
        .route("/local-service/v1/shutdown", post(shutdown))
        .route("/v1/convert/source", post(convert))
        .route(
            "/local-service/v1/conversions/{id}",
            get(crate::conversion_tasks::status).delete(crate::conversion_tasks::cancel),
        )
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(state)
}
pub(crate) fn authorized(state: &State, headers: &HeaderMap) -> bool {
    headers.get("X-Api-Key").and_then(|h| h.to_str().ok()) == Some(state.token.as_str())
}
async fn shutdown(AppState(state): AppState<Arc<State>>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        );
    }
    match state.begin_shutdown() {
        Ok(operation) => (
            StatusCode::ACCEPTED,
            Json(json!({"state":"draining", "operationId":operation})),
        ),
        Err(cause) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"shutdown_failed", "message":cause})),
        ),
    }
}
async fn shutdown_idle(AppState(state): AppState<Arc<State>>, headers: HeaderMap) -> StatusCode {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    let _ = state.snapshot(); // Expire dead clients before testing ownership.
    let Ok(mut life) = state.lifecycle.lock() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    if !life.leases.is_empty() || life.requests != 0 {
        return StatusCode::CONFLICT;
    }
    life.draining = true; // Reject new leases/conversions before signalling exit.
    state.shutdown.notify_one();
    StatusCode::OK
}
async fn status(AppState(state): AppState<Arc<State>>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        );
    }
    match state.snapshot() {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"unavailable"})),
        ),
    }
}
async fn lease(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    if !authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| lease_loop(state, socket))
}
async fn receive(socket: &mut WebSocket, seconds: u64) -> Option<Value> {
    loop {
        match tokio::time::timeout(Duration::from_secs(seconds), socket.recv())
            .await
            .ok()??
            .ok()?
        {
            Message::Text(text) => return serde_json::from_str(&text).ok(),
            Message::Ping(bytes) => {
                socket.send(Message::Pong(bytes)).await.ok()?;
            }
            Message::Close(_) => return None,
            _ => {}
        }
    }
}
async fn lease_loop(state: Arc<State>, mut socket: WebSocket) {
    let Some(hello) = receive(&mut socket, 5).await else {
        return;
    };
    let Some(client) = hello["clientId"]
        .as_str()
        .filter(|c| !c.is_empty() && c.len() < 200)
    else {
        return;
    };
    if hello["type"] != "hello" || hello["protocolVersion"] != PROTOCOL_VERSION {
        return;
    }
    let id = Uuid::new_v4().to_string();
    if let Ok(mut life) = state.lifecycle.lock() {
        if life.draining || state.control.stopping() {
            return;
        }
        life.leases.retain(|_, l| l.client != client);
        life.leases.insert(
            id.clone(),
            Lease {
                client: client.into(),
                seen: Instant::now(),
            },
        );
        life.idle = None;
    } else {
        return;
    }
    let grant = json!({"type":"lease","leaseId":id,"serviceInstanceId":state.instance,
        "heartbeatIntervalMs":10000,"leaseTimeoutMs":30000,
        "activeClientCount":state.snapshot().ok().and_then(|v|v["activeClientCount"].as_u64()),"activeRequestCount":0});
    if socket
        .send(Message::Text(grant.to_string().into()))
        .await
        .is_ok()
    {
        loop {
            let message = tokio::select! {
                _ = state.control.stopped() => break,
                message = receive(&mut socket, 30) => match message { Some(message) => message, None => break },
            };
            if message["type"] != "heartbeat" || message["leaseId"] != id {
                break;
            }
            let alive = if let Ok(mut life) = state.lifecycle.lock() {
                if let Some(lease) = life.leases.get_mut(&id) {
                    lease.seen = Instant::now();
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !alive {
                break;
            }
            let mut ack = state.snapshot().unwrap_or(json!({}));
            ack["type"] = json!("heartbeatAck");
            ack["leaseId"] = json!(id);
            if socket
                .send(Message::Text(ack.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    }
    if let Ok(mut life) = state.lifecycle.lock() {
        life.leases.remove(&id);
    }
    state.publish_status();
}
struct RequestGuard(Arc<State>, Option<String>);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Some(id) = &self.1 {
            self.0.tasks.remove(id);
        }
        if let Ok(mut life) = self.0.lifecycle.lock() {
            life.requests = life.requests.saturating_sub(1);
        }
        self.0.publish_status();
    }
}
async fn convert(
    AppState(state): AppState<Arc<State>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        );
    }
    let lease = headers
        .get("X-Docling-Lease-Id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let request_id = headers
        .get("X-Docling-Request-Id")
        .and_then(|h| h.to_str().ok())
        .filter(|id| Uuid::parse_str(id).is_ok())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let admitted = if let Ok(mut life) = state.lifecycle.lock() {
        if life.draining
            || state.control.stopping()
            || life
                .leases
                .get(lease)
                .is_none_or(|l| l.seen.elapsed() >= Duration::from_secs(30))
            || life.requests >= 2
        {
            Err(StatusCode::CONFLICT)
        } else {
            // Hold lifecycle admission through task registration. Shutdown takes
            // these locks in the same order, so a late task cannot miss cancellation.
            match state.tasks.register(&request_id, lease) {
                Ok(task) => {
                    life.requests += 1;
                    Ok(task)
                }
                Err(error) => Err(error),
            }
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    };
    let task = match admitted {
        Ok(task) => task,
        Err(status) => {
            return (
                status,
                Json(json!({"error":"conversion admission rejected"})),
            );
        }
    };
    let guard = RequestGuard(state.clone(), Some(request_id));
    state.publish_status();
    // Wait asynchronously: a queued cancellation must not wait for another PDF.
    let permit = tokio::select! {
        _ = task.cancelled() => return (StatusCode::GONE, Json(json!({"error":"cancelled"}))),
        permit = state.inference.clone().acquire_owned() => match permit {
            Ok(permit) => permit,
            Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"inference unavailable"}))),
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let _guard = guard;
        let _permit = permit;
        let cancelled = || task.cancel.load(std::sync::atomic::Ordering::Acquire);
        if cancelled() { return Err("cancelled".into()); }
        let sources = body["sources"].as_array().filter(|s| s.len() == 1).ok_or("one PDF source required")?;
        let source = &sources[0];
        if source["kind"] != "file" { return Err("only PDF byte input supported".into()); }
        let bytes = STANDARD.decode(source["base64_string"].as_str().ok_or("missing bytes")?).map_err(|_| "invalid bytes")?;
        if !bytes.starts_with(b"%PDF-") { return Err("PDF input required".into()); }
        let started = Instant::now();
        let mut pipeline = state.pipeline.lock().map_err(|_| "inference unavailable; reconnect service")?;
        if cancelled() { return Err("cancelled".into()); }
        task.stage("parsing");
        pipeline.set_cancel_flag(Some(task.cancel.clone()));
        let progress = task.clone();
        pipeline.set_progress(Some(Arc::new(move |done, total| progress.pages(done, total))));
        let outcome = pipeline.convert_outcome(&bytes, None, "document.pdf");
        pipeline.set_progress(None);
        pipeline.set_cancel_flag(None);
        if cancelled() { return Err("cancelled".into()); }
        let outcome = outcome.map_err(|_| "PDF conversion failed")?;
        task.stage("exporting");
        let status = if outcome.completion.timed_out() { "partial_success" } else { "success" };
        Ok(json!({"status":status,"document":{"json_content":outcome.document.export_to_json_value()},
            "errors":[],"processing_time":started.elapsed().as_secs_f64(),
            "runtime":{"engine":"docling-rs","engineVersion":ENGINE_VERSION,"precision":"fp32",
                "modelVersion":state.model.version,"parserRevision":research_docling_runtime::PARSER_REVISION,
                "identity":state.model.execution_identity()?}}))
    }).await;
    match result {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)),
        Ok(Err(error)) if error == "cancelled" => {
            (StatusCode::GONE, Json(json!({"error":"cancelled"})))
        }
        _ => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"PDF conversion failed"})),
        ),
    }
}
