//! Native local daemon: the application owns only a lease; the daemon owns inference.
mod control;
mod conversion_tasks;
mod service;
#[cfg(test)]
mod tests;
#[cfg(target_os = "macos")]
mod tray;
use research_docling_runtime::{ModelManifest, PROTOCOL_VERSION};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use uuid::Uuid;

struct Lease {
    client: String,
    seen: Instant,
}
struct Lifecycle {
    leases: HashMap<String, Lease>,
    requests: usize,
    idle: Option<Instant>,
    draining: bool,
}
struct State {
    token: String,
    instance: String,
    model: ModelManifest,
    lifecycle: Mutex<Lifecycle>,
    pipeline: Mutex<docling_pdf::Pipeline>,
    tasks: conversion_tasks::Tasks,
    inference: Arc<tokio::sync::Semaphore>,
    shutdown: Notify,
    control: control::Control,
}
impl State {
    fn begin_shutdown(&self) -> Result<String, &'static str> {
        let mut life = self.lifecycle.lock().map_err(|_| "state unavailable")?;
        let operation = self.control.request_stop();
        life.draining = true;
        self.tasks.cancel_all()?;
        Ok(operation)
    }

    fn publish_status(&self) {
        if let Ok(life) = self.lifecycle.lock() {
            let mut status = self.control.status();
            status.clients = life.leases.len();
            status.requests = life.requests;
            status.stopping |= life.draining;
            self.control.update(status);
        }
    }

    fn snapshot(&self) -> Result<serde_json::Value, &'static str> {
        let mut life = self.lifecycle.lock().map_err(|_| "state unavailable")?;
        life.leases
            .retain(|_, lease| lease.seen.elapsed() < Duration::from_secs(30));
        Ok(
            json!({"schemaVersion":1,"protocolVersion":PROTOCOL_VERSION,"serviceInstanceId":self.instance,
            "state":if life.draining || self.control.stopping() {"draining"} else {"ready"},"activeClientCount":life.leases.len(),
            "activeRequestCount":life.requests,"serviceRevision":research_docling_runtime::SERVICE_REVISION,"engineVersion":research_docling_runtime::ENGINE_VERSION,
            "modelIdentity":self.model.execution_identity().map_err(|_| "invalid model")?,"precision":"fp32",
            "shutdownOperationId":self.control.operation_id()}),
        )
    }
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("missing parent")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}
fn bind(default: u16, start: u16, end: u16) -> Result<TcpListener, std::io::Error> {
    for port in std::iter::once(default)
        .chain(start..=end)
        .chain(std::iter::once(0))
    {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            listener.set_nonblocking(true)?;
            return Ok(listener);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "no loopback port",
    ))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Version probes and model validation are CLI operations, never menu owners.
    #[cfg(target_os = "macos")]
    {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        if !arguments
            .iter()
            .any(|arg| arg == "--version" || arg == "--validate-models")
        {
            return tray::run();
        }
    }
    run_runtime(control::Control::default())
}

fn run_runtime(control: control::Control) -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_service(control))
}

async fn run_service(control: control::Control) -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments == ["--version"] {
        println!(
            "research-docling-daemon {} fp32 protocol={PROTOCOL_VERSION} parser={} service={}",
            research_docling_runtime::ENGINE_VERSION,
            research_docling_runtime::PARSER_REVISION,
            research_docling_runtime::SERVICE_REVISION
        );
        return Ok(());
    }
    let args: HashMap<_, _> = arguments
        .chunks_exact(2)
        .map(|pair| (pair[0].as_str(), pair[1].as_str()))
        .collect();
    let get = |key| args.get(key).copied().ok_or("missing daemon argument");
    let models = PathBuf::from(get("--model-directory")?);
    // Held through inference and shutdown, including model verification/loading.
    // Storage cleanup must acquire this same lock exclusively.
    let model_usage = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(
            models
                .parent()
                .ok_or("missing model parent")?
                .join("usage.lock"),
        )?;
    fs2::FileExt::lock_shared(&model_usage)?;
    let manifest: ModelManifest = serde_json::from_slice(&fs::read(models.join("manifest.json"))?)?;
    manifest.verify(&models)?;
    // Upstream permits degraded OCR/table operation. This product requires every
    // declared model to load; a broken bundle must never look like a full parse.
    for file in &manifest.files {
        if control.stopping() {
            return Ok(());
        }
        if file.path.ends_with(".onnx") {
            let _session =
                docling_pdf::ep::session_builder()?.commit_from_file(models.join(&file.path))?;
        }
    }
    if std::env::var("DOCLING_RS_FP32").as_deref() != Ok("1") {
        return Err("FP32 mode is required".into());
    }
    // The launcher pins the multilingual PP-OCRv6 model and dictionary; the
    // language selector only chooses legacy fallback assets, which we do not ship.
    let mut pipeline = docling_pdf::Pipeline::new()?
        .ocr_engine(Some(docling_pdf::OcrEngine::PpOcr))
        .ocr_scale(Some(3.0));
    pipeline.warm_up()?;
    if args.contains_key("--validate-models") || control.stopping() {
        return Ok(());
    }
    let registry = PathBuf::from(get("--registry-path")?);
    let token_path = PathBuf::from(get("--token-path")?);
    let state = Arc::new(State {
        token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        instance: Uuid::new_v4().to_string(),
        model: manifest,
        pipeline: Mutex::new(pipeline),
        tasks: conversion_tasks::Tasks::default(),
        inference: Arc::new(tokio::sync::Semaphore::new(1)),
        lifecycle: Mutex::new(Lifecycle {
            leases: HashMap::new(),
            requests: 0,
            idle: None,
            draining: false,
        }),
        shutdown: Notify::new(),
        control: control.clone(),
    });
    let listener = bind(
        get("--default-port")?.parse()?,
        get("--fallback-start-port")?.parse()?,
        get("--fallback-end-port")?.parse()?,
    )?;
    let port = listener.local_addr()?.port();
    control.update(control::Status {
        ready: true,
        port: Some(port),
        ..Default::default()
    });
    private_write(&token_path, state.token.as_bytes())?;
    let record = json!({"schemaVersion":1,"protocolVersion":PROTOCOL_VERSION,"serviceInstanceId":state.instance,
        "state":"ready","httpBaseUrl":format!("http://127.0.0.1:{port}"),
        "websocketUrl":format!("ws://127.0.0.1:{port}/local-service/v1/lease"),"pid":std::process::id(),
        "startedAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs().to_string(),
        "wrapperVersion":"2","doclingServeVersion":research_docling_runtime::ENGINE_VERSION,
        "heartbeatIntervalMs":10000,"leaseTimeoutMs":30000,"idleShutdownMs":get("--idle-shutdown-ms")?.parse::<u64>()?,
        "modelIdentity":state.model.execution_identity()?,"precision":"fp32"});
    private_write(&registry, &serde_json::to_vec(&record)?)?;
    let idle = Duration::from_millis(get("--idle-shutdown-ms")?.parse()?);
    let supervisor_state = state.clone();
    let supervisor = tokio::spawn(async move {
        loop {
            if supervisor_state.control.stopping() {
                let _ = supervisor_state.begin_shutdown();
                tokio::time::sleep(Duration::from_millis(100)).await;
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                    _ = supervisor_state.control.stopped() => {},
                    _ = tokio::signal::ctrl_c() => { supervisor_state.control.request_stop(); },
                }
            }
            let _ = supervisor_state.snapshot();
            supervisor_state.publish_status();
            let should_exit = if let Ok(mut life) = supervisor_state.lifecycle.lock() {
                if supervisor_state.control.stopping() && life.requests == 0 {
                    life.draining = true;
                    true
                } else if life.leases.is_empty() && life.requests == 0 {
                    let since = *life.idle.get_or_insert_with(Instant::now);
                    if since.elapsed() >= idle {
                        life.draining = true;
                        true
                    } else {
                        false
                    }
                } else {
                    life.idle = None;
                    false
                }
            } else {
                true
            };
            if should_exit {
                supervisor_state.shutdown.notify_one();
                break;
            }
        }
    });
    let stop = state.clone();
    let result = axum::serve(
        tokio::net::TcpListener::from_std(listener)?,
        service::router(state.clone()),
    )
    .with_graceful_shutdown(async move {
        stop.shutdown.notified().await;
    })
    .await;
    supervisor.abort();
    // An older process must never remove a newer process's discovery files.
    if fs::read(&registry)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|record| record["serviceInstanceId"] == state.instance)
    {
        let _ = fs::remove_file(registry);
        let _ = fs::remove_file(token_path);
    }
    result?;
    Ok(())
}
